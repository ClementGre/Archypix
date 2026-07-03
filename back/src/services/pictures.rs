use crate::clients::federation::FederationClient;
use crate::domain::hierarchy::TagPredicate;
use crate::domain::picture::{Picture, PictureVersion, UploadSession};
use crate::domain::tag::TagPath;
use crate::infra::config::Config;
use crate::infra::error::{AppError, map_sqlx_error};
use crate::infra::redis::{Cache, RedisKey, cache_get_json, cache_set_json_ex};
use crate::infra::routine::RoutineHandle;
use crate::infra::s3::{self, Storage};
use crate::repository::dedup::DedupRepository;
use crate::repository::picture::{
    PictureListFilter, PictureRepository, PictureSortField, ResolvedSelection, SortOrder,
};
use crate::repository::picture_version::PictureVersionRepository;
use crate::repository::tag::TagRepository;
use crate::services::users::find_local_user_id;
use archypix_common::job::{ExifField, FullExif};
use chrono::{DateTime, NaiveDateTime, Utc};
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

    /// A generated-thumbnail variant (small/medium/large). These only exist once the worker has
    /// generated thumbnails (`pictures.thumbnails_generated_at`), which it skips for
    /// non-thumbnailable formats (PDFs, some videos, …). The `original` always exists.
    pub fn is_thumbnail(&self) -> bool {
        !matches!(self, PictureVariant::Original)
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
    /// Front-provided import label (`Uploaded.YYYY_MM_DD_HH_MM`, fixed per batch). When set, the
    /// picture is tagged with it (feature 15). A single ltree label, validated server-side.
    pub upload_label: Option<String>,
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
    /// Flat tag-set filter (§6.3). Comma-separated ltree paths; combined per `match`.
    pub include_tags: Option<String>,
    pub exclude_tags: Option<String>,
    /// Comma-separated ltree paths matched **exactly** (`tag_path = p`, no descendants) — strict
    /// tag navigation (feature 15). Combined with `include`/`exclude` per `match`.
    pub exact: Option<String>,
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
    pub mime_type: Option<String>,
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

/// One requested upload slot in a batch presign: a filename and, optionally, the client-computed
/// SHA-256 (lowercase hex) of the bytes about to be uploaded. The hash drives upload-time
/// deduplication against the user's existing owned pictures.
#[derive(Debug, Clone, Deserialize)]
pub struct BatchUploadFile {
    pub filename: String,
    pub file_hash: Option<String>,
}

/// The outcome of one batch presign slot: either a fresh upload slot (PUT the bytes to
/// `presigned_url`, then `complete`), or a deduplication hit against an existing owned picture
/// (no upload needed — `picture_id` is the existing picture).
pub enum BatchUploadOutcome {
    New {
        picture_id: Uuid,
        presigned_url: String,
    },
    Duplicate {
        picture_id: Uuid,
        was_deleted: bool,
    },
}

/// Presign upload slots for a batch of files in one call. Returns one outcome per file in input
/// order.
///
/// Validate a front-provided import label and derive the three marker tag paths (wire form):
/// `(base, base.AlreadyExisting, base.AlreadyExisting.Deleted)`.
fn upload_marker_tags(label: &str) -> Result<(String, String, String), AppError> {
    let base = TagPath::parse(label, false)
        .map_err(AppError::BadRequest)?
        .as_ltree()
        .to_string();
    Ok((
        base.clone(),
        format!("{base}.AlreadyExisting"),
        format!("{base}.AlreadyExisting.Deleted"),
    ))
}

#[tracing::instrument(skip(db, cache, storage, config, files, initial_tags, waker), fields(user_id = %user_id, count = files.len()))]
pub async fn begin_upload_batch(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    config: &Config,
    user_id: Uuid,
    files: &[BatchUploadFile],
    initial_tags: &[String],
    upload_label: Option<&str>,
    waker: &RoutineHandle<Uuid>,
) -> Result<Vec<BatchUploadOutcome>, AppError> {
    if files.is_empty() {
        return Err(AppError::BadRequest("No filenames provided".to_string()));
    }
    if files.len() > 100 {
        return Err(AppError::BadRequest(
            "Cannot request more than 100 upload slots at once".to_string(),
        ));
    }

    // Validate any initial tags up front — reject malformed paths and the reserved `SharedToMe`
    // prefix, matching the `complete`/`PATCH /tags` contract.
    let initial_tags: Vec<String> = initial_tags
        .iter()
        .map(|t| {
            TagPath::parse(t, false)
                .map(|p| p.as_ltree().to_string())
                .map_err(AppError::BadRequest)
        })
        .collect::<Result<_, _>>()?;

    // Marker tags derived from the import label (validated once up front).
    let markers = match upload_label {
        Some(label) => Some(upload_marker_tags(label)?),
        None => None,
    };

    let mut outcomes = Vec::with_capacity(files.len());
    // Live (non-deleted) and trashed existing duplicates, tagged differently below.
    let mut existing_live: Vec<Uuid> = Vec::new();
    let mut existing_deleted: Vec<Uuid> = Vec::new();
    // The canonical target for each hash seen so far in this batch — a DB-existing picture or the
    // first new slot minted for it — so a second identical file in the *same* batch dedups onto the
    // first instead of minting a redundant slot (neither is committed yet, so the DB check alone
    // can't catch it). The bool carries whether that target was trashed.
    let mut seen_hashes: HashMap<&str, (Uuid, bool)> = HashMap::new();
    for file in files {
        if let Some(hash) = file.file_hash.as_deref().filter(|h| !h.is_empty()) {
            // Earlier file in this batch already claimed this hash.
            if let Some(&(picture_id, was_deleted)) = seen_hashes.get(hash) {
                outcomes.push(BatchUploadOutcome::Duplicate {
                    picture_id,
                    was_deleted,
                });
                continue;
            }
            // Already on an existing owned picture (including a trashed one — flagged, not restored).
            if let Some(existing) =
                PictureRepository::find_owned_by_hash(db, user_id, hash, true).await?
            {
                let was_deleted = existing.deleted_at.is_some();
                seen_hashes.insert(hash, (existing.id, was_deleted));
                if was_deleted {
                    existing_deleted.push(existing.id);
                } else {
                    existing_live.push(existing.id);
                }
                outcomes.push(BatchUploadOutcome::Duplicate {
                    picture_id: existing.id,
                    was_deleted,
                });
                continue;
            }
            // First time we see this hash — mint a slot and remember it as the canonical target.
            let (picture_id, presigned_url) =
                begin_upload(cache, storage, config, user_id, &file.filename).await?;
            seen_hashes.insert(hash, (picture_id, false));
            outcomes.push(BatchUploadOutcome::New {
                picture_id,
                presigned_url,
            });
            continue;
        }
        // No hash supplied — can't dedup; always a fresh slot.
        let (picture_id, presigned_url) =
            begin_upload(cache, storage, config, user_id, &file.filename).await?;
        outcomes.push(BatchUploadOutcome::New {
            picture_id,
            presigned_url,
        });
    }

    // Tags to land on each duplicate class: the user's `initial_tags` plus the import marker.
    let live_tags: Vec<String> = initial_tags
        .iter()
        .cloned()
        .chain(markers.as_ref().map(|(_, already, _)| already.clone()))
        .collect();
    let deleted_tags: Vec<String> = initial_tags
        .iter()
        .cloned()
        .chain(markers.as_ref().map(|(_, _, deleted)| deleted.clone()))
        .collect();

    let tag_live = !existing_live.is_empty() && !live_tags.is_empty();
    let tag_deleted = !existing_deleted.is_empty() && !deleted_tags.is_empty();
    if tag_live || tag_deleted {
        let mut tx = db
            .begin()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        if tag_live {
            // batch_assign re-dirties the (live) pictures the user already holds.
            TagRepository::batch_assign(&mut *tx, user_id, &existing_live, &live_tags).await?;
        }
        if tag_deleted {
            TagRepository::batch_assign_including_deleted(
                &mut *tx,
                user_id,
                &existing_deleted,
                &deleted_tags,
            )
            .await?;
        }
        tx.commit().await.map_err(map_sqlx_error)?;
        waker.trigger_debounced(user_id);
    }

    Ok(outcomes)
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
    let mut initial_tags: Vec<String> = match meta.initial_tags.as_ref() {
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
    // Feature 15: tag a freshly-uploaded picture with the front's import label (`Uploaded....`).
    if let Some(label) = meta.upload_label.as_deref() {
        let (base, _, _) = upload_marker_tags(label)?;
        initial_tags.push(base);
    }

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
    crate::services::jobs::enqueue_thumbnail_job(
        &mut *tx,
        user_id,
        picture_id,
        true,
        meta.file_hash.as_deref(),
    )
    .await?;

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

/// Copy a received (or owned) picture into the caller's library as a new, independent owned picture
/// (feature 11 §3): a fresh id, `copy_source_*` provenance root, server-side byte copy (S3 copy for a
/// local source, presign+fetch for a cross-instance owner), seeded effective EXIF, and a
/// `gen_thumbnail` enqueue. See doc/features/11 §3.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, storage, config, federation, waker), fields(user_id = %user_id, source_id = %source_picture_id))]
pub async fn copy_picture(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    config: &Config,
    federation: &FederationClient,
    waker: &crate::infra::routine::RoutineHandle<uuid::Uuid>,
    user_id: Uuid,
    caller_username: &str,
    source_picture_id: Uuid,
) -> Result<Picture, AppError> {
    let source = PictureRepository::find_by_id(db, source_picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if source.local_user_id != user_id {
        return Err(AppError::NotFound);
    }

    let new_id = Uuid::new_v4();
    let new_key = s3::picture_key(user_id, new_id);

    // ── Copy the bytes into the caller's pictures object ──────────────────────
    if source.is_owned() {
        storage
            .copy_object(
                &config.s3_bucket_pictures,
                &s3::picture_key(user_id, source.id),
                &config.s3_bucket_pictures,
                &new_key,
            )
            .await?;
    } else {
        let owner_username = source.owner_username.as_deref().unwrap_or_default();
        let owner_instance = source.owner_instance_domain.as_deref().unwrap_or_default();
        let remote_id: Uuid = source
            .remote_picture_id
            .as_deref()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                AppError::InternalServerError("received picture missing remote_picture_id".into())
            })?;
        if let Some(owner_id) =
            find_local_user_id(cache, db, config, owner_username, owner_instance).await?
        {
            storage
                .copy_object(
                    &config.s3_bucket_pictures,
                    &s3::picture_key(owner_id, remote_id),
                    &config.s3_bucket_pictures,
                    &new_key,
                )
                .await?;
        } else {
            // Cross-instance: the owner must be reachable. Presign the original via the picture's
            // own token, download the bytes, and upload them under the caller's key.
            let token = TagRepository::find_active_picture_token(db, source.id)
                .await?
                .ok_or_else(|| {
                    AppError::Unauthorized(format!(
                        "no active presign token for picture {}",
                        source.id
                    ))
                })?;
            let mut urls = federation
                .presign_remote_pictures(owner_username, owner_instance, &[(token, "original")])
                .await?;
            let url = urls.remove(&token).ok_or_else(|| {
                AppError::InternalServerError("owner backend returned no presigned URL".into())
            })?;
            let resp = reqwest::get(&url)
                .await
                .map_err(|e| AppError::InternalServerError(format!("copy fetch failed: {e}")))?
                .error_for_status()
                .map_err(|e| AppError::InternalServerError(format!("copy fetch status: {e}")))?;
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| AppError::InternalServerError(format!("copy read failed: {e}")))?
                .to_vec();
            storage
                .put_object(
                    &config.s3_bucket_pictures,
                    &new_key,
                    bytes,
                    source.mime_type.as_deref(),
                )
                .await?;
        }
    }

    // ── Provenance root (§3 / §7.1): point at the genuine original, not the intermediary ─────
    let (cs_user, cs_instance, cs_pic) = if source.copy_source_picture_id.is_some() {
        (
            source.copy_source_owner_username.clone(),
            source.copy_source_owner_instance.clone(),
            source.copy_source_picture_id.clone(),
        )
    } else if source.is_owned() {
        (
            Some(caller_username.to_string()),
            Some(config.global_domain.clone()),
            Some(source.id.to_string()),
        )
    } else {
        (
            source.owner_username.clone(),
            source.owner_instance_domain.clone(),
            source.remote_picture_id.clone(),
        )
    };

    // ── New owned row, seeded from the source's effective EXIF ────────────────
    let eff = source.full_exif();
    let camera_json = serde_json::to_value(&eff.camera).unwrap_or_else(|_| serde_json::json!({}));

    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;
    let copy = PictureRepository::create_copy(
        &mut *tx,
        new_id,
        user_id,
        source.filename.as_deref(),
        source.mime_type.as_deref(),
        source.file_size,
        source.width,
        source.height,
        camera_json,
        eff.captured_at,
        eff.gps_lat,
        eff.gps_lng,
        eff.gps_alt,
        eff.orientation,
        cs_user.as_deref(),
        cs_instance.as_deref(),
        cs_pic.as_deref(),
    )
    .await?;
    // `is_initial = false`: keep the seeded effective EXIF (don't re-extract the owner's embedded
    // EXIF), but still compute file_size/hash, content_hash, dimensions and thumbnails.
    crate::services::jobs::enqueue_thumbnail_job(&mut *tx, user_id, new_id, false, None).await?;
    tx.commit().await.map_err(map_sqlx_error)?;

    // Wake the pipeline so the new owned picture is tagged; the dedup reconcile runs again once
    // `gen_thumbnail` lands its `content_hash` (that completion wakes the pipeline too).
    waker.trigger_debounced(user_id);

    Ok(copy)
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
    waker: &RoutineHandle<Uuid>,
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
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    picture_id: Uuid,
) -> Result<Picture, AppError> {
    set_trashed(db, waker, user_id, picture_id, false).await
}

/// Result of a batch trash/restore: the dry-run count, or the applied count.
pub enum TrashBatchOutcome {
    DryRun(crate::services::aggregate::DryRun),
    Applied { affected: i64 },
}

/// Batch soft-delete / restore over a [`ResolvedSelection`] (feature 14 §6) — a single set-based
/// UPDATE (no per-picture loop). With `dry_run` returns the affected count without mutating.
/// Re-dirties + wakes the pipeline so owned pictures re-announce their owner-deletion lifecycle.
#[tracing::instrument(skip(db, waker, sel), fields(user_id = %user_id, deleted, dry_run))]
pub async fn batch_set_trashed_selection(
    db: &PgPool,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    sel: &crate::repository::picture::ResolvedSelection,
    deleted: bool,
    dry_run: bool,
) -> Result<TrashBatchOutcome, AppError> {
    if dry_run {
        let affected = PictureRepository::count_selection(db, user_id, sel).await?;
        return Ok(TrashBatchOutcome::DryRun(
            crate::services::aggregate::DryRun {
                affected,
                ..Default::default()
            },
        ));
    }
    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;
    let affected =
        PictureRepository::batch_set_trashed_selection(&mut *tx, user_id, sel, deleted).await?;
    if deleted {
        // Reject the touched groups; the reconcile picks each representative (feature 11 §5.3).
        DedupRepository::boomerang_dedupe_in_manual_groups(&mut *tx, user_id).await?;
    } else {
        // Restore lifts the rejection (boomerang → content_dedupe), before the reconcile runs.
        DedupRepository::dedupe_boomerang_in_live_groups(&mut *tx, user_id).await?;
    }
    tx.commit().await.map_err(map_sqlx_error)?;
    waker.trigger(user_id);
    Ok(TrashBatchOutcome::Applied {
        affected: affected as i64,
    })
}

#[tracing::instrument(skip(db, waker), fields(user_id = %user_id, picture_id = %picture_id, deleted))]
async fn set_trashed(
    db: &PgPool,
    waker: &RoutineHandle<Uuid>,
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
    // Content-dedup rejection lifecycle (feature 11 §5.3): delete rejects the whole group (priority
    // copy → manual representative, rest → boomerang); restore lifts it (boomerangs → content_dedupe).
    if deleted {
        DedupRepository::reject_content_group(&mut *tx, user_id, picture_id).await?;
    } else {
        DedupRepository::dedupe_boomerang_siblings(&mut *tx, user_id, picture_id).await?;
    }
    // Re-dirty so the announcement reconcile re-delivers the lifecycle change (owned) and tagging
    // re-evaluates; harmless for received rows.
    PipelineRepository::invalidate(&mut *tx, &[picture_id]).await?;
    tx.commit().await.map_err(map_sqlx_error)?;
    waker.trigger(user_id);
    PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)
}

