use crate::domain::hierarchy::TagPredicate;
use crate::domain::job::{CameraExif, FullExif};
use crate::domain::picture::{ExifSyncStatus, Picture};
use crate::infra::error::{AppError, map_sqlx_error};
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
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    Asc,
    #[default]
    Desc,
}

#[derive(Debug, Clone)]
pub struct PictureListFilter {
    pub page: i64,
    pub page_size: i64,
    pub sort: PictureSortField,
    pub order: SortOrder,
    pub tag: Option<String>,
    /// Generalised tag-set predicate (hierarchy `browse`, public `include_tags`/`exclude_tags`/
    /// `match`/`untagged`). Rendered in addition to `tag` (callers set at most one).
    pub predicate: Option<TagPredicate>,
    pub owned_only: bool,
    pub shared_with_me: bool,
    pub include_deleted: bool,
    pub captured_after: Option<NaiveDateTime>,
    pub captured_before: Option<NaiveDateTime>,
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
    ) -> Result<Picture, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let exif_json = exif_data.unwrap_or_else(|| serde_json::json!({}));
        sqlx::query_as!(
            Picture,
            r#"INSERT INTO pictures (id, local_user_id, filename, mime_type, file_size, width, height, exif_data, metadata, captured_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, '{}'::jsonb, $9)
               RETURNING id, local_user_id, remote_picture_id, owner_username, owner_instance_domain,
                         filename, mime_type, file_size, width, height,
                         exif_data as "exif_data: _", metadata as "metadata: _",
                         deleted_at, deleted_reason as "deleted_reason: _",
                         owner_deleted_at, owner_purge_at,
                         remote_exif_data as "remote_exif_data: _",
                         local_exif_overrides as "local_exif_overrides: _",
                         captured_at, ingested_at, updated_at,
                         blurhash, gps_lat, gps_lng, gps_alt, orientation, thumbnails_generated_at,
                         file_hash, exif_sync_status as "exif_sync_status: _""#,
            id,
            local_user_id,
            filename,
            mime_type,
            file_size,
            width,
            height,
            serde_json::Value::from(exif_json) as serde_json::Value,
            captured_at,
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
        thumbnails_generated_at: Option<NaiveDateTime>,
        remote_exif_data: &FullExif,
        owner_deleted_at: Option<NaiveDateTime>,
        owner_purge_at: Option<NaiveDateTime>,
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
                    blurhash, file_hash, thumbnails_generated_at,
                    remote_exif_data, owner_deleted_at, owner_purge_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, '{}'::jsonb,
                       $10, $11, $12, $13, $14, $15)
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
                   thumbnails_generated_at = COALESCE(EXCLUDED.thumbnails_generated_at,
                                                      pictures.thumbnails_generated_at),
                   -- Owner-authoritative state is refreshed; local_exif_overrides is preserved.
                   remote_exif_data = EXCLUDED.remote_exif_data,
                   owner_deleted_at = EXCLUDED.owner_deleted_at,
                   owner_purge_at   = EXCLUDED.owner_purge_at
               RETURNING id, local_user_id, remote_picture_id, owner_username, owner_instance_domain,
                         filename, mime_type, file_size, width, height,
                         exif_data as "exif_data: _", metadata as "metadata: _",
                         deleted_at, deleted_reason as "deleted_reason: _",
                         owner_deleted_at, owner_purge_at,
                         remote_exif_data as "remote_exif_data: _",
                         local_exif_overrides as "local_exif_overrides: _",
                         captured_at, ingested_at, updated_at,
                         blurhash, gps_lat, gps_lng, gps_alt, orientation, thumbnails_generated_at,
                         file_hash, exif_sync_status as "exif_sync_status: _""#,
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
        )
            .fetch_one(ex)
            .await
            .map_err(map_sqlx_error)
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

    /// Replace a received row's `local_exif_overrides` (the recipient's sticky per-field set) — used
    /// by the local-override endpoint (09 §6.2). The caller re-materialises afterwards. `None` /
    /// an empty object both clear all overrides. Returns the row's `remote_exif_data` so the caller
    /// can recompute the merge without an extra read.
    #[tracing::instrument(skip(ex, overrides), fields(user_id = %user_id, picture_id = %picture_id))]
    pub async fn set_local_exif_overrides<'e, E>(
        ex: E,
        user_id: Uuid,
        picture_id: Uuid,
        overrides: &FullExif,
    ) -> Result<Option<FullExif>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let overrides_json = serde_json::to_value(overrides)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        let row = sqlx::query!(
            r#"UPDATE pictures
               SET local_exif_overrides = $3
               WHERE id = $1 AND local_user_id = $2 AND remote_picture_id IS NOT NULL
               RETURNING remote_exif_data"#,
            picture_id,
            user_id,
            overrides_json,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(row.map(|r| {
            r.remote_exif_data
                .and_then(|v| serde_json::from_value::<FullExif>(v).ok())
                .unwrap_or_default()
        }))
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
                      p.captured_at, p.ingested_at, p.updated_at,
                      p.blurhash, p.gps_lat, p.gps_lng, p.gps_alt, p.orientation,
                      p.thumbnails_generated_at, p.file_hash,
                      p.exif_sync_status as "exif_sync_status: _"
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
                      captured_at, ingested_at, updated_at,
                      blurhash, gps_lat, gps_lng, gps_alt, orientation, thumbnails_generated_at,
                      file_hash, exif_sync_status as "exif_sync_status: _"
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
                      captured_at, ingested_at, updated_at,
                      blurhash, gps_lat, gps_lng, gps_alt, orientation, thumbnails_generated_at,
                      file_hash, exif_sync_status as "exif_sync_status: _"
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
                      captured_at, ingested_at, updated_at,
                      blurhash, gps_lat, gps_lng, gps_alt, orientation, thumbnails_generated_at,
                      file_hash, exif_sync_status as "exif_sync_status: _"
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
        let res = sqlx::query!(
            "UPDATE pictures SET filename = $3 WHERE id = $1 AND local_user_id = $2",
            picture_id,
            user_id,
            filename,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected() > 0)
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
        let sort_col = match filter.sort {
            PictureSortField::CapturedAt => "p.captured_at",
            PictureSortField::IngestedAt => "p.ingested_at",
            PictureSortField::UpdatedAt => "p.updated_at",
        };
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
                          p.captured_at, p.ingested_at, p.updated_at,
                          p.blurhash, p.gps_lat, p.gps_lng, p.gps_alt, p.orientation,
                          p.thumbnails_generated_at, p.file_hash, p.exif_sync_status
                   FROM pictures p WHERE p.local_user_id = "#,
            );
            q.push_bind(local_user_id);
            Self::push_filters(&mut q, filter);
            q.push(format!(" ORDER BY {} {} LIMIT ", sort_col, sort_dir));
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

    fn push_filters(q: &mut sqlx::QueryBuilder<Postgres>, filter: &PictureListFilter) {
        if !filter.include_deleted {
            q.push(" AND p.deleted_at IS NULL");
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
        if let Some(ref tag) = filter.tag {
            q.push(
                " AND EXISTS (SELECT 1 FROM tags t WHERE t.picture_id = p.id AND t.tag_path <@ ",
            )
            .push_bind(tag.clone())
            .push("::ltree)");
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
                         captured_at, ingested_at, updated_at,
                         blurhash, gps_lat, gps_lng, gps_alt, orientation, thumbnails_generated_at,
                         file_hash, exif_sync_status as "exif_sync_status: _""#,
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
                   thumbnails_generated_at = COALESCE(thumbnails_generated_at, now() AT TIME ZONE 'utc')
               WHERE id = $1
               RETURNING id, local_user_id, remote_picture_id, owner_username, owner_instance_domain,
                         filename, mime_type, file_size, width, height,
                         exif_data as "exif_data: _", metadata as "metadata: _",
                         deleted_at, deleted_reason as "deleted_reason: _",
                         owner_deleted_at, owner_purge_at,
                         remote_exif_data as "remote_exif_data: _",
                         local_exif_overrides as "local_exif_overrides: _",
                         captured_at, ingested_at, updated_at,
                         blurhash, gps_lat, gps_lng, gps_alt, orientation, thumbnails_generated_at,
                         file_hash, exif_sync_status as "exif_sync_status: _""#,
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
                   width     = COALESCE($6, width),
                   height    = COALESCE($7, height)
               WHERE id = $1"#,
            id,
            set_thumbnails,
            blurhash,
            file_size,
            file_hash,
            width,
            height,
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
}
