use crate::domain::hierarchy::TagPredicate;
use crate::domain::job::{CameraExif, FullExif};
use crate::domain::picture::{ExifSyncStatus, Picture};
use archypix_common::error::{AppError, map_sqlx_error};
use chrono::NaiveDateTime;
use serde::Deserialize;
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PictureSortField {
    CapturedAt,
    #[default]
    IngestedAt,
    UpdatedAt,
    FileSize,
    Filename,
    /// Proximity sort by `|captured_at − near_time|` (feature 29 §6). Requires `near_time`; rows
    /// without a capture date sort last; `SortOrder` is ignored (always nearest-first).
    TimeNear,
    /// Proximity sort by approximate (equirectangular) distance to `near_lat`/`near_lng` (feature 29
    /// §6). Requires both; ungeotagged rows sort last; `SortOrder` is ignored.
    GeoNear,
}

impl PictureSortField {
    /// Whether this is a reference-point proximity sort (nearest-first, order-agnostic).
    fn is_proximity(&self) -> bool {
        matches!(self, Self::TimeNear | Self::GeoNear)
    }
}

/// Per-field metadata-presence filter (feature 29 §4). AND-composed with every other list arm.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceFilter {
    /// No constraint.
    #[default]
    Any,
    /// The field is populated.
    Present,
    /// The field is NULL.
    Missing,
}

impl PresenceFilter {
    fn is_any(&self) -> bool {
        matches!(self, Self::Any)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    Asc,
    #[default]
    Desc,
}

/// Trash-membership state for a picture list. The trash is a **filter over the main view**, not a
/// separate page: `Exclude` is the normal gallery, `Only` is the trash, `Include` shows both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrashFilter {
    /// Live pictures only (`deleted_at IS NULL`) — the default gallery.
    #[default]
    Exclude,
    /// Live + `manual`-trashed pictures.
    Include,
    /// `manual`-trashed pictures only — the trash view.
    Only,
}

#[derive(Debug, Clone, Default)]
pub struct PictureListFilter {
    pub page: i64,
    pub page_size: i64,
    pub sort: PictureSortField,
    pub order: SortOrder,
    /// Generalised tag-set predicate: the flat gallery (`include`/`exclude`/`exact`/`match`/
    /// `untagged`) and hierarchy `browse` both lower to this. `None` ⇒ no tag constraint.
    pub predicate: Option<TagPredicate>,
    pub owned_only: bool,
    pub shared_with_me: bool,
    pub trash: TrashFilter,
    pub captured_after: Option<NaiveDateTime>,
    pub captured_before: Option<NaiveDateTime>,
    /// GPS-presence filter (feature 29 §4). AND-composed with the other arms.
    pub gps: PresenceFilter,
    /// Capture-date presence filter (over `captured_at`).
    pub capture_date: PresenceFilter,
    /// "Any issue" OR convenience (§4): `(gps IS NULL OR captured_at IS NULL)`. Mutually exclusive
    /// with a non-`Any` `gps`/`capture_date` (rejected at construction).
    pub missing_any: bool,
    /// Reference instant for `PictureSortField::TimeNear`.
    pub near_time: Option<NaiveDateTime>,
    /// Reference point for `PictureSortField::GeoNear`.
    pub near_lat: Option<f64>,
    pub near_lng: Option<f64>,
    /// Date-fix mode (feature 30 §4): float undated rows (`captured_at IS NULL`) to the top with a
    /// `filename, id` tiebreaker so the broken ones surface for fixing while the dated references stay
    /// scrollable below. A prefix on top of the current column sort; ignored for proximity sorts.
    pub undated_first: bool,
}

