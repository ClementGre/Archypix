use crate::clients::federation::FederationClient;
use crate::domain::hierarchy::TagPredicate;
use crate::domain::picture::{Picture, PictureVersion, UploadSession};
use crate::domain::tag::TagPath;
use crate::infra::config::Config;
use crate::infra::error::{AppError, map_sqlx_error};
use crate::infra::redis::{Cache, RedisKey, cache_get_json, cache_set_json_ex};
use crate::infra::s3::{self, Storage};
use crate::repository::picture::{
    PictureListFilter, PictureRepository, PictureSortField, SortOrder,
};
use crate::repository::picture_version::PictureVersionRepository;
use crate::repository::tag::TagRepository;
use crate::services::users::find_local_user_id;
use chrono::{DateTime, NaiveDateTime, Utc};
use futures_util::future::try_join_all;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use std::str::FromStr;
use tracing::trace;
use uuid::Uuid;

/// Selectable picture variant for presigning. Used both in list thumbnails and the per-picture URL endpoint.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PictureVariant {
    Original,
    Small,
    Medium,
    Large,
}

impl PictureVariant {
    pub fn bucket<'a>(&self, config: &'a Config) -> &'a str {
        match self {
            PictureVariant::Original => &config.s3_bucket_pictures,
            PictureVariant::Small => &config.s3_bucket_small,
            PictureVariant::Medium => &config.s3_bucket_medium,
            PictureVariant::Large => &config.s3_bucket_large,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }
}

impl FromStr for PictureVariant {
    type Err = AppError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "original" => Ok(Self::Original),
            "small" => Ok(Self::Small),
            "medium" => Ok(Self::Medium),
            "large" => Ok(Self::Large),
            other => Err(AppError::BadRequest(format!("Unknown variant: {other}"))),
        }
    }
}

// Keep the old name as an alias so list_pictures still compiles.
pub type ThumbnailSize = PictureVariant;

#[derive(Debug, Clone, Deserialize)]
pub struct UploadMetadata {
    pub mime_type: Option<String>,
    pub file_size: Option<i64>,
    pub file_hash: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub exif_data: Option<serde_json::Value>,
    pub captured_at: Option<NaiveDateTime>,
    pub initial_tags: Option<Vec<String>>,
    #[serde(default)]
    pub defer_pipeline: bool,
}

fn default_page() -> u32 {
    1
}
fn default_page_size() -> u32 {
    50
}

#[derive(Debug, Clone, Deserialize)]
pub struct PictureListParams {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
    #[serde(default)]
    pub sort: PictureSortField,
    #[serde(default)]
    pub order: SortOrder,
    pub tag: Option<String>,
    /// Flat tag-set filter (§6.3). Comma-separated ltree paths; combined per `match`.
    pub include_tags: Option<String>,
    pub exclude_tags: Option<String>,
    /// `all` (AND) | `any` (OR) over `include_tags`. Default `all`.
    #[serde(rename = "match")]
    pub match_mode: Option<String>,
    /// `true` ⇒ pictures with no stored tag of any source (mutually exclusive with include/exclude).
    #[serde(default)]
    pub untagged: bool,
    #[serde(default)]
    pub owned_only: bool,
    #[serde(default)]
    pub shared_with_me: bool,
    #[serde(default)]
    pub include_deleted: bool,
    pub captured_after: Option<DateTime<Utc>>,
    pub captured_before: Option<DateTime<Utc>>,
    pub thumbnail: Option<ThumbnailSize>,
}

#[derive(Debug, Serialize)]
pub struct PictureListItem {
    pub id: Uuid,
    pub filename: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub captured_at: Option<NaiveDateTime>,
    pub ingested_at: NaiveDateTime,
    /// BlurHash string for progressive loading. `None` until the thumbnail worker runs.
    pub blurhash: Option<String>,
    /// EXIF orientation value (1–8). Thumbnails are stored in raw pixel orientation, so the
    /// client rotates them to display correctly.
    pub orientation: Option<i16>,
    pub thumbnail_url: Option<String>,
    /// `true` when this row is a picture owned by the local user; `false` for a received
    /// (shared) picture. Lets the client label/filter shared pictures.
    pub owned: bool,
    /// Original owner identity for received pictures (`@owner_username:owner_instance`); `None`
    /// for owned pictures.
    pub owner_username: Option<String>,
    pub owner_instance: Option<String>,
    /// Convergence of the file's embedded EXIF vs the DB row.
    pub exif_sync_status: crate::domain::picture::ExifSyncStatus,
    /// The recipient's own local soft-delete timestamp (trash view); `None` when not trashed.
    pub deleted_at: Option<NaiveDateTime>,
    /// Owner-deletion lifecycle for received pictures (09 §5.3): the owner's soft-delete timestamp
    /// and announced purge deadline. Drive the red "owner will delete this on X" badge.
    pub owner_deleted_at: Option<NaiveDateTime>,
    pub owner_purge_at: Option<NaiveDateTime>,
}