/// Apply a recipient's local EXIF override to a **received** picture (09 §6.2): write the sparse
/// per-field key set into `local_exif_overrides`, re-materialise `exif_data` + promoted columns from
/// `merge(remote_exif_data, overrides)`, and fire the local `metadata` event (re-dirty + wake). DB
/// only — no `edit_picture` job, no file reconcile (the recipient does not own the file). `set`
/// fields claim the override; `empty` fields claim it as empty/`null` (10 §6.3); `clear` fields drop
/// the override (the owner's value flows through again). Returns the updated picture.
#[tracing::instrument(skip(db, waker, set, empty, clear), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn override_received_exif(
    db: &PgPool,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    picture_id: Uuid,
    set: FullExif,
    empty: Vec<ExifField>,
    clear: Vec<ExifField>,
) -> Result<Picture, AppError> {
    use crate::domain::received_exif;

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

    // Same normalisation + set-based merge as batch editing: `set` claims a field, `empty` claims it
    // as empty (null), `clear` drops the claim.
    let (empty, clear) = crate::domain::validation::validate_exif_edit(&set, empty, clear)
        .map_err(AppError::BadRequest)?;
    let (patch, clear_keys) = received_exif::override_patch(&set, &empty, &clear);
    let sel = ResolvedSelection::explicit(vec![picture_id]);
    PictureRepository::batch_apply_exif_received_local_selection(
        db,
        user_id,
        &sel,
        &patch,
        &clear_keys,
    )
    .await?;

    waker.trigger(user_id);
    PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)
}

