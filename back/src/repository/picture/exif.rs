use crate::domain::job::FullExif;
use crate::domain::picture::{ExifSyncStatus, Picture};
use archypix_common::error::{AppError, map_sqlx_error};
use chrono::NaiveDateTime;
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;
use super::*;

impl PictureRepository {
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
                         file_hash, exif_sync_status as "exif_sync_status: _", file_exif as "file_exif: _",
                         content_hash, copy_source_owner_username,
                         copy_source_owner_instance, copy_source_picture_id,
                         creator, creator_override, original_file_created_at, file_modified_at"#,
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

    /// Apply a worker-extracted EXIF snapshot as the authoritative physical-file state.
    ///
    /// Used by `gen_thumbnail` extraction (initial ingest and external overwrites, feature 31 §5):
    /// the promoted columns and the camera keys of `exif_data` are set to the file's values,
    /// `file_exif` records the same snapshot, and the row becomes `synced` — unless it is
    /// `unsupported`, a terminal state extraction must not undo.
    #[tracing::instrument(skip(ex, extracted), fields(picture_id = %id))]
    pub async fn update_from_worker<'e, E>(
        ex: E,
        id: Uuid,
        extracted: &FullExif,
        width: Option<i32>,
        height: Option<i32>,
        blurhash: Option<&str>,
        file_size: Option<i64>,
        file_hash: Option<&str>,
        content_hash: Option<&str>,
        set_thumbnails: bool,
    ) -> Result<Picture, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let file_exif_json = serde_json::to_value(extracted)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        let camera_patch = serde_json::to_value(&extracted.camera)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        let camera_keys: Vec<String> = CAMERA_KEYS.iter().map(|s| s.to_string()).collect();
        sqlx::query_as!(
            Picture,
            r#"UPDATE pictures
               SET width       = COALESCE($2,  width),
                   height      = COALESCE($3,  height),
                   captured_at = $4,
                   gps_lat     = $5,
                   gps_lng     = $6,
                   gps_alt     = $7,
                   orientation = $8,
                   blurhash    = COALESCE($9,  blurhash),
                   exif_data   = (exif_data - $10::text[]) || $11::jsonb,
                   file_size   = COALESCE($12, file_size),
                   file_hash   = COALESCE($13, file_hash),
                   content_hash = COALESCE($14, content_hash),
                   file_exif   = $15::jsonb,
                   -- A successful extraction is direct evidence against any stale verdict (§6.3).
                   exif_sync_status = 'synced'::picture_exif_sync_status,
                   thumbnails_generated_at = CASE WHEN $16
                                                  THEN COALESCE(thumbnails_generated_at, now() AT TIME ZONE 'utc')
                                                  ELSE thumbnails_generated_at
                                             END,
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
                         file_hash, exif_sync_status as "exif_sync_status: _", file_exif as "file_exif: _",
                         content_hash, copy_source_owner_username,
                         copy_source_owner_instance, copy_source_picture_id,
                         creator, creator_override, original_file_created_at, file_modified_at"#,
            id,
            width,
            height,
            extracted.captured_at,
            extracted.gps_lat,
            extracted.gps_lng,
            extracted.gps_alt,
            extracted.orientation,
            blurhash,
            &camera_keys as &[String],
            camera_patch,
            file_size,
            file_hash,
            content_hash,
            file_exif_json,
            set_thumbnails,
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

    /// Set `exif_sync_status` on a batch of pictures, optionally only where it currently equals
    /// `from` (a compare-and-set, so a state that already moved on is not stomped). Returns rows
    /// changed.
    #[tracing::instrument(skip(ex, ids), fields(count = ids.len()))]
    pub async fn set_exif_sync_status_bulk<'e, E>(
        ex: E,
        ids: &[Uuid],
        from: Option<ExifSyncStatus>,
        to: ExifSyncStatus,
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if ids.is_empty() {
            return Ok(0);
        }
        let res = sqlx::query!(
            "UPDATE pictures SET exif_sync_status = $2
              WHERE id = ANY($1) AND ($3::picture_exif_sync_status IS NULL
                                      OR exif_sync_status = $3)",
            ids,
            to as ExifSyncStatus,
            from as Option<ExifSyncStatus>,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected())
    }

    /// Up to `limit` owned pictures whose `exif_sync_status` is one of `statuses` and that have no
    /// `gen_thumbnail` job in flight, optionally narrowed to `mime_types` (lower-cased). Returns
    /// `(picture_id, owner_id)` — the worklist shape the sweeps and drains share.
    #[tracing::instrument(skip(ex, statuses, mime_types))]
    pub async fn find_by_exif_sync_status(
        ex: &PgPool,
        statuses: &[ExifSyncStatus],
        mime_types: Option<&[String]>,
        limit: i64,
    ) -> Result<Vec<(Uuid, Uuid)>, AppError> {
        if statuses.is_empty() {
            return Ok(Vec::new());
        }
        let labels: Vec<String> = statuses.iter().map(|s| s.as_str().to_string()).collect();
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT p.id, p.local_user_id FROM pictures p \
             WHERE p.remote_picture_id IS NULL AND p.deleted_at IS NULL \
               AND p.exif_sync_status::text = ANY(",
        );
        q.push_bind(labels).push("::text[])");
        if let Some(mimes) = mime_types {
            q.push(" AND lower(p.mime_type) = ANY(")
                .push_bind(mimes.to_vec())
                .push("::text[])");
        }
        q.push(
            " AND NOT EXISTS (SELECT 1 FROM jobs j WHERE j.picture_id = p.id \
                                AND j.job_type = 'gen_thumbnail' \
                                AND j.status IN ('pending', 'processing')) \
             ORDER BY p.ingested_at LIMIT ",
        );
        q.push_bind(limit);
        let rows = q
            .build_query_as::<(Uuid, Uuid)>()
            .fetch_all(ex)
            .await
            .map_err(map_sqlx_error)?;
        Ok(rows)
    }

    /// Update only the persisted physical-file EXIF snapshot (`file_exif`) after a successful
    /// worker write, without mutating the authoritative DB EXIF columns.
    #[tracing::instrument(skip(ex, file_exif), fields(picture_id = %id))]
    pub async fn set_file_exif<'e, E>(ex: E, id: Uuid, file_exif: &FullExif) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let file_exif_json = serde_json::to_value(file_exif)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        sqlx::query!(
            "UPDATE pictures SET file_exif = $2::jsonb WHERE id = $1",
            id,
            file_exif_json
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

}