#[derive(Debug, Serialize)]
pub struct PictureListResult {
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
    pub items: Vec<PictureListItem>,
}

#[derive(Debug, Serialize)]
pub struct PictureDetails {
    pub picture: Picture,
    pub versions: Vec<PictureVersion>,
}

/// Presign upload slots for a batch of files in one call. Returns one (picture_id, presigned_url)
/// pair per filename in the same order. Each entry is independent — a failure on one does not
/// affect the others, but the whole call fails if any presign errors.
#[tracing::instrument(skip(cache, storage, config), fields(user_id = %user_id, count = filenames.len()))]
pub async fn begin_upload_batch(
    cache: &dyn Cache,
    storage: &dyn Storage,
    config: &Config,
    user_id: Uuid,
    filenames: &[String],
) -> Result<Vec<(Uuid, String)>, AppError> {
    if filenames.is_empty() {
        return Err(AppError::BadRequest("No filenames provided".to_string()));
    }
    if filenames.len() > 100 {
        return Err(AppError::BadRequest(
            "Cannot request more than 100 upload slots at once".to_string(),
        ));
    }
    let futures = filenames
        .iter()
        .map(|name| begin_upload(cache, storage, config, user_id, name));
    try_join_all(futures).await
}

#[tracing::instrument(skip(cache, storage, config), fields(user_id = %user_id))]
pub async fn begin_upload(
    cache: &dyn Cache,
    storage: &dyn Storage,
    config: &Config,
    user_id: Uuid,
    filename: &str,
) -> Result<(Uuid, String), AppError> {
    if filename.trim().is_empty() {
        return Err(AppError::BadRequest("Filename cannot be empty".to_string()));
    }

    let picture_id = Uuid::new_v4();
    let s3_key_staging = format!("staging/{}/{}", user_id, picture_id);

    let presigned_url = storage
        .presign_put(&config.s3_bucket_staging, &s3_key_staging)
        .await?;

    let session = UploadSession {
        user_id,
        picture_id,
        s3_key_staging,
        filename: filename.to_string(),
    };
    cache_set_json_ex(
        cache,
        RedisKey::UploadSession(picture_id),
        &session,
        config.s3_presign_ttl_secs + 60,
    )
    .await?;

    Ok((picture_id, presigned_url))
}