/// The twelve editable EXIF fields, used to enumerate which fields a `set`/`clear` delta touches.
const ALL_EXIF_FIELDS: [ExifField; 12] = {
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
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    requester_username: &str,
    picture_id: Uuid,
    set: FullExif,
    clear: Vec<ExifField>,
) -> Result<Picture, AppError> {
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
    let mut touched: Vec<ExifField> = clear.clone();
    for f in ALL_EXIF_FIELDS {
        if set.has(f) && !touched.contains(&f) {
            touched.push(f);
        }
    }
    // Drop the touched fields from the override via the shared set-based merge (empty patch, the
    // touched keys as the clear set) — same path as a local override / batch edit.
    let (patch, clear_keys) =
        crate::domain::received_exif::override_patch(&FullExif::default(), &[], &touched);
    let sel = ResolvedSelection::explicit(vec![picture_id]);
    PictureRepository::batch_apply_exif_received_local_selection(
        db,
        user_id,
        &sel,
        &patch,
        &clear_keys,
    )
    .await?;
    waker.trigger(user_id);

    PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)
}

/// List the content-dedup group of a picture the caller holds (feature 11 §5.5) — the survivor plus
/// its hidden `content_dedupe`/`boomerang`/`manual` siblings. The caller must hold the picture.
#[tracing::instrument(skip(db), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn picture_copies(
    db: &PgPool,
    user_id: Uuid,
    picture_id: Uuid,
) -> Result<Vec<crate::repository::dedup::CopyRow>, AppError> {
    let picture = PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if picture.local_user_id != user_id {
        return Err(AppError::NotFound);
    }
    let group = DedupRepository::list_content_group(db, user_id, picture_id).await?;
    if !group.is_empty() {
        return Ok(group);
    }
    // No content/file hash yet (still processing) → the group is just this picture.
    Ok(vec![crate::repository::dedup::CopyRow {
        id: picture.id,
        content_hash: picture.content_hash.clone(),
        file_hash: picture.file_hash.clone(),
        deleted_reason: picture.deleted_reason,
        deleted_at: picture.deleted_at,
        updated_at: picture.updated_at,
        is_owned: picture.is_owned(),
        owner_username: picture.owner_username.clone(),
        owner_instance_domain: picture.owner_instance_domain.clone(),
        owner_deleted_at: picture.owner_deleted_at.clone(),
        copy_source_owner_username: picture.copy_source_owner_username.clone(),
        copy_source_owner_instance: picture.copy_source_owner_instance.clone(),
        copy_source_picture_id: picture.copy_source_picture_id.clone(),
        filename: picture.filename.clone(),
    }])
}