impl PictureListFilter {
    /// Reject the mutually-exclusive presence combination (§4) and a proximity sort missing its
    /// reference param (§6). Called at the wire-parse boundary so a bad request surfaces as a 400.
    pub fn validate(&self) -> Result<(), AppError> {
        if self.missing_any && (!self.gps.is_any() || !self.capture_date.is_any()) {
            return Err(AppError::BadRequest(
                "missing_any cannot be combined with a per-field gps/capture_date presence filter"
                    .to_string(),
            ));
        }
        match self.sort {
            PictureSortField::TimeNear if self.near_time.is_none() => {
                return Err(AppError::BadRequest(
                    "sort=time_near requires near_time".to_string(),
                ));
            }
            PictureSortField::GeoNear if self.near_lat.is_none() || self.near_lng.is_none() => {
                return Err(AppError::BadRequest(
                    "sort=geo_near requires near_lat and near_lng".to_string(),
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

/// A picture selection (feature 14 §2) resolved against the DB: the query lowered to a
/// [`PictureListFilter`] (`None` ⇒ pure explicit set, or a hierarchy directory with no direct files)
/// plus the explicit id deltas. The reusable membership term every batch endpoint resolves to:
/// `(filter ∪ include_ids) \ exclude_ids`, scoped to the caller. Built by `services::selection`.
#[derive(Debug, Clone)]
pub struct ResolvedSelection {
    pub filter: Option<PictureListFilter>,
    pub include_ids: Vec<Uuid>,
    pub exclude_ids: Vec<Uuid>,
}

impl ResolvedSelection {
    /// A pure explicit set over the given ids (the degenerate single-/multi-click case).
    pub fn explicit(include_ids: Vec<Uuid>) -> Self {
        Self {
            filter: None,
            include_ids,
            exclude_ids: vec![],
        }
    }

    /// True when the selection can match no picture regardless of the user's holdings (no query and
    /// no explicitly-included id). Callers short-circuit to an empty result.
    pub fn is_empty(&self) -> bool {
        self.filter.is_none() && self.include_ids.is_empty()
    }
}

/// Selection-summary aggregate (feature 14 §4.1) — all read straight off the `pictures` row.
#[derive(Debug, Default)]
pub struct SelectionSummary {
    pub count: i64,
    pub owned_count: i64,
    pub received_count: i64,
    pub total_file_size: i64,
    pub trashed_count: i64,
    pub owner_deleting_count: i64,
    pub thumbnail_pending_count: i64,
    pub duplicate_count: i64,
    /// Distinct remote owners of received pictures in the selection.
    pub owners: Vec<OwnerCount>,
    /// `exif_sync_status` histogram (label → count), including `pending_job_creation`.
    pub exif_sync: Vec<(ExifSyncStatus, i64)>,
}

#[derive(Debug)]
pub struct OwnerCount {
    pub username: String,
    pub instance: String,
    pub count: i64,
}

/// Min/max/avg of a numeric field over the selection (`null_count` = rows where the field is NULL).
#[derive(Debug, Default, Clone)]
pub struct NumericAgg {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub avg: Option<f64>,
    pub null_count: i64,
}

/// Min/max range + avg instant of a date field over the selection.
#[derive(Debug, Default, Clone)]
pub struct DateAgg {
    pub min: Option<NaiveDateTime>,
    pub max: Option<NaiveDateTime>,
    pub avg: Option<NaiveDateTime>,
    pub null_count: i64,
}

/// Exact bounding box + centroid of the GPS points in the selection.
#[derive(Debug, Default, Clone)]
pub struct GpsAgg {
    pub lat_min: Option<f64>,
    pub lat_max: Option<f64>,
    pub lng_min: Option<f64>,
    pub lng_max: Option<f64>,
    pub centroid_lat: Option<f64>,
    pub centroid_lng: Option<f64>,
    pub null_count: i64,
}

/// Distinct-value histogram of a string/enum field over the selection.
#[derive(Debug, Default, Clone)]
pub struct DistinctAgg {
    /// `(value, count)` pairs ordered by descending count (NULLs excluded — see `null_count`).
    pub values: Vec<(String, i64)>,
    pub null_count: i64,
}

pub struct PictureRepository;

impl PictureRepository {
    #[tracing::instrument(skip(ex), fields(picture_id = %id, user_id = %local_user_id))]
    pub async fn create<'e, E>(
        ex: E,
        id: Uuid,
        local_user_id: Uuid,
        filename: Option<&str>,
        mime_type: Option<&str>,
        file_size: Option<i64>,
        width: Option<i32>,
        height: Option<i32>,
        exif_data: Option<serde_json::Value>,
        captured_at: Option<NaiveDateTime>,
        original_file_created_at: Option<NaiveDateTime>,
    ) -> Result<Picture, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let exif_json = exif_data.unwrap_or_else(|| serde_json::json!({}));
        sqlx::query_as!(
            Picture,
            r#"INSERT INTO pictures (id, local_user_id, filename, mime_type, file_size, width, height, exif_data, metadata, captured_at, original_file_created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, '{}'::jsonb, $9, $10)
               RETURNING id, local_user_id, remote_picture_id, owner_username, owner_instance_domain,
                         filename, mime_type, file_size, width, height,
                         exif_data as "exif_data: _", metadata as "metadata: _",
                         deleted_at, deleted_reason as "deleted_reason: _",
                         owner_deleted_at, owner_purge_at,
                         remote_exif_data as "remote_exif_data: _",
                         local_exif_overrides as "local_exif_overrides: _",
                         captured_at, ingested_at, updated_at, remote_updated_at,
                         blurhash, gps_lat, gps_lng, gps_alt, orientation, thumbnails_generated_at,
                         file_hash, exif_sync_status as "exif_sync_status: _",
                         content_hash, copy_source_owner_username,
                         copy_source_owner_instance, copy_source_picture_id,
                         creator, creator_override, original_file_created_at"#,
            id,
            local_user_id,
            filename,
            mime_type,
            file_size,
            width,
            height,
            serde_json::Value::from(exif_json) as serde_json::Value,
            captured_at,
            original_file_created_at,
        )
            .fetch_one(ex)
            .await
            .map_err(map_sqlx_error)
    }

    /// Create a **physical copy** as a new owned picture (feature 11 §3): `local_user_id = caller`,
    /// `remote_picture_id`/`owner_*` NULL, `copy_source_*` carrying the provenance **root** (the
    /// genuine original's owner identity). The EXIF is seeded from the source's *effective* values at
    /// copy time (a copy is a snapshot — it does not stay linked to the owner). `content_hash`/
    /// `file_hash`/thumbnails are filled by the enqueued `gen_thumbnail`.
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip(ex, exif_data), fields(picture_id = %id, user_id = %local_user_id))]
    pub async fn create_copy<'e, E>(
        ex: E,
        id: Uuid,
        local_user_id: Uuid,
        filename: Option<&str>,
        mime_type: Option<&str>,
        file_size: Option<i64>,
        width: Option<i32>,
        height: Option<i32>,
        exif_data: serde_json::Value,
        captured_at: Option<NaiveDateTime>,
        gps_lat: Option<f64>,
        gps_lng: Option<f64>,
        gps_alt: Option<i32>,
        orientation: Option<i16>,
        copy_source_owner_username: Option<&str>,
        copy_source_owner_instance: Option<&str>,
        copy_source_picture_id: Option<&str>,
        creator: Option<&str>,
    ) -> Result<Picture, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            Picture,
            r#"INSERT INTO pictures (id, local_user_id, filename, mime_type, file_size, width, height,
                                     exif_data, metadata, captured_at, gps_lat, gps_lng, gps_alt, orientation,
                                     copy_source_owner_username, copy_source_owner_instance, copy_source_picture_id,
                                     creator)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, '{}'::jsonb, $9, $10, $11, $12, $13, $14, $15, $16, $17)
               RETURNING id, local_user_id, remote_picture_id, owner_username, owner_instance_domain,
                         filename, mime_type, file_size, width, height,
                         exif_data as "exif_data: _", metadata as "metadata: _",
                         deleted_at, deleted_reason as "deleted_reason: _",
                         owner_deleted_at, owner_purge_at,
                         remote_exif_data as "remote_exif_data: _",
                         local_exif_overrides as "local_exif_overrides: _",
                         captured_at, ingested_at, updated_at, remote_updated_at,
                         blurhash, gps_lat, gps_lng, gps_alt, orientation, thumbnails_generated_at,
                         file_hash, exif_sync_status as "exif_sync_status: _",
                         content_hash, copy_source_owner_username,
                         copy_source_owner_instance, copy_source_picture_id,
                         creator, creator_override, original_file_created_at"#,
            id,
            local_user_id,
            filename,
            mime_type,
            file_size,
            width,
            height,
            exif_data,
            captured_at,
            gps_lat,
            gps_lng,
            gps_alt,
            orientation,
            copy_source_owner_username,
            copy_source_owner_instance,
            copy_source_picture_id,
            creator,
        )
            .fetch_one(ex)
            .await
            .map_err(map_sqlx_error)
    }

    /// Create or refresh a received (non-owned) picture row on behalf of a recipient user.
    ///
    /// `remote_picture_id` is the sender's picture UUID (stored as string for cross-instance compat).
    /// Deduplication is handled by the `uq_received_picture` unique index. On conflict the row's
    /// owner-authoritative state — `remote_exif_data`, `owner_deleted_at`, `owner_purge_at` — is
    /// refreshed while the recipient's `local_exif_overrides` are **preserved** (09 §8). The caller
    /// then re-materialises `exif_data` + the promoted columns from the merge via
    /// [`apply_received_materialization`]; this method does not touch them.
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip(ex, remote_exif_data), fields(user_id = %recipient_id))]
    pub async fn create_received<'e, E>(
        ex: E,
        recipient_id: Uuid,
        remote_picture_id: &str,
        owner_username: &str,
        owner_instance_domain: &str,
        filename: Option<&str>,
        mime_type: Option<&str>,
        file_size: Option<i64>,
        width: Option<i32>,
        height: Option<i32>,
        blurhash: Option<&String>,
        file_hash: Option<&str>,
        content_hash: Option<&str>,
        thumbnails_generated_at: Option<NaiveDateTime>,
        remote_exif_data: &FullExif,
        owner_deleted_at: Option<NaiveDateTime>,
        owner_purge_at: Option<NaiveDateTime>,
        creator: Option<&str>,
        remote_updated_at: Option<NaiveDateTime>,
    ) -> Result<Picture, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        // The owner snapshot is stored as a JSONB object (camera/lens keys + promoted keys flattened).
        let remote_exif_json = serde_json::to_value(remote_exif_data)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        sqlx::query_as!(
            Picture,
            r#"INSERT INTO pictures
                   (local_user_id, remote_picture_id, owner_username, owner_instance_domain,
                    filename, mime_type, file_size, width, height, metadata,
                    blurhash, file_hash, content_hash, thumbnails_generated_at,
                    remote_exif_data, owner_deleted_at, owner_purge_at, creator, remote_updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, '{}'::jsonb,
                       $10, $11, $16, $12, $13, $14, $15, $17, $18)
               ON CONFLICT (local_user_id, remote_picture_id)
               WHERE remote_picture_id IS NOT NULL
               DO UPDATE SET
                   filename  = COALESCE(EXCLUDED.filename,  pictures.filename),
                   mime_type = COALESCE(EXCLUDED.mime_type, pictures.mime_type),
                   file_size = COALESCE(EXCLUDED.file_size, pictures.file_size),
                   width     = COALESCE(EXCLUDED.width,     pictures.width),
                   height    = COALESCE(EXCLUDED.height,    pictures.height),
                   blurhash  = COALESCE(EXCLUDED.blurhash,  pictures.blurhash),
                   file_hash   = COALESCE(EXCLUDED.file_hash, pictures.file_hash),
                   content_hash = COALESCE(EXCLUDED.content_hash, pictures.content_hash),
                   thumbnails_generated_at = COALESCE(EXCLUDED.thumbnails_generated_at,
                                                      pictures.thumbnails_generated_at),
                   -- Owner-authoritative state is refreshed; local_exif_overrides + creator_override
                   -- (the recipient's own relabel) are preserved. The stale-announce guard (§7) is
                   -- applied by the caller before this upsert; here we just stamp the new value.
                   remote_exif_data = EXCLUDED.remote_exif_data,
                   owner_deleted_at = EXCLUDED.owner_deleted_at,
                   owner_purge_at   = EXCLUDED.owner_purge_at,
                   creator          = EXCLUDED.creator,
                   remote_updated_at = COALESCE(EXCLUDED.remote_updated_at, pictures.remote_updated_at)
               RETURNING id, local_user_id, remote_picture_id, owner_username, owner_instance_domain,
                         filename, mime_type, file_size, width, height,
                         exif_data as "exif_data: _", metadata as "metadata: _",
                         deleted_at, deleted_reason as "deleted_reason: _",
                         owner_deleted_at, owner_purge_at,
                         remote_exif_data as "remote_exif_data: _",
                         local_exif_overrides as "local_exif_overrides: _",
                         captured_at, ingested_at, updated_at, remote_updated_at,
                         blurhash, gps_lat, gps_lng, gps_alt, orientation, thumbnails_generated_at,
                         file_hash, exif_sync_status as "exif_sync_status: _",
                         content_hash, copy_source_owner_username,
                         copy_source_owner_instance, copy_source_picture_id,
                         creator, creator_override, original_file_created_at"#,
            recipient_id,
            remote_picture_id,
            owner_username,
            owner_instance_domain,
            filename,
            mime_type,
            file_size,
            width,
            height,
            blurhash,
            file_hash,
            thumbnails_generated_at,
            remote_exif_json,
            owner_deleted_at,
            owner_purge_at,
            content_hash,
            creator,
            remote_updated_at,
        )
            .fetch_one(ex)
            .await
            .map_err(map_sqlx_error)
    }

    /// The last-applied owner `updated_at` for a received row, for the stale-announcement guard
    /// (feature 28 §7). Outer `None` ⇒ no such received row yet; inner `None` ⇒ a row from a peer
    /// that predated the field. Keyed by the received-picture unique `(local_user_id, remote_picture_id)`.
    pub async fn received_remote_updated_at<'e, E>(
        ex: E,
        recipient_id: Uuid,
        remote_picture_id: &str,
    ) -> Result<Option<Option<NaiveDateTime>>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let row = sqlx::query!(
            r#"SELECT remote_updated_at FROM pictures
               WHERE local_user_id = $1 AND remote_picture_id = $2"#,
            recipient_id,
            remote_picture_id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(row.map(|r| r.remote_updated_at))
    }

    /// Re-materialise a received row's `exif_data` + promoted columns from the
    /// `merge(remote_exif_data, local_exif_overrides)` the caller computed (09 §6/§8). Bumps
    /// `updated_at` (announcement re-delivery gate) and re-dirties the row for the local `metadata`
    /// event (date/GPS rules re-evaluate on the merged EXIF).
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip(ex, camera), fields(picture_id = %id))]
    pub async fn apply_received_materialization<'e, E>(
        ex: E,
        id: Uuid,
        camera: &CameraExif,
        captured_at: Option<NaiveDateTime>,
        gps_lat: Option<f64>,
        gps_lng: Option<f64>,
        gps_alt: Option<i32>,
        orientation: Option<i16>,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let exif_data = serde_json::to_value(camera)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        sqlx::query!(
            r#"UPDATE pictures
               SET exif_data   = $2,
                   captured_at = $3,
                   gps_lat     = $4,
                   gps_lng     = $5,
                   gps_alt     = $6,
                   orientation = $7,
                   last_pipeline_run_at = NULL
               WHERE id = $1"#,
            id,
            exif_data,
            captured_at,
            gps_lat,
            gps_lng,
            gps_alt,
            orientation,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Delete received-picture rows from `sender` for `recipient_id` that have no remaining
    /// `incoming_share` tags.
    ///
    /// Called after `TagRepository::remove_incoming_share_tags` during share revocation.
    ///
    /// A revoked picture is unreachable regardless of any local tags Bob may have added —
    /// the sender's presign endpoint will reject requests once the share token is invalid.
    /// Manual tags are therefore not a reason to keep the row.
    ///
    /// Pictures received from the same sender via a *different, still-active* share survive:
    /// they retain `incoming_share` tags from that other share, so the `NOT EXISTS` check
    /// excludes them.
    ///
    /// Returns the number of deleted rows.
    #[tracing::instrument(skip(ex), fields(user_id = %recipient_id))]
    pub async fn delete_received_without_share_tags<'e, E>(
        ex: E,
        recipient_id: Uuid,
        sender_username: &str,
        sender_instance: &str,
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let result = sqlx::query!(
            r#"DELETE FROM pictures
               WHERE local_user_id = $1
                 AND owner_username = $2
                 AND owner_instance_domain = $3
                 AND remote_picture_id IS NOT NULL
                 AND NOT EXISTS (
                     SELECT 1 FROM tags
                     WHERE tags.picture_id = pictures.id
                       AND tags.source = 'incoming_share'::tag_source
                 )"#,
            recipient_id,
            sender_username,
            sender_instance,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(result.rows_affected())
    }

    /// From `candidate_picture_ids`, return those that still carry at least one
    /// `incoming_share` source tag (i.e. survived a share's tag cleanup). Used by
    /// `cleanup_incoming_share` to mark survivors dirty for token refresh.
    #[tracing::instrument(skip(ex, candidate_picture_ids), fields(user_id = %recipient_id))]
    pub async fn find_with_any_incoming_share_tag<'e, E>(
        ex: E,
        recipient_id: Uuid,
        candidate_picture_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if candidate_picture_ids.is_empty() {
            return Ok(vec![]);
        }
        sqlx::query_scalar!(
            r#"SELECT DISTINCT p.id
               FROM pictures p
               JOIN tags t ON t.picture_id = p.id
               WHERE p.id = ANY($1::uuid[])
                 AND p.local_user_id = $2
                 AND t.source = 'incoming_share'::tag_source"#,
            candidate_picture_ids as &[Uuid],
            recipient_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Map a set of `remote_picture_id` strings to the recipient's local picture ids.
    /// Used by per-picture unannounce to resolve the sender's announce ids locally.
    #[tracing::instrument(skip(ex, remote_ids), fields(user_id = %recipient_id))]
    pub async fn find_ids_by_remote_ids<'e, E>(
        ex: E,
        recipient_id: Uuid,
        remote_ids: &[String],
    ) -> Result<Vec<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if remote_ids.is_empty() {
            return Ok(vec![]);
        }
        sqlx::query_scalar!(
            r#"SELECT id FROM pictures
               WHERE local_user_id = $1
                 AND remote_picture_id = ANY($2::text[])"#,
            recipient_id,
            remote_ids as &[String],
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Delete the received pictures in `picture_ids` that have no remaining `incoming_share`
    /// tag. Returns the deleted ids. Used by per-picture unannounce.
    #[tracing::instrument(skip(ex, picture_ids), fields(user_id = %recipient_id))]
    pub async fn delete_orphans_among<'e, E>(
        ex: E,
        recipient_id: Uuid,
        picture_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if picture_ids.is_empty() {
            return Ok(vec![]);
        }
        sqlx::query_scalar!(
            r#"DELETE FROM pictures
               WHERE id = ANY($1::uuid[])
                 AND local_user_id = $2
                 AND remote_picture_id IS NOT NULL
                 AND NOT EXISTS (
                     SELECT 1 FROM tags
                     WHERE tags.picture_id = pictures.id
                       AND tags.source = 'incoming_share'::tag_source
                 )
               RETURNING id"#,
            picture_ids as &[Uuid],
            recipient_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// List all active owned pictures that carry a tag under `tag_path_ltree` (inclusive).
    ///
    /// Used by Alice's backend to enumerate pictures to announce when a share is accepted.
    #[tracing::instrument(skip(ex), fields(owner_id = %owner_id))]
    pub async fn list_by_tag_and_owner<'e, E>(
        ex: E,
        owner_id: Uuid,
        tag_path_ltree: &str,
    ) -> Result<Vec<Picture>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            Picture,
            r#"SELECT DISTINCT p.id, p.local_user_id, p.remote_picture_id, p.owner_username,
                      p.owner_instance_domain, p.filename, p.mime_type, p.file_size,
                      p.width, p.height, p.exif_data as "exif_data: _", p.metadata as "metadata: _",
                      p.deleted_at, p.deleted_reason as "deleted_reason: _",
                      p.owner_deleted_at, p.owner_purge_at,
                      p.remote_exif_data as "remote_exif_data: _",
                      p.local_exif_overrides as "local_exif_overrides: _",
                      p.captured_at, p.ingested_at, p.updated_at, p.remote_updated_at,
                      p.blurhash, p.gps_lat, p.gps_lng, p.gps_alt, p.orientation,
                      p.thumbnails_generated_at, p.file_hash,
                      p.exif_sync_status as "exif_sync_status: _",
                      p.content_hash, p.copy_source_owner_username,
                      p.copy_source_owner_instance, p.copy_source_picture_id,
                      p.creator, p.creator_override, p.original_file_created_at
               FROM pictures p
               JOIN tags t ON t.picture_id = p.id
               WHERE p.local_user_id = $1
                 AND p.remote_picture_id IS NULL
                 AND p.deleted_at IS NULL
                 AND t.tag_path <@ $2::text::ltree"#,
            owner_id,
            tag_path_ltree,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Load a batch of picture rows by id (order unspecified). Used by the pipeline
    /// announcement step to build announcement payloads for the pictures it announces.
    #[tracing::instrument(skip(ex, ids))]
    pub async fn list_by_ids<'e, E>(ex: E, ids: &[Uuid]) -> Result<Vec<Picture>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        sqlx::query_as!(
            Picture,
            r#"SELECT id, local_user_id, remote_picture_id, owner_username, owner_instance_domain,
                      filename, mime_type, file_size, width, height,
                      exif_data as "exif_data: _", metadata as "metadata: _",
                      deleted_at, deleted_reason as "deleted_reason: _",
                      owner_deleted_at, owner_purge_at,
                      remote_exif_data as "remote_exif_data: _",
                      local_exif_overrides as "local_exif_overrides: _",
                      captured_at, ingested_at, updated_at, remote_updated_at,
                      blurhash, gps_lat, gps_lng, gps_alt, orientation, thumbnails_generated_at,
                      file_hash, exif_sync_status as "exif_sync_status: _",
                      content_hash, copy_source_owner_username,
                      copy_source_owner_instance, copy_source_picture_id,
                      creator, creator_override, original_file_created_at
               FROM pictures WHERE id = ANY($1::uuid[])"#,
            ids as &[Uuid],
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex), fields(picture_id = %id))]
    pub async fn find_by_id<'e, E>(ex: E, id: Uuid) -> Result<Option<Picture>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            Picture,
            r#"SELECT id, local_user_id, remote_picture_id, owner_username, owner_instance_domain,
                      filename, mime_type, file_size, width, height,
                      exif_data as "exif_data: _", metadata as "metadata: _",
                      deleted_at, deleted_reason as "deleted_reason: _",
                      owner_deleted_at, owner_purge_at,
                      remote_exif_data as "remote_exif_data: _",
                      local_exif_overrides as "local_exif_overrides: _",
                      captured_at, ingested_at, updated_at, remote_updated_at,
                      blurhash, gps_lat, gps_lng, gps_alt, orientation, thumbnails_generated_at,
                      file_hash, exif_sync_status as "exif_sync_status: _",
                      content_hash, copy_source_owner_username,
                      copy_source_owner_instance, copy_source_picture_id,
                      creator, creator_override, original_file_created_at
               FROM pictures WHERE id = $1"#,
            id
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Find an owned picture by its `file_hash` (the WebDAV ETag). Used by the WebDAV PUT
    /// path to recognise a relocate/copy expressed as a fresh upload and avoid creating a
    /// duplicate (06_webdav.md §8). `include_deleted` lets the caller also match a recently
    /// trashed picture (un-delete on rematch).
    #[tracing::instrument(skip(ex), fields(user_id = %user_id))]
    pub async fn find_owned_by_hash<'e, E>(
        ex: E,
        user_id: Uuid,
        file_hash: &str,
        include_deleted: bool,
    ) -> Result<Option<Picture>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            Picture,
            r#"SELECT id, local_user_id, remote_picture_id, owner_username, owner_instance_domain,
                      filename, mime_type, file_size, width, height,
                      exif_data as "exif_data: _", metadata as "metadata: _",
                      deleted_at, deleted_reason as "deleted_reason: _",
                      owner_deleted_at, owner_purge_at,
                      remote_exif_data as "remote_exif_data: _",
                      local_exif_overrides as "local_exif_overrides: _",
                      captured_at, ingested_at, updated_at, remote_updated_at,
                      blurhash, gps_lat, gps_lng, gps_alt, orientation, thumbnails_generated_at,
                      file_hash, exif_sync_status as "exif_sync_status: _",
                      content_hash, copy_source_owner_username,
                      copy_source_owner_instance, copy_source_picture_id,
                      creator, creator_override, original_file_created_at
               FROM pictures
               WHERE local_user_id = $1 AND file_hash = $2
                 AND remote_picture_id IS NULL
                 AND ($3 OR deleted_at IS NULL)
               ORDER BY deleted_at NULLS FIRST
               LIMIT 1"#,
            user_id,
            file_hash,
            include_deleted,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Set a picture's `file_hash` (and optionally `file_size`) inline after a WebDAV upload,
    /// before the thumbnail worker runs. This makes the ETag (`file_hash`) and dedupe
    /// (`find_owned_by_hash`) correct immediately, so a quick re-upload of the same bytes is
    /// recognised as a relocate rather than a fresh picture (06_webdav.md §8).
    #[tracing::instrument(skip(ex), fields(picture_id = %id))]
    pub async fn set_file_hash<'e, E>(
        ex: E,
        id: Uuid,
        file_hash: &str,
        file_size: Option<i64>,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"UPDATE pictures
               SET file_hash = $2,
                   file_size = COALESCE($3, file_size)
               WHERE id = $1"#,
            id,
            file_hash,
            file_size,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Rename an owned picture (WebDAV MOVE within a directory, §7.1). Returns false if the
    /// picture is not owned by the user.
    #[tracing::instrument(skip(ex), fields(user_id = %user_id, picture_id = %picture_id))]
    pub async fn set_filename<'e, E>(
        ex: E,
        user_id: Uuid,
        picture_id: Uuid,
        filename: &str,
    ) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        // Filename is a `metadata` event (filename rules) → re-dirty so the pipeline re-evaluates.
        let res = sqlx::query!(
            "UPDATE pictures SET filename = $3, last_pipeline_run_at = NULL \
             WHERE id = $1 AND local_user_id = $2",
            picture_id,
            user_id,
            filename,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected() > 0)
    }

    /// Set the owner-authoritative `creator` on an **owned** picture the user holds (feature 26 §7).
    /// `None` ⇒ reset to the owner default (`creator = NULL`). Bumps `updated_at` and re-dirties the
    /// pipeline so the change re-announces to recipients through the announcement-delta path. Returns
    /// false if the user holds no such owned picture.
    #[tracing::instrument(skip(ex), fields(user_id = %user_id, picture_id = %picture_id))]
    pub async fn set_creator<'e, E>(
        ex: E,
        user_id: Uuid,
        picture_id: Uuid,
        creator: Option<&str>,
    ) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let res = sqlx::query!(
            r#"UPDATE pictures
               SET creator = $3,
                   updated_at = (now() AT TIME ZONE 'utc'),
                   last_pipeline_run_at = NULL
               WHERE id = $1 AND local_user_id = $2 AND remote_picture_id IS NULL"#,
            picture_id,
            user_id,
            creator,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected() > 0)
    }

    /// Set the recipient-local `creator_override` on a **received** picture the user holds (feature 26
    /// §7). `None` ⇒ clear the override (reset to the origin's propagated creator). Never propagates.
    /// Returns false if the user holds no such received picture.
    #[tracing::instrument(skip(ex), fields(user_id = %user_id, picture_id = %picture_id))]
    pub async fn set_creator_override<'e, E>(
        ex: E,
        user_id: Uuid,
        picture_id: Uuid,
        creator_override: Option<&str>,
    ) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let res = sqlx::query!(
            r#"UPDATE pictures
               SET creator_override = $3
               WHERE id = $1 AND local_user_id = $2 AND remote_picture_id IS NOT NULL"#,
            picture_id,
            user_id,
            creator_override,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected() > 0)
    }

    /// Batch-set the creator over a selection (feature 26 batch integration) — one set-based UPDATE.
    /// `owned = true` targets the owner-authoritative `creator` on **owned** rows (bumps `updated_at`
    /// and re-dirties the pipeline so the change re-announces to recipients); `owned = false` targets
    /// the recipient-local `creator_override` on **received** rows (DB-only, never propagates).
    /// `value = None` resets to the owner default (owned) / clears the override (received). Returns
    /// rows changed.
    #[tracing::instrument(skip(ex, sel), fields(user_id = %local_user_id, owned))]
    pub async fn batch_set_creator_selection<'e, E>(
        ex: E,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
        value: Option<&str>,
        owned: bool,
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if sel.is_empty() {
            return Ok(0);
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new("UPDATE pictures AS p SET ");
        if owned {
            q.push("creator = ")
                .push_bind(value.map(str::to_string))
                .push(
                    ", updated_at = (now() AT TIME ZONE 'utc'), last_pipeline_run_at = NULL WHERE ",
                );
        } else {
            q.push("creator_override = ")
                .push_bind(value.map(str::to_string))
                .push(" WHERE ");
        }
        Self::push_selection_where(&mut q, local_user_id, sel);
        q.push(if owned {
            " AND p.remote_picture_id IS NULL"
        } else {
            " AND p.remote_picture_id IS NOT NULL"
        });
        let res = q.build().execute(ex).await.map_err(map_sqlx_error)?;
        Ok(res.rows_affected())
    }

    /// Set or clear `deleted_at` (+ `deleted_reason = 'manual'`) on a picture the user holds —
    /// owned or received (WebDAV `fullDelete` / un-delete on rematch, §7–8; the Trash API, 09 §5).
    /// Owned-picture trash keeps share coverage (the share-coverage query does not exclude
    /// `deleted_at`); received-picture trash is local only. Returns false if the user holds no such
    /// picture.
    #[tracing::instrument(skip(ex), fields(user_id = %user_id, picture_id = %picture_id))]
    pub async fn set_deleted<'e, E>(
        ex: E,
        user_id: Uuid,
        picture_id: Uuid,
        deleted: bool,
    ) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let res = sqlx::query!(
            r#"UPDATE pictures
               SET deleted_at = CASE WHEN $3 THEN (now() at time zone 'utc') ELSE NULL END,
                   deleted_reason = CASE WHEN $3 THEN 'manual'::picture_deleted_reason ELSE NULL END
               WHERE id = $1 AND local_user_id = $2"#,
            picture_id,
            user_id,
            deleted,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected() > 0)
    }

    /// Owned, soft-deleted pictures whose retention window has elapsed — the purge sweep's work set
    /// (09 §5.1). `owner_purge_at` is **derived** here as `deleted_at + retention_days` from the
    /// owner's `user_settings.trash_retention_days` (so a retention change takes effect with no
    /// backfill). Returns `(picture_id, local_user_id)`.
    #[tracing::instrument(skip(ex))]
    pub async fn find_purgeable<'e, E>(ex: E, limit: i64) -> Result<Vec<(Uuid, Uuid)>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows = sqlx::query!(
            r#"SELECT p.id, p.local_user_id
               FROM pictures p
               LEFT JOIN user_settings us ON us.user_id = p.local_user_id
               WHERE p.remote_picture_id IS NULL
                 AND p.deleted_at IS NOT NULL
                 AND p.deleted_at + make_interval(days => COALESCE(us.trash_retention_days, 30))
                     < (now() at time zone 'utc')
               ORDER BY p.deleted_at
               LIMIT $1"#,
            limit,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(rows.into_iter().map(|r| (r.id, r.local_user_id)).collect())
    }

    /// Hard-delete a picture row (purge). Tags cascade; the caller must have already removed the
    /// S3 objects and unannounced any downstream recipients (`share_announcements` has no FK to
    /// pictures, so its rows must be deleted explicitly first).
    #[tracing::instrument(skip(ex), fields(picture_id = %picture_id))]
    pub async fn hard_delete<'e, E>(ex: E, picture_id: Uuid) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!("DELETE FROM pictures WHERE id = $1", picture_id)
            .execute(ex)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }

    #[tracing::instrument(skip(db, filter), fields(user_id = %local_user_id))]
    pub async fn list(
        db: &PgPool,
        local_user_id: Uuid,
        filter: &PictureListFilter,
    ) -> Result<(Vec<Picture>, i64), AppError> {
        filter.validate()?;
        let sort_dir = match filter.order {
            SortOrder::Asc => "ASC",
            SortOrder::Desc => "DESC",
        };
        let offset = (filter.page - 1) * filter.page_size;

        let total: i64 = {
            let mut q = sqlx::QueryBuilder::<Postgres>::new(
                "SELECT COUNT(*) FROM pictures p WHERE p.local_user_id = ",
            );
            q.push_bind(local_user_id);
            Self::push_filters(&mut q, filter);
            q.build_query_scalar()
                .fetch_one(db)
                .await
                .map_err(map_sqlx_error)?
        };

        let items: Vec<Picture> = {
            let mut q = sqlx::QueryBuilder::<Postgres>::new(
                r#"SELECT p.id, p.local_user_id, p.remote_picture_id, p.owner_username,
                          p.owner_instance_domain, p.filename, p.mime_type, p.file_size,
                          p.width, p.height, p.exif_data, p.metadata,
                          p.deleted_at, p.deleted_reason, p.owner_deleted_at, p.owner_purge_at,
                          p.remote_exif_data, p.local_exif_overrides,
                          p.captured_at, p.ingested_at, p.updated_at, p.remote_updated_at,
                          p.blurhash, p.gps_lat, p.gps_lng, p.gps_alt, p.orientation,
                          p.thumbnails_generated_at, p.file_hash, p.exif_sync_status,
                          p.content_hash, p.copy_source_owner_username,
                          p.copy_source_owner_instance, p.copy_source_picture_id,
                          p.creator, p.creator_override, p.original_file_created_at
                   FROM pictures p WHERE p.local_user_id = "#,
            );
            q.push_bind(local_user_id);
            Self::push_filters(&mut q, filter);
            Self::push_order_by(&mut q, filter, sort_dir);
            q.push(" LIMIT ");
            q.push_bind(filter.page_size);
            q.push(" OFFSET ");
            q.push_bind(offset);
            q.build_query_as()
                .fetch_all(db)
                .await
                .map_err(map_sqlx_error)?
        };

        Ok((items, total))
    }

    /// Count pictures matching `filter` (no pagination). Used by the hierarchy `tree` endpoint's
    /// per-directory `picture_count` / empty-directory pruning.
    #[tracing::instrument(skip(db, filter), fields(user_id = %local_user_id))]
    pub async fn count(
        db: &PgPool,
        local_user_id: Uuid,
        filter: &PictureListFilter,
    ) -> Result<i64, AppError> {
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT COUNT(*) FROM pictures p WHERE p.local_user_id = ",
        );
        q.push_bind(local_user_id);
        Self::push_filters(&mut q, filter);
        q.build_query_scalar()
            .fetch_one(db)
            .await
            .map_err(map_sqlx_error)
    }

    /// Emit the `ORDER BY` clause. Column sorts use `NULLS LAST` + the `p.id` tiebreaker (total
    /// order ⇒ stable pagination). Proximity sorts (feature 29 §6) are always nearest-first,
    /// sort field-missing rows last, and ignore `SortOrder`; the reference params are bound.
    fn push_order_by(
        q: &mut sqlx::QueryBuilder<Postgres>,
        filter: &PictureListFilter,
        sort_dir: &str,
    ) {
        match filter.sort {
            PictureSortField::TimeNear => {
                // |captured_at − near_time|. Undated rows are already excluded (push_filters).
                q.push(" ORDER BY abs(extract(epoch FROM (p.captured_at - ");
                q.push_bind(filter.near_time);
                q.push("))) ASC, p.id ASC");
            }
            PictureSortField::GeoNear => {
                // Haversine central-angle term `a` (§6): monotonic with true great-circle distance,
                // so exact for a sort while skipping the final `asin`/`R` scaling; `sin²(Δlng/2)`
                // wraps the antimeridian correctly. Ungeotagged rows are excluded (push_filters).
                q.push(" ORDER BY sin(radians(p.gps_lat - ");
                q.push_bind(filter.near_lat);
                q.push(")/2)^2 + cos(radians(");
                q.push_bind(filter.near_lat);
                q.push(")) * cos(radians(p.gps_lat)) * sin(radians(p.gps_lng - ");
                q.push_bind(filter.near_lng);
                q.push(")/2)^2 ASC, p.id ASC");
            }
            _ => {
                let sort_col = match filter.sort {
                    PictureSortField::CapturedAt => "p.captured_at",
                    PictureSortField::IngestedAt => "p.ingested_at",
                    PictureSortField::UpdatedAt => "p.updated_at",
                    PictureSortField::FileSize => "p.file_size",
                    PictureSortField::Filename => "p.filename",
                    // Proximity variants handled above.
                    PictureSortField::TimeNear | PictureSortField::GeoNear => unreachable!(),
                };
                // Date-fix mode (feature 30 §4): undated rows first, then the column sort, then the
                // load-bearing `filename, id` tiebreak (undated rows have no captured_at to order by,
                // and run interpolation relies on a stable filename-contiguous order across pages).
                let missing_first = if filter.undated_first {
                    "(p.captured_at IS NULL) DESC, "
                } else {
                    ""
                };
                let tiebreak = if filter.undated_first {
                    format!(", p.filename {sort_dir}")
                } else {
                    String::new()
                };
                q.push(format!(
                    " ORDER BY {missing_first}{sort_col} {sort_dir} NULLS LAST{tiebreak}, p.id {sort_dir}"
                ));
            }
        }
    }

    fn push_filters(q: &mut sqlx::QueryBuilder<Postgres>, filter: &PictureListFilter) {
        // Content-dedup rows (`content_dedupe`/`boomerang`) are internal hidden state — they never
        // surface in gallery or trash listings, only via the per-picture copies endpoint. Any state
        // that admits trashed rows therefore shows `manual`-trashed only, so a rejected content group
        // shows exactly one recoverable entry rather than a pile of duplicates.
        match filter.trash {
            TrashFilter::Exclude => {
                q.push(" AND p.deleted_at IS NULL");
            }
            TrashFilter::Include => {
                q.push(" AND (p.deleted_at IS NULL OR p.deleted_reason = 'manual'::picture_deleted_reason)");
            }
            TrashFilter::Only => {
                q.push(" AND p.deleted_at IS NOT NULL AND p.deleted_reason = 'manual'::picture_deleted_reason");
            }
        }
        if filter.owned_only {
            q.push(" AND p.remote_picture_id IS NULL");
        }
        if filter.shared_with_me {
            q.push(" AND p.remote_picture_id IS NOT NULL");
        }
        if let Some(v) = filter.captured_after {
            q.push(" AND p.captured_at >= ").push_bind(v);
        }
        if let Some(v) = filter.captured_before {
            q.push(" AND p.captured_at <= ").push_bind(v);
        }
        // Presence filters (feature 29 §4). `missing_any` is the OR convenience; the per-field
        // arms are AND-composed (mutual exclusion enforced by `PictureListFilter::validate`).
        if filter.missing_any {
            q.push(" AND (p.captured_at IS NULL OR p.gps_lat IS NULL OR p.gps_lng IS NULL)");
        } else {
            match filter.gps {
                PresenceFilter::Any => {}
                PresenceFilter::Present => {
                    q.push(" AND p.gps_lat IS NOT NULL AND p.gps_lng IS NOT NULL");
                }
                PresenceFilter::Missing => {
                    q.push(" AND (p.gps_lat IS NULL OR p.gps_lng IS NULL)");
                }
            }
            match filter.capture_date {
                PresenceFilter::Any => {}
                PresenceFilter::Present => {
                    q.push(" AND p.captured_at IS NOT NULL");
                }
                PresenceFilter::Missing => {
                    q.push(" AND p.captured_at IS NULL");
                }
            }
        }
        // A proximity sort is meaningless for rows missing its field — exclude them entirely
        // (feature 29 §6) rather than trailing them at the end of the page.
        match filter.sort {
            PictureSortField::TimeNear => {
                q.push(" AND p.captured_at IS NOT NULL");
            }
            PictureSortField::GeoNear => {
                q.push(" AND p.gps_lat IS NOT NULL AND p.gps_lng IS NOT NULL");
            }
            _ => {}
        }
        if let Some(ref predicate) = filter.predicate {
            q.push(" AND ");
            Self::render_predicate(q, predicate);
        }
    }

    /// Render a [`TagPredicate`] to a SQL boolean over `pictures p`. Recursive: `minus_children`
    /// are negated sub-predicates ("most-specific node wins"). See `TagPredicate` docs for the
    /// membership semantics.
    fn render_predicate(q: &mut sqlx::QueryBuilder<Postgres>, pred: &TagPredicate) {
        q.push("(");
        if pred.untagged {
            q.push("NOT EXISTS (SELECT 1 FROM tags t WHERE t.picture_id = p.id)");
        } else if pred.include.is_empty() && pred.exact.is_empty() {
            // No positive arms ⇒ membership is vacuously true (all pictures).
            q.push("TRUE");
        } else {
            let joiner = if pred.match_all { " AND " } else { " OR " };
            q.push("(");
            let mut first = true;
            for inc in &pred.include {
                if !first {
                    q.push(joiner);
                }
                first = false;
                q.push("EXISTS (SELECT 1 FROM tags t WHERE t.picture_id = p.id AND t.tag_path <@ ")
                    .push_bind(inc.as_ltree().to_string())
                    .push("::ltree)");
            }
            for ex in &pred.exact {
                if !first {
                    q.push(joiner);
                }
                first = false;
                q.push("EXISTS (SELECT 1 FROM tags t WHERE t.picture_id = p.id AND t.tag_path = ")
                    .push_bind(ex.as_ltree().to_string())
                    .push("::ltree)");
            }
            q.push(")");
        }
        for ex in &pred.exclude {
            q.push(" AND NOT EXISTS (SELECT 1 FROM tags t WHERE t.picture_id = p.id AND t.tag_path <@ ")
                .push_bind(ex.as_ltree().to_string())
                .push("::ltree)");
        }
        for term in &pred.and_terms {
            q.push(" AND ");
            Self::render_predicate(q, term);
        }
        for child in &pred.minus_children {
            q.push(" AND NOT ");
            Self::render_predicate(q, child);
        }
        q.push(")");
    }

    #[tracing::instrument(skip(ex, exif_data), fields(picture_id = %id))]
    pub async fn update_metadata<'e, E>(
        ex: E,
        id: Uuid,
        mime_type: Option<&str>,
        file_size: Option<i64>,
        width: Option<i32>,
        height: Option<i32>,
        exif_data: Option<serde_json::Value>,
        captured_at: Option<NaiveDateTime>,
    ) -> Result<Picture, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            Picture,
            r#"UPDATE pictures
               SET mime_type = COALESCE($2, mime_type),
                   file_size = COALESCE($3, file_size),
                   width = COALESCE($4, width),
                   height = COALESCE($5, height),
                   exif_data = COALESCE($6, exif_data),
                   captured_at = COALESCE($7, captured_at)
               WHERE id = $1
               RETURNING id, local_user_id, remote_picture_id, owner_username, owner_instance_domain,
                         filename, mime_type, file_size, width, height,
                         exif_data as "exif_data: _", metadata as "metadata: _",
                         deleted_at, deleted_reason as "deleted_reason: _",
                         owner_deleted_at, owner_purge_at,
                         remote_exif_data as "remote_exif_data: _",
                         local_exif_overrides as "local_exif_overrides: _",
                         captured_at, ingested_at, updated_at, remote_updated_at,
                         blurhash, gps_lat, gps_lng, gps_alt, orientation, thumbnails_generated_at,
                         file_hash, exif_sync_status as "exif_sync_status: _",
                         content_hash, copy_source_owner_username,
                         copy_source_owner_instance, copy_source_picture_id,
                         creator, creator_override, original_file_created_at"#,
            id,
            mime_type,
            file_size,
            width,
            height,
            exif_data as Option<serde_json::Value>,
            captured_at,
        )
            .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Update a picture's worker-extracted data after initial thumbnail generation.
    /// Only updates fields that the worker provides (COALESCE keeps existing values).
    #[tracing::instrument(skip(ex, exif_data_patch), fields(picture_id = %id))]
    pub async fn update_from_worker<'e, E>(
        ex: E,
        id: Uuid,
        width: Option<i32>,
        height: Option<i32>,
        captured_at: Option<NaiveDateTime>,
        gps_lat: Option<f64>,
        gps_lng: Option<f64>,
        gps_alt: Option<i32>,
        orientation: Option<i16>,
        blurhash: Option<&str>,
        exif_data_patch: Option<serde_json::Value>,
        file_size: Option<i64>,
        file_hash: Option<&str>,
        content_hash: Option<&str>,
    ) -> Result<Picture, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            Picture,
            r#"UPDATE pictures
               SET width       = COALESCE($2,  width),
                   height      = COALESCE($3,  height),
                   captured_at = COALESCE($4,  captured_at),
                   gps_lat     = COALESCE($5,  gps_lat),
                   gps_lng     = COALESCE($6,  gps_lng),
                   gps_alt     = COALESCE($7,  gps_alt),
                   orientation = COALESCE($8,  orientation),
                   blurhash    = COALESCE($9,  blurhash),
                   exif_data   = CASE WHEN $10::jsonb IS NOT NULL
                                      THEN exif_data || $10::jsonb
                                      ELSE exif_data
                                 END,
                   file_size   = COALESCE($11, file_size),
                   file_hash   = COALESCE($12, file_hash),
                   content_hash = COALESCE($13, content_hash),
                   thumbnails_generated_at = COALESCE(thumbnails_generated_at, now() AT TIME ZONE 'utc'),
                   last_pipeline_run_at = NULL
               WHERE id = $1
               RETURNING id, local_user_id, remote_picture_id, owner_username, owner_instance_domain,
                         filename, mime_type, file_size, width, height,
                         exif_data as "exif_data: _", metadata as "metadata: _",
                         deleted_at, deleted_reason as "deleted_reason: _",
                         owner_deleted_at, owner_purge_at,
                         remote_exif_data as "remote_exif_data: _",
                         local_exif_overrides as "local_exif_overrides: _",
                         captured_at, ingested_at, updated_at, remote_updated_at,
                         blurhash, gps_lat, gps_lng, gps_alt, orientation, thumbnails_generated_at,
                         file_hash, exif_sync_status as "exif_sync_status: _",
                         content_hash, copy_source_owner_username,
                         copy_source_owner_instance, copy_source_picture_id,
                         creator, creator_override, original_file_created_at"#,
            id,
            width,
            height,
            captured_at,
            gps_lat,
            gps_lng,
            gps_alt,
            orientation,
            blurhash,
            exif_data_patch as Option<serde_json::Value>,
            file_size,
            file_hash,
            content_hash,
        )
            .fetch_one(ex)
            .await
            .map_err(map_sqlx_error)
    }

    /// Update picture metadata set by the worker after any job completes, for cases
    /// where no EXIF is returned (edit_picture, non-initial gen_thumbnail).
    ///
    /// `set_thumbnails` controls whether `thumbnails_generated_at` is stamped; the
    /// other fields are always applied via COALESCE (existing value kept when `None`).
    #[tracing::instrument(skip(ex), fields(picture_id = %id))]
    pub async fn update_after_processing<'e, E>(
        ex: E,
        id: Uuid,
        set_thumbnails: bool,
        blurhash: Option<&str>,
        file_size: Option<i64>,
        file_hash: Option<&str>,
        content_hash: Option<&str>,
        width: Option<i32>,
        height: Option<i32>,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"UPDATE pictures
               SET thumbnails_generated_at = CASE WHEN $2
                                                  THEN COALESCE(thumbnails_generated_at, now() AT TIME ZONE 'utc')
                                                  ELSE thumbnails_generated_at
                                             END,
                   blurhash  = COALESCE($3, blurhash),
                   file_size = COALESCE($4, file_size),
                   file_hash = COALESCE($5, file_hash),
                   content_hash = COALESCE($8, content_hash),
                   width     = COALESCE($6, width),
                   height    = COALESCE($7, height),
                   last_pipeline_run_at = NULL
               WHERE id = $1"#,
            id,
            set_thumbnails,
            blurhash,
            file_size,
            file_hash,
            width,
            height,
            content_hash,
        )
            .execute(ex)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Write a complete editable-EXIF snapshot onto the picture row (write-through model).
    ///
    /// Every editable field is set to its `snapshot` value (`None` → NULL / JSONB key removed),
    /// the camera/lens keys in `exif_data` are rebuilt (other JSONB keys preserved), `updated_at`
    /// is bumped, `last_pipeline_run_at` is reset (the edit re-dirties the picture so date/GPS
    /// rules re-evaluate), and `exif_sync_status` is set to `status`.
    ///
    /// Used for both the forward edit (snapshot = previous applied with set/clear) and a value-gated
    /// revert (snapshot = previous), so the row state always reflects a full, coherent EXIF set.
    #[tracing::instrument(skip(ex, snapshot), fields(picture_id = %id))]
    pub async fn write_exif_snapshot<'e, E>(
        ex: E,
        id: Uuid,
        snapshot: &FullExif,
        status: ExifSyncStatus,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        // The camera/lens keys to write back into `exif_data` (sparse: only `Some` fields appear).
        let patch = serde_json::to_value(&snapshot.camera)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        const CAMERA_KEYS: [&str; 7] = [
            "camera_brand",
            "camera_model",
            "focal_length_mm",
            "f_number",
            "iso_speed",
            "exposure_time_num",
            "exposure_time_den",
        ];
        let camera_keys: Vec<String> = CAMERA_KEYS.iter().map(|s| s.to_string()).collect();

        sqlx::query!(
            r#"UPDATE pictures
               SET captured_at = $2,
                   gps_lat     = $3,
                   gps_lng     = $4,
                   gps_alt     = $5,
                   orientation = $6,
                   exif_data   = (exif_data - $7::text[]) || $8::jsonb,
                   exif_sync_status     = $9,
                   updated_at           = (now() AT TIME ZONE 'utc'),
                   last_pipeline_run_at = NULL
               WHERE id = $1"#,
            id,
            snapshot.captured_at,
            snapshot.gps_lat,
            snapshot.gps_lng,
            snapshot.gps_alt,
            snapshot.orientation,
            &camera_keys as &[String],
            patch as serde_json::Value,
            status as ExifSyncStatus,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Set only the `exif_sync_status` column (e.g. flip to `synced` once a reconcile succeeds).
    #[tracing::instrument(skip(ex), fields(picture_id = %id))]
    pub async fn set_exif_sync_status<'e, E>(
        ex: E,
        id: Uuid,
        status: ExifSyncStatus,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            "UPDATE pictures SET exif_sync_status = $2 WHERE id = $1",
            id,
            status as ExifSyncStatus,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Owned pictures to (re)enqueue a `gen_thumbnail` job for (admin regen, feature 11 helper).
    ///
    /// - `only_missing = true` — pictures with a **thumbnailable** MIME that have no thumbnail yet
    ///   and are older than 30 minutes (the consistency-check "missing thumbnail" set: failed or
    ///   never-run jobs). Non-thumbnailable formats are excluded so they aren't re-enqueued forever.
    /// - `only_missing = false` — **all** owned pictures (e.g. to recompute `content_hash` across the
    ///   library), regardless of MIME.
    ///
    /// Pictures with an in-flight `gen_thumbnail` job are always excluded. Received pictures are never
    /// included (the backend does not hold their file). `thumbnailable_mimes` is the lower-cased
    /// whitelist. Returns `(picture_id, owner_id)`.
    #[tracing::instrument(skip(ex, thumbnailable_mimes))]
    pub async fn find_for_thumbnail_regen<'e, E>(
        ex: E,
        only_missing: bool,
        thumbnailable_mimes: &[String],
        limit: i64,
    ) -> Result<Vec<(Uuid, Uuid)>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows = sqlx::query!(
            r#"SELECT p.id, p.local_user_id
               FROM pictures p
               WHERE p.remote_picture_id IS NULL
                 AND (
                   NOT $1
                   OR (
                     p.thumbnails_generated_at IS NULL
                     AND p.ingested_at < (now() AT TIME ZONE 'utc') - interval '30 minutes'
                     AND p.mime_type IS NOT NULL
                     AND lower(p.mime_type) = ANY($2::text[])
                   )
                 )
                 AND NOT EXISTS (
                   SELECT 1 FROM jobs j
                   WHERE j.picture_id = p.id
                     AND j.job_type = 'gen_thumbnail'
                     AND j.status IN ('pending', 'processing')
                 )
               ORDER BY p.ingested_at
               LIMIT $3"#,
            only_missing,
            thumbnailable_mimes,
            limit,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(rows.into_iter().map(|r| (r.id, r.local_user_id)).collect())
    }

    /// Picture ids in `pending` EXIF sync that have no in-flight `edit_picture` job — the
    /// crash-mid-completion case the optional resync sweep / manual resync recovers.
    #[tracing::instrument(skip(ex))]
    pub async fn find_exif_pending_without_job<'e, E>(ex: E) -> Result<Vec<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_scalar!(
            r#"SELECT p.id
               FROM pictures p
               WHERE p.exif_sync_status = 'pending'
                 AND NOT EXISTS (
                     SELECT 1 FROM jobs j
                     WHERE j.picture_id = p.id
                       AND j.job_type = 'edit_picture'
                       AND j.status IN ('pending', 'processing')
                 )"#,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    // ── Selection (feature 14) ────────────────────────────────────────────────

    /// Push the selection membership predicate over alias `p` into `q`, scoped to `local_user_id`:
    /// `(query ∪ include_ids) \ exclude_ids`. Assumes `q` is positioned where a boolean is expected
    /// (e.g. right after `WHERE `). Reuses [`push_filters`](Self::push_filters) for the query branch
    /// so a selection filters identically to `GET /pictures`.
    pub fn push_selection_where(
        q: &mut sqlx::QueryBuilder<Postgres>,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
    ) {
        q.push("p.local_user_id = ").push_bind(local_user_id);
        match &sel.filter {
            Some(filter) => {
                q.push(" AND ((TRUE");
                Self::push_filters(q, filter);
                q.push(")");
                if !sel.include_ids.is_empty() {
                    q.push(" OR p.id = ANY(")
                        .push_bind(sel.include_ids.clone())
                        .push("::uuid[])");
                }
                q.push(")");
            }
            None => {
                q.push(" AND p.id = ANY(")
                    .push_bind(sel.include_ids.clone())
                    .push("::uuid[])");
            }
        }
        if !sel.exclude_ids.is_empty() {
            q.push(" AND NOT (p.id = ANY(")
                .push_bind(sel.exclude_ids.clone())
                .push("::uuid[]))");
        }
    }

    /// Count the pictures in the selection.
    #[tracing::instrument(skip(db, sel), fields(user_id = %local_user_id))]
    pub async fn count_selection(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
    ) -> Result<i64, AppError> {
        if sel.is_empty() {
            return Ok(0);
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new("SELECT COUNT(*) FROM pictures p WHERE ");
        Self::push_selection_where(&mut q, local_user_id, sel);
        q.build_query_scalar()
            .fetch_one(db)
            .await
            .map_err(map_sqlx_error)
    }

    /// Count owned pictures in the selection (used by the EXIF dry-run owner/local partition).
    #[tracing::instrument(skip(db, sel), fields(user_id = %local_user_id))]
    pub async fn count_owned_selection(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
    ) -> Result<i64, AppError> {
        if sel.is_empty() {
            return Ok(0);
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT COUNT(*) FROM pictures p WHERE p.remote_picture_id IS NULL AND ",
        );
        Self::push_selection_where(&mut q, local_user_id, sel);
        q.build_query_scalar()
            .fetch_one(db)
            .await
            .map_err(map_sqlx_error)
    }

    /// Materialise the picture ids in the selection (used by the tags batch, which applies via the
    /// existing array-based `batch_assign`/`batch_remove`). Resolve inside the caller's transaction.
    #[tracing::instrument(skip(ex, sel), fields(user_id = %local_user_id))]
    pub async fn resolve_selection_ids<'e, E>(
        ex: E,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
    ) -> Result<Vec<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if sel.is_empty() {
            return Ok(vec![]);
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new("SELECT p.id FROM pictures p WHERE ");
        Self::push_selection_where(&mut q, local_user_id, sel);
        q.build_query_scalar()
            .fetch_all(ex)
            .await
            .map_err(map_sqlx_error)
    }

    /// Materialise the **received** picture ids in the selection (used by the suggest-mode EXIF path,
    /// which proposes/overrides per picture).
    #[tracing::instrument(skip(ex, sel), fields(user_id = %local_user_id))]
    pub async fn resolve_selection_received_ids<'e, E>(
        ex: E,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
    ) -> Result<Vec<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if sel.is_empty() {
            return Ok(vec![]);
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT p.id FROM pictures p WHERE p.remote_picture_id IS NOT NULL AND ",
        );
        Self::push_selection_where(&mut q, local_user_id, sel);
        q.build_query_scalar()
            .fetch_all(ex)
            .await
            .map_err(map_sqlx_error)
    }

    /// Count owned pictures in the selection whose format cannot embed EXIF (the dry-run
    /// `unsupported` partition). `supported_mimes` is the lower-cased whitelist.
    #[tracing::instrument(skip(db, sel, supported_mimes), fields(user_id = %local_user_id))]
    pub async fn count_owned_unsupported_selection(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
        supported_mimes: &[String],
    ) -> Result<i64, AppError> {
        if sel.is_empty() {
            return Ok(0);
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT COUNT(*) FROM pictures p WHERE p.remote_picture_id IS NULL \
             AND (p.mime_type IS NULL OR NOT (lower(p.mime_type) = ANY(",
        );
        q.push_bind(supported_mimes.to_vec())
            .push("::text[]))) AND ");
        Self::push_selection_where(&mut q, local_user_id, sel);
        q.build_query_scalar()
            .fetch_one(db)
            .await
            .map_err(map_sqlx_error)
    }

    /// Count received pictures in the selection an active share grants EXIF-edit on (the dry-run
    /// `suggested` partition; feature 14 §6.1).
    #[tracing::instrument(skip(db, sel), fields(user_id = %local_user_id))]
    pub async fn count_selection_received_suggestable(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
    ) -> Result<i64, AppError> {
        if sel.is_empty() {
            return Ok(0);
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT COUNT(*) FROM pictures p WHERE p.remote_picture_id IS NOT NULL AND EXISTS ( \
               SELECT 1 FROM tags t JOIN incoming_shares ish ON ish.id = t.source_id \
               WHERE t.picture_id = p.id AND t.source = 'incoming_share'::tag_source \
                 AND ish.status = 'active'::share_status AND ish.allow_exif_edit = true) AND ",
        );
        Self::push_selection_where(&mut q, local_user_id, sel);
        q.build_query_scalar()
            .fetch_one(db)
            .await
            .map_err(map_sqlx_error)
    }

    /// Compute the [`SelectionSummary`] (feature 14 §4.1) — all from the `pictures` row (no joins).
    #[tracing::instrument(skip(db, sel), fields(user_id = %local_user_id))]
    pub async fn aggregate_summary(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
    ) -> Result<SelectionSummary, AppError> {
        if sel.is_empty() {
            return Ok(SelectionSummary::default());
        }

        // Scalar aggregates (one row).
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            r#"SELECT
                 COUNT(*)::bigint AS count,
                 COUNT(*) FILTER (WHERE p.remote_picture_id IS NULL)::bigint AS owned_count,
                 COUNT(*) FILTER (WHERE p.remote_picture_id IS NOT NULL)::bigint AS received_count,
                 COALESCE(SUM(p.file_size), 0)::bigint AS total_file_size,
                 COUNT(*) FILTER (WHERE p.deleted_at IS NOT NULL)::bigint AS trashed_count,
                 COUNT(*) FILTER (WHERE p.owner_deleted_at IS NOT NULL)::bigint AS owner_deleting_count,
                 COUNT(*) FILTER (WHERE p.thumbnails_generated_at IS NULL)::bigint AS thumbnail_pending_count
               FROM pictures p WHERE "#,
        );
        Self::push_selection_where(&mut q, local_user_id, sel);
        let row = q.build().fetch_one(db).await.map_err(map_sqlx_error)?;
        use sqlx::Row;
        let mut summary = SelectionSummary {
            count: row.try_get("count").map_err(map_sqlx_error)?,
            owned_count: row.try_get("owned_count").map_err(map_sqlx_error)?,
            received_count: row.try_get("received_count").map_err(map_sqlx_error)?,
            total_file_size: row.try_get("total_file_size").map_err(map_sqlx_error)?,
            trashed_count: row.try_get("trashed_count").map_err(map_sqlx_error)?,
            owner_deleting_count: row
                .try_get("owner_deleting_count")
                .map_err(map_sqlx_error)?,
            thumbnail_pending_count: row
                .try_get("thumbnail_pending_count")
                .map_err(map_sqlx_error)?,
            ..Default::default()
        };

        // Duplicate count: pictures sharing a file_hash with another in the selection.
        let mut dq = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT COALESCE(SUM(c), 0)::bigint FROM (SELECT COUNT(*) AS c FROM pictures p WHERE p.file_hash IS NOT NULL AND ",
        );
        Self::push_selection_where(&mut dq, local_user_id, sel);
        dq.push(" GROUP BY p.file_hash HAVING COUNT(*) > 1) g");
        summary.duplicate_count = dq
            .build_query_scalar()
            .fetch_one(db)
            .await
            .map_err(map_sqlx_error)?;

        // exif_sync histogram.
        let mut hq = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT p.exif_sync_status::text AS status, COUNT(*)::bigint AS cnt FROM pictures p WHERE ",
        );
        Self::push_selection_where(&mut hq, local_user_id, sel);
        hq.push(" GROUP BY p.exif_sync_status");
        let rows = hq.build().fetch_all(db).await.map_err(map_sqlx_error)?;
        for r in rows {
            let label: String = r.try_get("status").map_err(map_sqlx_error)?;
            let cnt: i64 = r.try_get("cnt").map_err(map_sqlx_error)?;
            if let Some(status) = parse_exif_sync_status(&label) {
                summary.exif_sync.push((status, cnt));
            }
        }

        // Distinct remote owners of received pictures.
        let mut oq = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT p.owner_username AS username, p.owner_instance_domain AS instance, COUNT(*)::bigint AS cnt \
             FROM pictures p WHERE p.remote_picture_id IS NOT NULL AND ",
        );
        Self::push_selection_where(&mut oq, local_user_id, sel);
        oq.push(" GROUP BY p.owner_username, p.owner_instance_domain ORDER BY cnt DESC");
        let rows = oq.build().fetch_all(db).await.map_err(map_sqlx_error)?;
        for r in rows {
            let username: Option<String> = r.try_get("username").map_err(map_sqlx_error)?;
            let instance: Option<String> = r.try_get("instance").map_err(map_sqlx_error)?;
            let count: i64 = r.try_get("cnt").map_err(map_sqlx_error)?;
            summary.owners.push(OwnerCount {
                username: username.unwrap_or_default(),
                instance: instance.unwrap_or_default(),
                count,
            });
        }

        Ok(summary)
    }

    /// Per-field numeric aggregates (min/max/avg/null_count) over the selection, one row per field.
    /// `fields` is `(field_name, SQL value expression)`; expressions are trusted constants (never
    /// user input).
    #[tracing::instrument(skip(db, sel, fields), fields(user_id = %local_user_id))]
    pub async fn aggregate_numeric(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
        fields: &[(&str, &str)],
    ) -> Result<Vec<(String, NumericAgg)>, AppError> {
        if sel.is_empty() || fields.is_empty() {
            return Ok(vec![]);
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new("");
        for (i, (name, expr)) in fields.iter().enumerate() {
            if i > 0 {
                q.push(" UNION ALL ");
            }
            q.push("SELECT '")
                .push(name)
                .push("' AS field, MIN(v)::float8 AS min_v, MAX(v)::float8 AS max_v, \
                       AVG(v)::float8 AS avg_v, (COUNT(*) FILTER (WHERE v IS NULL))::bigint AS null_count \
                       FROM (SELECT ")
                .push(expr)
                .push(" AS v FROM pictures p WHERE ");
            Self::push_selection_where(&mut q, local_user_id, sel);
            q.push(") s");
        }
        let rows = q.build().fetch_all(db).await.map_err(map_sqlx_error)?;
        use sqlx::Row;
        let mut out = Vec::new();
        for r in rows {
            out.push((
                r.try_get::<String, _>("field").map_err(map_sqlx_error)?,
                NumericAgg {
                    min: r.try_get("min_v").map_err(map_sqlx_error)?,
                    max: r.try_get("max_v").map_err(map_sqlx_error)?,
                    avg: r.try_get("avg_v").map_err(map_sqlx_error)?,
                    null_count: r.try_get("null_count").map_err(map_sqlx_error)?,
                },
            ));
        }
        Ok(out)
    }

    /// Per-field date aggregates (min/max range + avg instant + null_count), one row per field.
    #[tracing::instrument(skip(db, sel, fields), fields(user_id = %local_user_id))]
    pub async fn aggregate_dates(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
        fields: &[(&str, &str)],
    ) -> Result<Vec<(String, DateAgg)>, AppError> {
        if sel.is_empty() || fields.is_empty() {
            return Ok(vec![]);
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new("");
        for (i, (name, expr)) in fields.iter().enumerate() {
            if i > 0 {
                q.push(" UNION ALL ");
            }
            q.push("SELECT '")
                .push(name)
                .push(
                    "' AS field, MIN(v) AS min_v, MAX(v) AS max_v, \
                       (to_timestamp(AVG(EXTRACT(EPOCH FROM v))) AT TIME ZONE 'utc') AS avg_v, \
                       (COUNT(*) FILTER (WHERE v IS NULL))::bigint AS null_count \
                       FROM (SELECT ",
                )
                .push(expr)
                .push(" AS v FROM pictures p WHERE ");
            Self::push_selection_where(&mut q, local_user_id, sel);
            q.push(") s");
        }
        let rows = q.build().fetch_all(db).await.map_err(map_sqlx_error)?;
        use sqlx::Row;
        let mut out = Vec::new();
        for r in rows {
            out.push((
                r.try_get::<String, _>("field").map_err(map_sqlx_error)?,
                DateAgg {
                    min: r.try_get("min_v").map_err(map_sqlx_error)?,
                    max: r.try_get("max_v").map_err(map_sqlx_error)?,
                    avg: r.try_get("avg_v").map_err(map_sqlx_error)?,
                    null_count: r.try_get("null_count").map_err(map_sqlx_error)?,
                },
            ));
        }
        Ok(out)
    }

    /// GPS bounding box + centroid over the selection.
    #[tracing::instrument(skip(db, sel), fields(user_id = %local_user_id))]
    pub async fn aggregate_gps(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
    ) -> Result<GpsAgg, AppError> {
        if sel.is_empty() {
            return Ok(GpsAgg::default());
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            r#"SELECT MIN(p.gps_lat)::float8 AS lat_min, MAX(p.gps_lat)::float8 AS lat_max,
                      MIN(p.gps_lng)::float8 AS lng_min, MAX(p.gps_lng)::float8 AS lng_max,
                      AVG(p.gps_lat)::float8 AS clat, AVG(p.gps_lng)::float8 AS clng,
                      (COUNT(*) FILTER (WHERE p.gps_lat IS NULL OR p.gps_lng IS NULL))::bigint AS null_count
               FROM pictures p WHERE "#,
        );
        Self::push_selection_where(&mut q, local_user_id, sel);
        let r = q.build().fetch_one(db).await.map_err(map_sqlx_error)?;
        use sqlx::Row;
        Ok(GpsAgg {
            lat_min: r.try_get("lat_min").map_err(map_sqlx_error)?,
            lat_max: r.try_get("lat_max").map_err(map_sqlx_error)?,
            lng_min: r.try_get("lng_min").map_err(map_sqlx_error)?,
            lng_max: r.try_get("lng_max").map_err(map_sqlx_error)?,
            centroid_lat: r.try_get("clat").map_err(map_sqlx_error)?,
            centroid_lng: r.try_get("clng").map_err(map_sqlx_error)?,
            null_count: r.try_get("null_count").map_err(map_sqlx_error)?,
        })
    }

    /// Distinct-value histogram of a string/enum field (`expr` is a trusted SQL value expression).
    #[tracing::instrument(skip(db, sel), fields(user_id = %local_user_id))]
    pub async fn aggregate_distinct(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
        expr: &str,
    ) -> Result<DistinctAgg, AppError> {
        if sel.is_empty() {
            return Ok(DistinctAgg::default());
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT v AS value, COUNT(*)::bigint AS cnt FROM (SELECT ",
        );
        q.push(expr).push(" AS v FROM pictures p WHERE ");
        Self::push_selection_where(&mut q, local_user_id, sel);
        q.push(") s GROUP BY v ORDER BY cnt DESC");
        Self::fold_distinct(db, q).await
    }

    /// Distinct-value histogram of the **resolved displayed creator** (feature 26): `coalesce(
    /// creator_override, creator, owner_default)`, where the owner default is `owner_default_identity`
    /// for owned rows and the stored origin owner for received rows. `null_count` = unresolvable rows.
    #[tracing::instrument(skip(db, sel), fields(user_id = %local_user_id))]
    pub async fn aggregate_creator(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
        owner_default_identity: &str,
    ) -> Result<DistinctAgg, AppError> {
        if sel.is_empty() {
            return Ok(DistinctAgg::default());
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT v AS value, COUNT(*)::bigint AS cnt FROM (SELECT COALESCE(\
             NULLIF(p.creator_override, ''), NULLIF(p.creator, ''), \
             CASE WHEN p.remote_picture_id IS NULL THEN ",
        );
        q.push_bind(owner_default_identity.to_string());
        q.push(
            " WHEN COALESCE(p.owner_username, '') <> '' \
             THEN '@' || p.owner_username || ':' || COALESCE(p.owner_instance_domain, '') \
             ELSE NULL END) AS v FROM pictures p WHERE ",
        );
        Self::push_selection_where(&mut q, local_user_id, sel);
        q.push(") s GROUP BY v ORDER BY cnt DESC");
        Self::fold_distinct(db, q).await
    }

    /// Run a `(value, cnt)` distinct query built by the caller and fold it into a [`DistinctAgg`]
    /// (NULLs land in `null_count`). Shared by [`aggregate_distinct`] and [`aggregate_creator`].
    async fn fold_distinct(
        db: &PgPool,
        mut q: sqlx::QueryBuilder<Postgres>,
    ) -> Result<DistinctAgg, AppError> {
        let rows = q.build().fetch_all(db).await.map_err(map_sqlx_error)?;
        use sqlx::Row;
        let mut agg = DistinctAgg::default();
        for r in rows {
            let value: Option<String> = r.try_get("value").map_err(map_sqlx_error)?;
            let cnt: i64 = r.try_get("cnt").map_err(map_sqlx_error)?;
            match value {
                Some(v) => agg.values.push((v, cnt)),
                None => agg.null_count = cnt,
            }
        }
        Ok(agg)
    }

    /// Batch soft-delete / restore over a selection (feature 14 §6). One set-based UPDATE; resets
    /// `last_pipeline_run_at` so an owned picture re-announces its owner-deletion lifecycle. Returns
    /// the number of rows changed.
    #[tracing::instrument(skip(ex, sel), fields(user_id = %local_user_id, deleted))]
    pub async fn batch_set_trashed_selection<'e, E>(
        ex: E,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
        deleted: bool,
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if sel.is_empty() {
            return Ok(0);
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new("UPDATE pictures AS p SET deleted_at = ");
        if deleted {
            q.push("(now() AT TIME ZONE 'utc'), deleted_reason = 'manual'::picture_deleted_reason");
        } else {
            q.push("NULL, deleted_reason = NULL");
        }
        q.push(", last_pipeline_run_at = NULL WHERE ");
        Self::push_selection_where(&mut q, local_user_id, sel);
        let res = q.build().execute(ex).await.map_err(map_sqlx_error)?;
        Ok(res.rows_affected())
    }

    /// Apply a `set`/`clear` EXIF delta to the **owned** pictures in a selection set-based (feature
    /// 14 §5). Touches only owned, already-extracted pictures. Stamps `exif_sync_status` =
    /// `pending_job_creation` when the format embeds EXIF (the drain creates the reconcile job) or
    /// `unsupported` otherwise. `supported` selects which MIME partition this call targets;
    /// `supported_mimes` is the lower-cased whitelist. Returns rows changed.
    #[tracing::instrument(skip(ex, sel, set, clear, supported_mimes), fields(user_id = %local_user_id, supported))]
    pub async fn batch_apply_exif_owned_selection<'e, E>(
        ex: E,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
        set: &FullExif,
        clear: &[crate::domain::job::ExifField],
        supported: bool,
        supported_mimes: &[String],
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if sel.is_empty() {
            return Ok(0);
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new("UPDATE pictures AS p SET ");
        push_exif_column_assignments(&mut q, set, clear);
        q.push("exif_sync_status = ");
        if supported {
            q.push("'pending_job_creation'::picture_exif_sync_status");
        } else {
            q.push("'unsupported'::picture_exif_sync_status");
        }
        q.push(", updated_at = (now() AT TIME ZONE 'utc'), last_pipeline_run_at = NULL WHERE ");
        Self::push_selection_where(&mut q, local_user_id, sel);
        q.push(" AND p.remote_picture_id IS NULL AND p.thumbnails_generated_at IS NOT NULL AND ");
        if supported {
            q.push("lower(p.mime_type) = ANY(")
                .push_bind(supported_mimes.to_vec())
                .push("::text[])");
        } else {
            q.push("(p.mime_type IS NULL OR NOT (lower(p.mime_type) = ANY(")
                .push_bind(supported_mimes.to_vec())
                .push("::text[])))");
        }
        let res = q.build().execute(ex).await.map_err(map_sqlx_error)?;
        Ok(res.rows_affected())
    }

    /// Apply a recipient-local EXIF override delta to the **received** pictures in a selection,
    /// set-based (feature 14 §5). Merges `set`/`clear` into `local_exif_overrides` — dropping any set
    /// key already equal to the owner's value (09 §6.1) — then re-materialises `exif_data` + the
    /// promoted columns from `merge(remote_exif_data, overrides)` (override wins per field). DB-only;
    /// no file job. Returns rows changed.
    #[tracing::instrument(skip(ex, sel, set_patch, clear_keys), fields(user_id = %local_user_id))]
    pub async fn batch_apply_exif_received_local_selection<'e, E>(
        ex: E,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
        set_patch: &serde_json::Value,
        clear_keys: &[String],
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if sel.is_empty() {
            return Ok(0);
        }
        // new_ov = (overrides - clear_keys) || set_patch ; merged = remote || new_ov. The stored
        // override additionally drops any set key already equal to the owner's value (redundant — it
        // must not shadow a future owner edit, 09 §6.1); redundant keys don't change `merged`, so the
        // column assignments below keep the un-pruned `remote || new_ov`.
        const NEW_OV: &str =
            "((COALESCE(p.local_exif_overrides, '{}'::jsonb) - $CLEAR::text[]) || $PATCH::jsonb)";
        const MERGED: &str = "(COALESCE(p.remote_exif_data, '{}'::jsonb) || NEW_OV)";
        // Build with real binds; we repeat the bind references, so push them via QueryBuilder.
        let mut q =
            sqlx::QueryBuilder::<Postgres>::new("UPDATE pictures AS p SET local_exif_overrides = ");
        push_pruned_new_ov(&mut q, set_patch, clear_keys);
        q.push(", exif_data = (");
        push_merged(&mut q, set_patch, clear_keys);
        q.push(" - ARRAY['captured_at','gps_lat','gps_lng','gps_alt','orientation']::text[])");
        q.push(", captured_at = ((");
        push_merged(&mut q, set_patch, clear_keys);
        q.push(")->>'captured_at')::timestamp");
        q.push(", gps_lat = ((");
        push_merged(&mut q, set_patch, clear_keys);
        q.push(")->>'gps_lat')::float8");
        q.push(", gps_lng = ((");
        push_merged(&mut q, set_patch, clear_keys);
        q.push(")->>'gps_lng')::float8");
        q.push(", gps_alt = ((");
        push_merged(&mut q, set_patch, clear_keys);
        q.push(")->>'gps_alt')::int");
        q.push(", orientation = ((");
        push_merged(&mut q, set_patch, clear_keys);
        q.push(")->>'orientation')::smallint");
        q.push(", last_pipeline_run_at = NULL WHERE ");
        Self::push_selection_where(&mut q, local_user_id, sel);
        q.push(" AND p.remote_picture_id IS NOT NULL");
        let _ = (NEW_OV, MERGED); // documentation constants
        let res = q.build().execute(ex).await.map_err(map_sqlx_error)?;
        Ok(res.rows_affected())
    }

    /// Up to `limit` picture ids stamped `pending_job_creation` with no in-flight `edit_picture`
    /// job — the deferred-EXIF-job drain's work set (feature 14 §5). Returns `(picture_id, owner)`.
    #[tracing::instrument(skip(ex))]
    pub async fn find_pending_job_creation<'e, E>(
        ex: E,
        limit: i64,
    ) -> Result<Vec<(Uuid, Uuid)>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows = sqlx::query!(
            r#"SELECT p.id, p.local_user_id
               FROM pictures p
               WHERE p.exif_sync_status = 'pending_job_creation'
                 AND NOT EXISTS (
                     SELECT 1 FROM jobs j
                     WHERE j.picture_id = p.id
                       AND j.job_type = 'edit_picture'
                       AND j.status IN ('pending', 'processing')
                 )
               ORDER BY p.updated_at
               LIMIT $1"#,
            limit,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(rows.into_iter().map(|r| (r.id, r.local_user_id)).collect())
    }
}

/// Parse an `picture_exif_sync_status` text label into the enum (drives the summary histogram).
fn parse_exif_sync_status(label: &str) -> Option<ExifSyncStatus> {
    match label {
        "synced" => Some(ExifSyncStatus::Synced),
        "pending" => Some(ExifSyncStatus::Pending),
        "unsupported" => Some(ExifSyncStatus::Unsupported),
        "pending_job_creation" => Some(ExifSyncStatus::PendingJobCreation),
        _ => None,
    }
}

/// Push the `new_ov` JSONB expression `(overrides - clear) || patch` with fresh binds.
fn push_new_ov(
    q: &mut sqlx::QueryBuilder<Postgres>,
    set_patch: &serde_json::Value,
    clear_keys: &[String],
) {
    q.push("((COALESCE(p.local_exif_overrides, '{}'::jsonb) - ")
        .push_bind(clear_keys.to_vec())
        .push("::text[]) || ")
        .push_bind(set_patch.clone())
        .push("::jsonb)");
}

/// Push `new_ov` with the redundant `set` keys dropped: a set key whose value already equals the
/// owner's `remote_exif_data` value is not stored as an override (it would needlessly shadow a future
/// owner edit — 09 §6.1). Pre-existing overrides on untouched fields are left intact.
fn push_pruned_new_ov(
    q: &mut sqlx::QueryBuilder<Postgres>,
    set_patch: &serde_json::Value,
    clear_keys: &[String],
) {
    q.push("(");
    push_new_ov(q, set_patch, clear_keys);
    q.push(" - ARRAY(SELECT e.k FROM jsonb_each(")
        .push_bind(set_patch.clone())
        .push("::jsonb) AS e(k, v) WHERE COALESCE(p.remote_exif_data -> e.k, 'null'::jsonb) IS NOT DISTINCT FROM e.v))");
}

/// Push the `merged` JSONB expression `remote || new_ov` with fresh binds.
fn push_merged(
    q: &mut sqlx::QueryBuilder<Postgres>,
    set_patch: &serde_json::Value,
    clear_keys: &[String],
) {
    q.push("(COALESCE(p.remote_exif_data, '{}'::jsonb) || ");
    push_new_ov(q, set_patch, clear_keys);
    q.push(")");
}

/// The seven camera/lens JSONB keys held in `exif_data`.
const CAMERA_KEYS: [&str; 7] = [
    "camera_brand",
    "camera_model",
    "focal_length_mm",
    "f_number",
    "iso_speed",
    "exposure_time_num",
    "exposure_time_den",
];

/// Push the promoted-column + `exif_data` assignments for an owned-picture EXIF `set`/`clear`,
/// trailing each with `, ` (the caller appends `exif_sync_status = …`). Only touched fields appear.
fn push_exif_column_assignments(
    q: &mut sqlx::QueryBuilder<Postgres>,
    set: &FullExif,
    clear: &[crate::domain::job::ExifField],
) {
    use crate::domain::job::ExifField;
    let cleared = |f: ExifField| clear.contains(&f);

    // Promoted columns.
    if set.captured_at.is_some() {
        q.push("captured_at = ")
            .push_bind(set.captured_at)
            .push(", ");
    } else if cleared(ExifField::CapturedAt) {
        q.push("captured_at = NULL, ");
    }
    if set.gps_lat.is_some() {
        q.push("gps_lat = ").push_bind(set.gps_lat).push(", ");
    } else if cleared(ExifField::GpsLat) {
        q.push("gps_lat = NULL, ");
    }
    if set.gps_lng.is_some() {
        q.push("gps_lng = ").push_bind(set.gps_lng).push(", ");
    } else if cleared(ExifField::GpsLng) {
        q.push("gps_lng = NULL, ");
    }
    if set.gps_alt.is_some() {
        q.push("gps_alt = ").push_bind(set.gps_alt).push(", ");
    } else if cleared(ExifField::GpsAlt) {
        q.push("gps_alt = NULL, ");
    }
    if set.orientation.is_some() {
        q.push("orientation = ")
            .push_bind(set.orientation)
            .push(", ");
    } else if cleared(ExifField::Orientation) {
        q.push("orientation = NULL, ");
    }

    // Camera/lens JSONB: drop cleared keys, merge the set patch.
    let clear_camera: Vec<String> = CAMERA_KEYS
        .iter()
        .filter(|k| camera_key_cleared(k, clear))
        .map(|k| k.to_string())
        .collect();
    let patch = serde_json::to_value(&set.camera).unwrap_or_else(|_| serde_json::json!({}));
    let patch_empty = patch.as_object().map(|o| o.is_empty()).unwrap_or(true);
    if !clear_camera.is_empty() || !patch_empty {
        q.push("exif_data = (exif_data - ")
            .push_bind(clear_camera)
            .push("::text[]) || ")
            .push_bind(patch)
            .push("::jsonb, ");
    }
}

/// Whether a camera JSONB key is in the `clear` list.
fn camera_key_cleared(key: &str, clear: &[crate::domain::job::ExifField]) -> bool {
    use crate::domain::job::ExifField::*;
    let f = match key {
        "camera_brand" => CameraBrand,
        "camera_model" => CameraModel,
        "focal_length_mm" => FocalLengthMm,
        "f_number" => FNumber,
        "iso_speed" => IsoSpeed,
        "exposure_time_num" => ExposureTimeNum,
        "exposure_time_den" => ExposureTimeDen,
        _ => return false,
    };
    clear.contains(&f)
}