#[tracing::instrument(skip(db, cache, storage, config, meta), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn complete_upload(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    config: &Config,
    user_id: Uuid,
    picture_id: Uuid,
    meta: UploadMetadata,
) -> Result<Picture, AppError> {
    let session: UploadSession = cache_get_json(cache, RedisKey::UploadSession(picture_id))
        .await?
        .ok_or_else(|| AppError::BadRequest("Upload session not found or expired".to_string()))?;

    if session.user_id != user_id {
        return Err(AppError::Unauthorized(
            "Upload session belongs to another user".to_string(),
        ));
    }

    // Validate any initial tags up front (before touching S3) — reject malformed paths and the
    // reserved `SharedToMe` prefix, matching the `PATCH /tags` contract (07_security_audit.md §2.5).
    let initial_tags: Vec<String> = match meta.initial_tags.as_ref() {
        Some(tags) => tags
            .iter()
            .map(|t| {
                TagPath::parse(t, false)
                    .map(|p| p.as_ltree().to_string())
                    .map_err(AppError::BadRequest)
            })
            .collect::<Result<_, _>>()?,
        None => Vec::new(),
    };

    // S3: copy staging → pictures, then delete staging (S3 ops can't be in a DB tx)
    let pictures_key = s3::picture_key(user_id, picture_id);
    storage
        .copy_object(
            &config.s3_bucket_staging,
            &session.s3_key_staging,
            &config.s3_bucket_pictures,
            &pictures_key,
        )
        .await?;
    storage
        .delete_object(&config.s3_bucket_staging, &session.s3_key_staging)
        .await?;

    // Authoritative size: read it back from S3 rather than trusting the client value
    let file_size = match storage
        .object_size(&config.s3_bucket_pictures, &pictures_key)
        .await
    {
        Ok(size) => Some(size),
        Err(e) => {
            tracing::warn!(picture_id = %picture_id, error = ?e, "complete_upload: S3 HEAD failed; falling back to client-reported size");
            meta.file_size
        }
    };

    // Single DB transaction: create picture row, thumbnail job.
    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;

    let picture = PictureRepository::create(
        &mut *tx,
        picture_id,
        user_id,
        Some(session.filename.as_str()),
        meta.mime_type.as_deref(),
        file_size,
        meta.width,
        meta.height,
        meta.exif_data.clone(),
        meta.captured_at,
    )
    .await?;

    // Persist any client-computed SHA-256 as the provisional hash
    if let Some(hash) = meta.file_hash.as_deref() {
        PictureRepository::set_file_hash(&mut *tx, picture_id, hash, file_size).await?;
    }

    // Enqueue initial thumbnail generation + EXIF extraction inside the same transaction
    crate::services::jobs::enqueue_thumbnail_job(&mut *tx, user_id, picture_id, true).await?;

    // Assign any caller-supplied initial manual tags
    if !initial_tags.is_empty() {
        TagRepository::batch_assign(&mut *tx, user_id, &[picture_id], &initial_tags).await?;
    }

    tx.commit().await.map_err(map_sqlx_error)?;

    // Cache cleanup is after commit. A failure here is non-fatal
    if let Err(e) = cache.del(RedisKey::UploadSession(picture_id)).await {
        tracing::warn!(picture_id = %picture_id, error = ?e, "failed to delete upload session from cache");
    }

    Ok(picture)
}

/// Snapshot a picture's current original bytes as a new `picture_version` before a WebDAV
/// overwrite, per the user's `versioning_mode` (06_webdav.md §7.3):
///
/// - `None` → never snapshot (overwrite in place);
/// - `OriginalCopy` → snapshot only the first time (preserve the pristine original, once);
/// - `FullVersioning` → snapshot before every overwrite.
///
/// Reuses the version-snapshot machinery of the worker edit path: S3 copy first (no DB record
/// exists yet, so it is safe outside a transaction), then the version row in a transaction so
/// `version_number` is computed and stored atomically. Returns whether a snapshot was taken.
#[tracing::instrument(skip(db, storage, config, picture), fields(picture_id = %picture.id))]
pub async fn snapshot_version_on_overwrite(
    db: &PgPool,
    storage: &dyn Storage,
    config: &Config,
    versioning_mode: crate::domain::user_settings::VersioningMode,
    picture: &Picture,
) -> Result<bool, AppError> {
    use crate::domain::user_settings::VersioningMode;
    let snapshot = match versioning_mode {
        VersioningMode::None => false,
        VersioningMode::OriginalCopy => {
            !PictureVersionRepository::has_versions(db, picture.id).await?
        }
        VersioningMode::FullVersioning => true,
    };
    if !snapshot {
        return Ok(false);
    }

    let version_id = Uuid::new_v4();
    storage
        .copy_object(
            &config.s3_bucket_pictures,
            &s3::picture_key(picture.local_user_id, picture.id),
            &config.s3_bucket_versions,
            &s3::version_key(picture.local_user_id, picture.id, version_id),
        )
        .await?;

    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;
    let version_num = PictureVersionRepository::next_version_number(&mut *tx, picture.id).await?;
    PictureVersionRepository::create(
        &mut *tx,
        version_id,
        picture.id,
        version_num,
        picture.file_size,
        picture.mime_type.as_deref(),
    )
    .await?;
    tx.commit().await.map_err(map_sqlx_error)?;
    Ok(true)
}

/// Soft-delete a picture the user holds (owned or received), setting `deleted_reason = 'manual'`
/// (09 §5). The row is re-dirtied and the pipeline woken: for an **owned** picture this re-announces
/// it to recipients carrying the owner-deletion lifecycle flag (it stays in share coverage until the
/// purge sweep removes it); for a **received** picture the delete is purely local (never announced,
/// never affects downstream relay). Returns the updated picture.
#[tracing::instrument(skip(db, waker), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn trash_picture(
    db: &PgPool,
    waker: &crate::infra::pipeline::PipelineWaker,
    user_id: Uuid,
    picture_id: Uuid,
) -> Result<Picture, AppError> {
    set_trashed(db, waker, user_id, picture_id, true).await
}