/// Make `picture_id` the live survivor of its content-dedup group (feature 11 §5.5), hiding every
/// sibling as `content_dedupe`. Because the reconciler leaves a correct single-live group untouched,
/// this user choice sticks without a pin flag. The caller must hold the picture.
#[tracing::instrument(skip(db, waker), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn set_picture_survivor(
    db: &PgPool,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    picture_id: Uuid,
) -> Result<(), AppError> {
    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;

    let target = PictureRepository::find_by_id(&mut *tx, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if target.local_user_id != user_id {
        return Err(AppError::NotFound);
    }
    let was_live = target.deleted_at.is_none();
    // The previously-live sibling is the curated source of truth (its manual tag set reflects the
    // user's adds *and removes*) — capture it before the survivor flip.
    let old_live = DedupRepository::live_id_in_group(&mut *tx, user_id, picture_id).await?;

    let ok = DedupRepository::set_survivor(&mut *tx, user_id, picture_id).await?;
    if !ok {
        return Err(AppError::NotFound);
    }

    // Tag handoff (§5.5). Switching the live copy *replaces* the new survivor's manual tags with the
    // old live's exact set, so a tag the user removed from the old live stays removed. With no prior
    // live (a rejected group promoted by the user) the group's manual tags are merged in instead; a
    // re-keep of the already-sole-live copy leaves its curated set untouched.
    match old_live {
        Some(from) => {
            let paths = TagRepository::list_manual_paths(&mut *tx, from).await?;
            TagRepository::clear_manual_tags(&mut *tx, user_id, picture_id).await?;
            if !paths.is_empty() {
                TagRepository::batch_assign(&mut *tx, user_id, &[picture_id], &paths).await?;
            }
        }
        None if !was_live => {
            let paths =
                DedupRepository::group_manual_tag_paths(&mut *tx, user_id, picture_id).await?;
            if !paths.is_empty() {
                TagRepository::batch_assign(&mut *tx, user_id, &[picture_id], &paths).await?;
            }
        }
        None => {}
    }

    tx.commit().await.map_err(map_sqlx_error)?;
    // Re-announce / re-tag the now-live picture and let the reconciler confirm consistency.
    waker.trigger(user_id);
    Ok(())
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
    let exact = split_parse(&params.exact)?;

    if params.untagged && (!include.is_empty() || !exclude.is_empty() || !exact.is_empty()) {
        return Err(AppError::BadRequest(
            "untagged is mutually exclusive with include_tags/exclude_tags/exact".to_string(),
        ));
    }
    if !params.untagged && include.is_empty() && exclude.is_empty() && exact.is_empty() {
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
        exact,
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
            mime_type: pic.mime_type,
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

    // Step 1: cache check. A thumbnail variant on a picture with no generated thumbnail (pending,
    // or a non-thumbnailable format like a PDF) gets no URL — left absent so the client renders a
    // file-type placeholder instead of a broken image.
    for pic in pictures {
        if variant.is_thumbnail() && pic.thumbnails_generated_at.is_none() {
            continue;
        }
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
) -> Result<Option<String>, AppError> {
    let pic = PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;

    if pic.local_user_id != local_user_id {
        return Err(AppError::NotFound);
    }

    // No thumbnail to presign (pending, or a non-thumbnailable format) → `None` so the client shows
    // a file-type placeholder. The `original` always exists.
    if variant.is_thumbnail() && pic.thumbnails_generated_at.is_none() {
        return Ok(None);
    }

    // Single cache check for all picture types (owned, same-backend share, cross-instance share).
    if let Some(cached) = cache
        .get_str(RedisKey::PictureUrl(pic.id, variant.as_str()))
        .await?
    {
        trace!("presign cache hit");
        return Ok(Some(cached));
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
    Ok(Some(url))
}
