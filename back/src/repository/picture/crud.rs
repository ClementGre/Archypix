use crate::domain::job::FullExif;
use crate::domain::picture::{ExifSyncStatus, Picture};
use archypix_common::error::{AppError, map_sqlx_error};
use chrono::NaiveDateTime;
use sqlx::{Executor, Postgres};
use uuid::Uuid;
use super::*;

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
        exif_sync_status: ExifSyncStatus,
    ) -> Result<Picture, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let exif_json = exif_data.unwrap_or_else(|| serde_json::json!({}));
        sqlx::query_as!(
            Picture,
            r#"INSERT INTO pictures (id, local_user_id, filename, mime_type, file_size, width, height, exif_data, metadata, captured_at, original_file_created_at, exif_sync_status)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, '{}'::jsonb, $9, $10, $11)
               RETURNING id, local_user_id, remote_picture_id, owner_username, owner_instance_domain,
                         filename, mime_type, file_size, width, height,
                         exif_data as "exif_data: _", metadata as "metadata: _",
                         deleted_at, deleted_reason as "deleted_reason: _",
                         owner_deleted_at, owner_purge_at,
                         remote_exif_data as "remote_exif_data: _",
                         local_exif_overrides as "local_exif_overrides: _",
                         captured_at, ingested_at, updated_at, remote_updated_at,
                         blurhash, gps_lat, gps_lng, gps_alt, orientation, thumbnails_generated_at,
                         file_hash, exif_sync_status as "exif_sync_status: _", file_exif as "file_exif: _",
                         content_hash, copy_source_owner_username,
                         copy_source_owner_instance, copy_source_picture_id,
                         creator, creator_override, original_file_created_at, file_modified_at"#,
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
            exif_sync_status as ExifSyncStatus,
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
        exif_sync_status: ExifSyncStatus,
        file_exif: Option<serde_json::Value>,
    ) -> Result<Picture, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            Picture,
            r#"INSERT INTO pictures (id, local_user_id, filename, mime_type, file_size, width, height,
                                     exif_data, metadata, captured_at, gps_lat, gps_lng, gps_alt, orientation,
                                     copy_source_owner_username, copy_source_owner_instance, copy_source_picture_id,
                                     creator, exif_sync_status, file_exif)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, '{}'::jsonb, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19::jsonb)
               RETURNING id, local_user_id, remote_picture_id, owner_username, owner_instance_domain,
                         filename, mime_type, file_size, width, height,
                         exif_data as "exif_data: _", metadata as "metadata: _",
                         deleted_at, deleted_reason as "deleted_reason: _",
                         owner_deleted_at, owner_purge_at,
                         remote_exif_data as "remote_exif_data: _",
                         local_exif_overrides as "local_exif_overrides: _",
                         captured_at, ingested_at, updated_at, remote_updated_at,
                         blurhash, gps_lat, gps_lng, gps_alt, orientation, thumbnails_generated_at,
                         file_hash, exif_sync_status as "exif_sync_status: _", file_exif as "file_exif: _",
                         content_hash, copy_source_owner_username,
                         copy_source_owner_instance, copy_source_picture_id,
                         creator, creator_override, original_file_created_at, file_modified_at"#,
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
            exif_sync_status as ExifSyncStatus,
            file_exif,
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
                    remote_exif_data, owner_deleted_at, owner_purge_at, creator, remote_updated_at,
                    exif_sync_status)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, '{}'::jsonb,
                       $10, $11, $16, $12, $13, $14, $15, $17, $18,
                       -- Inert for a received row: no local original, never reaches the write path.
                       'synced'::picture_exif_sync_status)
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
                         file_hash, exif_sync_status as "exif_sync_status: _", file_exif as "file_exif: _",
                         content_hash, copy_source_owner_username,
                         copy_source_owner_instance, copy_source_picture_id,
                         creator, creator_override, original_file_created_at, file_modified_at"#,
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
                      p.exif_sync_status as "exif_sync_status: _", p.file_exif as "file_exif: _",
                      p.content_hash, p.copy_source_owner_username,
                      p.copy_source_owner_instance, p.copy_source_picture_id,
                      p.creator, p.creator_override, p.original_file_created_at, p.file_modified_at
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
                      file_hash, exif_sync_status as "exif_sync_status: _", file_exif as "file_exif: _",
                      content_hash, copy_source_owner_username,
                      copy_source_owner_instance, copy_source_picture_id,
                      creator, creator_override, original_file_created_at, file_modified_at
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
                      file_hash, exif_sync_status as "exif_sync_status: _", file_exif as "file_exif: _",
                      content_hash, copy_source_owner_username,
                      copy_source_owner_instance, copy_source_picture_id,
                      creator, creator_override, original_file_created_at, file_modified_at
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
                      file_hash, exif_sync_status as "exif_sync_status: _", file_exif as "file_exif: _",
                      content_hash, copy_source_owner_username,
                      copy_source_owner_instance, copy_source_picture_id,
                      creator, creator_override, original_file_created_at, file_modified_at
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

}