/// Restore a soft-deleted picture (clear `deleted_at`/`deleted_reason`). For an owned picture this
/// re-announces with the lifecycle flag cleared (09 §5.1). Returns the updated picture.
#[tracing::instrument(skip(db, waker), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn restore_picture(
    db: &PgPool,
    waker: &crate::infra::pipeline::PipelineWaker,
    user_id: Uuid,
    picture_id: Uuid,
) -> Result<Picture, AppError> {
    set_trashed(db, waker, user_id, picture_id, false).await
}

#[tracing::instrument(skip(db, waker), fields(user_id = %user_id, picture_id = %picture_id, deleted))]
async fn set_trashed(
    db: &PgPool,
    waker: &crate::infra::pipeline::PipelineWaker,
    user_id: Uuid,
    picture_id: Uuid,
    deleted: bool,
) -> Result<Picture, AppError> {
    use crate::repository::pipeline::PipelineRepository;
    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;
    let ok = PictureRepository::set_deleted(&mut *tx, user_id, picture_id, deleted).await?;
    if !ok {
        return Err(AppError::NotFound);
    }
    // Re-dirty so the announcement reconcile re-delivers the lifecycle change (owned) and tagging
    // re-evaluates; harmless for received rows.
    PipelineRepository::invalidate(&mut *tx, &[picture_id]).await?;
    tx.commit().await.map_err(map_sqlx_error)?;
    waker.wake(user_id);
    PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)
}

/// Apply a recipient's local EXIF override to a **received** picture (09 §6.2): write the sparse
/// per-field key set into `local_exif_overrides`, re-materialise `exif_data` + promoted columns from
/// `merge(remote_exif_data, overrides)`, and fire the local `metadata` event (re-dirty + wake). DB
/// only — no `edit_picture` job, no file reconcile (the recipient does not own the file). `set`
/// fields claim the override; `clear` fields drop the override (the owner's value flows through
/// again). Returns the updated picture.
#[tracing::instrument(skip(db, waker, set, clear), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn override_received_exif(
    db: &PgPool,
    waker: &crate::infra::pipeline::PipelineWaker,
    user_id: Uuid,
    picture_id: Uuid,
    set: crate::domain::job::FullExif,
    clear: Vec<crate::domain::job::ExifField>,
) -> Result<Picture, AppError> {
    use crate::repository::pipeline::PipelineRepository;

    let picture = PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if picture.local_user_id != user_id {
        return Err(AppError::NotFound);
    }
    if picture.is_owned() {
        return Err(AppError::BadRequest(
            "Local EXIF overrides apply to received pictures only; use /edit for owned pictures"
                .to_string(),
        ));
    }

    // Start from the current sticky override set; `set` claims a field (its value wins), `clear`
    // drops the claim (the owner's value flows through again).
    let mut overrides = picture
        .local_exif_overrides
        .as_ref()
        .map(|j| j.0.clone())
        .unwrap_or_default();
    overrides.apply_set(&set);
    overrides.clear_fields(&clear);

    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;
    let remote =
        PictureRepository::set_local_exif_overrides(&mut *tx, user_id, picture_id, &overrides)
            .await?
            .ok_or(AppError::NotFound)?;
    let merged = remote.merged_with(&overrides);
    PictureRepository::apply_received_materialization(
        &mut *tx,
        picture_id,
        &merged.camera,
        merged.captured_at,
        merged.gps_lat,
        merged.gps_lng,
        merged.gps_alt,
        merged.orientation,
    )
    .await?;
    PipelineRepository::invalidate(&mut *tx, &[picture_id]).await?;
    tx.commit().await.map_err(map_sqlx_error)?;

    waker.wake(user_id);
    PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)
}

/// The twelve editable EXIF fields, used to enumerate which fields a `set`/`clear` delta touches.
const ALL_EXIF_FIELDS: [crate::domain::job::ExifField; 12] = {
    use crate::domain::job::ExifField::*;
    [
        CapturedAt,
        GpsLat,
        GpsLng,
        GpsAlt,
        Orientation,
        CameraBrand,
        CameraModel,
        FocalLengthMm,
        FNumber,
        IsoSpeed,
        ExposureTimeNum,
        ExposureTimeDen,
    ]
};

/// Propose an EXIF edit on a **received** picture to its owner (10 §4.1, `mode: "propose"`).
///
/// Requires an active incoming share that grants editing (`allow_exif_edit`); otherwise `403`. The
/// delta is sent to the owner's backend (same-backend owners are short-circuited to a direct service
/// call). On success the proposed fields are **dropped from `local_exif_overrides`** so the owner's
/// authoritative value — arriving via the owner's re-announce — is no longer shadowed (09 §6.2). The
/// authoritative change lands asynchronously (owner reconcile + re-announce), so the caller returns
/// `202`. Returns the locally-updated picture (overrides cleared for the proposed fields).
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, config, federation, waker), fields(user_id = %user_id))]
pub async fn propose_received_exif(
    db: &PgPool,
    cache: &dyn Cache,
    config: &Config,
    federation: &FederationClient,
    waker: &crate::infra::pipeline::PipelineWaker,
    user_id: Uuid,
    requester_username: &str,
    picture_id: Uuid,
    set: crate::domain::job::FullExif,
    clear: Vec<crate::domain::job::ExifField>,
) -> Result<Picture, AppError> {
    use crate::repository::pipeline::PipelineRepository;
    use crate::repository::share::IncomingShareRepository;

    let picture = PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if picture.local_user_id != user_id {
        return Err(AppError::NotFound);
    }
    if picture.is_owned() {
        return Err(AppError::BadRequest(
            "EXIF proposals apply to received pictures only; use /edit for owned pictures"
                .to_string(),
        ));
    }

    // Gate: an active incoming share covering this picture must grant EXIF editing (10 §4.1).
    if IncomingShareRepository::find_active_exif_editable_for_picture(db, picture_id, user_id)
        .await?
        .is_none()
    {
        return Err(AppError::Forbidden(
            "this share does not authorise EXIF editing; use a local override instead".to_string(),
        ));
    }

    let owner_username = picture.owner_username.clone().unwrap_or_default();
    let owner_instance = picture.owner_instance_domain.clone().unwrap_or_default();
    let remote_id = picture.remote_picture_id.clone().ok_or_else(|| {
        AppError::InternalServerError("received picture missing remote_picture_id".into())
    })?;

    // Deliver the proposal to the owner. Same-backend owner → direct service call (mirrors the
    // share-announce same-backend short-circuit); cross-instance → federation. The owner validates
    // the fields and re-checks the grant, so an invalid/forbidden proposal errors here *before* we
    // clear any local override.
    if find_local_user_id(cache, db, config, &owner_username, &owner_instance)
        .await?
        .is_some()
    {
        crate::services::federation::receive_picture_edit_request(
            db,
            waker,
            &remote_id,
            requester_username,
            &config.global_domain,
            set.clone(),
            clear.clone(),
        )
        .await?;
    } else {
        federation
            .send_picture_edit_request(
                requester_username,
                &owner_username,
                &owner_instance,
                &crate::clients::federation::models::PictureEditRequest {
                    picture_id: remote_id,
                    requester_username: requester_username.to_string(),
                    requester_instance: config.global_domain.clone(),
                    set: set.clone(),
                    clear: clear.clone(),
                    idempotency_key: Uuid::new_v4().to_string(),
                },
            )
            .await?;
    }

    // Escalate clears the per-field local override so the owner's applied value (arriving via the
    // re-announce) is authoritative (09 §6.2 / 10 §2). Drop every field the proposal touched.
    let mut touched: Vec<crate::domain::job::ExifField> = clear.clone();
    for f in ALL_EXIF_FIELDS {
        if set.has(f) && !touched.contains(&f) {
            touched.push(f);
        }
    }
    let mut overrides = picture
        .local_exif_overrides
        .as_ref()
        .map(|j| j.0.clone())
        .unwrap_or_default();
    overrides.clear_fields(&touched);

    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;
    let remote =
        PictureRepository::set_local_exif_overrides(&mut *tx, user_id, picture_id, &overrides)
            .await?
            .ok_or(AppError::NotFound)?;
    let merged = remote.merged_with(&overrides);
    PictureRepository::apply_received_materialization(
        &mut *tx,
        picture_id,
        &merged.camera,
        merged.captured_at,
        merged.gps_lat,
        merged.gps_lng,
        merged.gps_alt,
        merged.orientation,
    )
    .await?;
    PipelineRepository::invalidate(&mut *tx, &[picture_id]).await?;
    tx.commit().await.map_err(map_sqlx_error)?;
    waker.wake(user_id);

    PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)
}

#[tracing::instrument(skip(db), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn get_picture_details(
    db: &PgPool,
    user_id: Uuid,
    picture_id: Uuid,
) -> Result<PictureDetails, AppError> {
    let picture = PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if picture.local_user_id != user_id {
        return Err(AppError::NotFound);
    }
    let versions = PictureVersionRepository::list_by_picture(db, picture_id).await?;
    Ok(PictureDetails { picture, versions })
}

/// Build the flat `TagPredicate` from the public list params (§6.3). Returns `None` when no flat
/// filter field is set. Comma-separated ltree paths; `match` selects AND/OR. No `exact`/
/// `minus_children` — hierarchy depth is only produced server-side by the resolver for `browse`.
fn build_flat_predicate(params: &PictureListParams) -> Result<Option<TagPredicate>, AppError> {
    fn split_parse(raw: &Option<String>) -> Result<Vec<TagPath>, AppError> {
        let Some(raw) = raw else { return Ok(vec![]) };
        raw.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            // Filtering (read-only) may reference protected `SharedToMe` paths.
            .map(|s| TagPath::parse(s, true).map_err(AppError::BadRequest))
            .collect()
    }

    let include = split_parse(&params.include_tags)?;
    let exclude = split_parse(&params.exclude_tags)?;

    if params.untagged && (!include.is_empty() || !exclude.is_empty()) {
        return Err(AppError::BadRequest(
            "untagged is mutually exclusive with include_tags/exclude_tags".to_string(),
        ));
    }
    if !params.untagged && include.is_empty() && exclude.is_empty() {
        return Ok(None);
    }

    let match_all = match params.match_mode.as_deref() {
        None | Some("all") => true,
        Some("any") => false,
        Some(other) => {
            return Err(AppError::BadRequest(format!(
                "invalid match mode {other:?} (expected \"all\" or \"any\")"
            )));
        }
    };

    Ok(Some(TagPredicate {
        include,
        match_all,
        exclude,
        untagged: params.untagged,
        exact: vec![],
        and_terms: vec![],
        minus_children: vec![],
    }))
}

#[tracing::instrument(skip(db, cache, storage, config, federation, params), fields(user_id = %user_id))]
pub async fn list_pictures(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    config: &Config,
    federation: &FederationClient,
    user_id: Uuid,
    params: PictureListParams,
) -> Result<PictureListResult, AppError> {
    if params.page_size > 200 {
        return Err(AppError::BadRequest(
            "page_size cannot exceed 200".to_string(),
        ));
    }

    let predicate = build_flat_predicate(&params)?;

    let filter = PictureListFilter {
        page: params.page as i64,
        page_size: params.page_size as i64,
        sort: params.sort,
        order: params.order,
        tag: params.tag,
        predicate,
        owned_only: params.owned_only,
        shared_with_me: params.shared_with_me,
        include_deleted: params.include_deleted,
        captured_after: params.captured_after.map(|dt| dt.naive_utc()),
        captured_before: params.captured_before.map(|dt| dt.naive_utc()),
    };

    list_with_filter(
        db,
        cache,
        storage,
        config,
        federation,
        user_id,
        filter,
        params.thumbnail,
    )
    .await
}

/// Run a picture list against a pre-built [`PictureListFilter`], presigning thumbnails for the
/// returned page. Shared by the public `GET /pictures` list and the hierarchy `browse` endpoint
/// (which builds its `filter.predicate` server-side from the resolver).
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, storage, config, federation, filter), fields(user_id = %user_id))]
pub async fn list_with_filter(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    config: &Config,
    federation: &FederationClient,
    user_id: Uuid,
    filter: PictureListFilter,
    thumbnail: Option<ThumbnailSize>,
) -> Result<PictureListResult, AppError> {
    let page = filter.page as u32;
    let page_size = filter.page_size as u32;

    let (pictures, total) = PictureRepository::list(db, user_id, &filter).await?;

    // Batch-presign thumbnails: one cache lookup + one HTTP call per remote owner backend
    // instead of N sequential calls.
    let thumbnail_urls = if let Some(variant) = thumbnail {
        Some(
            presign_for_picture_list(
                db, cache, storage, config, federation, user_id, &pictures, variant,
            )
            .await?,
        )
    } else {
        None
    };

    let items = pictures
        .into_iter()
        .map(|pic| PictureListItem {
            id: pic.id,
            filename: pic.filename,
            width: pic.width,
            height: pic.height,
            captured_at: pic.captured_at,
            ingested_at: pic.ingested_at,
            blurhash: pic.blurhash,
            orientation: pic.orientation,
            thumbnail_url: thumbnail_urls
                .as_ref()
                .and_then(|m| m.get(&pic.id))
                .cloned(),
            owned: pic.remote_picture_id.is_none(),
            exif_sync_status: pic.exif_sync_status,
            deleted_at: pic.deleted_at,
            owner_deleted_at: pic.owner_deleted_at,
            owner_purge_at: pic.owner_purge_at,
            owner_username: pic.owner_username,
            owner_instance: pic.owner_instance_domain,
        })
        .collect();

    Ok(PictureListResult {
        total,
        page,
        page_size,
        items,
    })
}

/// Resolve presigned URLs for a list of pictures at the given variant in a single pass.
///
/// Strategy:
/// 1. Cache check for all pictures.
/// 2. Owned + same-backend cache misses: individual local S3 presigns (cheap, no network hop).
/// 3. Cross-instance cache misses: grouped by (owner_username, owner_instance) → one HTTP call
///    per remote owner backend instead of one call per picture.
#[tracing::instrument(skip(db, cache, storage, config, federation, pictures, _local_user_id), fields(user_id = %_local_user_id))]
async fn presign_for_picture_list(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    config: &Config,
    federation: &FederationClient,
    _local_user_id: Uuid,
    pictures: &[Picture],
    variant: PictureVariant,
) -> Result<HashMap<Uuid, String>, AppError> {
    let ttl = config
        .s3_presign_ttl_secs
        .saturating_sub(config.s3_presign_cache_margin_secs);

    let mut urls: HashMap<Uuid, String> = HashMap::new();
    let mut misses: Vec<&Picture> = Vec::new();

    // Step 1: cache check
    for pic in pictures {
        match cache
            .get_str(RedisKey::PictureUrl(pic.id, variant.as_str()))
            .await?
        {
            Some(url) => {
                urls.insert(pic.id, url);
            }
            None => misses.push(pic),
        }
    }

    if misses.is_empty() {
        return Ok(urls);
    }

    // Step 2: classify cache misses
    let mut owned_misses: Vec<&Picture> = Vec::new();
    let mut same_backend_misses: Vec<(&Picture, Uuid)> = Vec::new();
    let mut cross_instance_groups: HashMap<(String, String), Vec<&Picture>> = HashMap::new();

    for pic in &misses {
        if pic.is_owned() {
            owned_misses.push(pic);
        } else {
            let owner_username = pic.owner_username.as_deref().unwrap_or_default();
            let owner_instance = pic.owner_instance_domain.as_deref().unwrap_or_default();
            if let Some(owner_id) =
                find_local_user_id(cache, db, config, owner_username, owner_instance).await?
            {
                same_backend_misses.push((pic, owner_id));
            } else {
                cross_instance_groups
                    .entry((owner_username.to_string(), owner_instance.to_string()))
                    .or_default()
                    .push(pic);
            }
        }
    }

    // Step 3: presign owned pictures locally
    for pic in owned_misses {
        let key = s3::picture_key(pic.local_user_id, pic.id);
        let url = storage.presign_get(variant.bucket(config), &key).await?;
        if ttl > 0 {
            let _ = cache
                .set_str_ex(RedisKey::PictureUrl(pic.id, variant.as_str()), &url, ttl)
                .await;
        }
        urls.insert(pic.id, url);
    }

    // Step 4: presign same-backend received pictures locally (using sender's key)
    for (pic, owner_id) in same_backend_misses {
        let remote_id: Uuid = pic
            .remote_picture_id
            .as_deref()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                AppError::InternalServerError("received picture missing remote_picture_id".into())
            })?;
        let key = s3::picture_key(owner_id, remote_id);
        let url = storage.presign_get(variant.bucket(config), &key).await?;
        if ttl > 0 {
            let _ = cache
                .set_str_ex(RedisKey::PictureUrl(pic.id, variant.as_str()), &url, ttl)
                .await;
        }
        urls.insert(pic.id, url);
    }

    // Step 5: batch-presign cross-instance pictures — one HTTP call per remote owner backend.
    // Each picture is authorised by its own per-picture token (stored on its incoming_share tag).
    for ((owner_username, owner_instance), pics) in &cross_instance_groups {
        // Resolve the per-picture token for each picture; skip any without an active token.
        let mut token_to_pic: HashMap<Uuid, &Picture> = HashMap::new();
        let mut batch: Vec<(Uuid, &str)> = Vec::new();
        for pic in pics {
            if let Some(token) = TagRepository::find_active_picture_token(db, pic.id).await? {
                token_to_pic.insert(token, pic);
                batch.push((token, variant.as_str()));
            }
        }
        if batch.is_empty() {
            continue;
        }

        let remote_urls = federation
            .presign_remote_pictures(owner_username, owner_instance, &batch)
            .await?;

        for (token, url) in remote_urls {
            if let Some(pic) = token_to_pic.get(&token) {
                if ttl > 0 {
                    let _ = cache
                        .set_str_ex(RedisKey::PictureUrl(pic.id, variant.as_str()), &url, ttl)
                        .await;
                }
                urls.insert(pic.id, url);
            }
        }
    }

    Ok(urls)
}

#[tracing::instrument(skip(db, cache, storage, config, federation), fields(user_id = %local_user_id, picture_id = %picture_id))]
pub async fn presign_picture_variant(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    config: &Config,
    federation: &FederationClient,
    local_user_id: Uuid,
    picture_id: Uuid,
    variant: PictureVariant,
) -> Result<String, AppError> {
    let pic = PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;

    if pic.local_user_id != local_user_id {
        return Err(AppError::NotFound);
    }

    // Single cache check for all picture types (owned, same-backend share, cross-instance share).
    if let Some(cached) = cache
        .get_str(RedisKey::PictureUrl(pic.id, variant.as_str()))
        .await?
    {
        trace!("presign cache hit");
        return Ok(cached);
    }

    let url = if pic.is_owned() {
        let key = s3::picture_key(pic.local_user_id, pic.id);
        storage.presign_get(variant.bucket(config), &key).await?
    } else {
        let owner_username = pic.owner_username.as_deref().unwrap_or_default();
        let owner_instance = pic.owner_instance_domain.as_deref().unwrap_or_default();
        // The remote picture's UUID on the owner's backend is stored as remote_picture_id.
        let remote_id: Uuid = pic
            .remote_picture_id
            .as_deref()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                AppError::InternalServerError("received picture missing remote_picture_id".into())
            })?;

        // Check if the owner lives on this backend (resolver setup allows multiple backends per
        // global domain). Cache the lookup to avoid a DB hit on every picture in a listing.
        if let Some(owner_id) =
            find_local_user_id(cache, db, config, owner_username, owner_instance).await?
        {
            // Owner is on this backend — derive S3 key from their user_id + original picture id.
            let key = s3::picture_key(owner_id, remote_id);
            storage.presign_get(variant.bucket(config), &key).await?
        } else {
            // Owner is on a different backend — authorise via the picture's own token and call remote.
            let picture_token = TagRepository::find_active_picture_token(db, pic.id)
                .await?
                .ok_or_else(|| {
                    AppError::Unauthorized(format!(
                        "No active presign token for picture {}",
                        pic.id
                    ))
                })?;
            federation
                .presign_remote_pictures(
                    owner_username,
                    owner_instance,
                    &[(picture_token, variant.as_str())],
                )
                .await
                .map(|mut urls| {
                    urls.remove(&picture_token).ok_or_else(|| {
                        AppError::InternalServerError(format!(
                            "Remote backend did not return presigned URL for picture {}",
                            pic.id
                        ))
                    })
                })??
        }
    };

    let ttl = config
        .s3_presign_ttl_secs
        .saturating_sub(config.s3_presign_cache_margin_secs);
    if ttl > 0 {
        cache
            .set_str_ex(RedisKey::PictureUrl(pic.id, variant.as_str()), &url, ttl)
            .await?;
    }
    Ok(url)
}
