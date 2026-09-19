use crate::domain::job::FullExif;
use archypix_common::error::{AppError, map_sqlx_error};
use sqlx::{Executor, Postgres};
use uuid::Uuid;
use super::*;

impl PictureRepository {
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
    /// 14 §5). One statement: rows carrying a terminal verdict take the DB-only branch and keep it,
    /// every other row is stamped `pending_job_creation` for the drain. `extracting` rows are
    /// skipped — an in-flight extraction would overwrite the edit (04 §11.2). The partition is the
    /// stored status, never a MIME list (feature 33 §10).
    #[tracing::instrument(skip(ex, sel, set, clear), fields(user_id = %local_user_id))]
    pub async fn batch_apply_exif_owned_selection<'e, E>(
        ex: E,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
        set: &FullExif,
        clear: &[crate::domain::job::ExifField],
    ) -> Result<BatchExifCounts, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if sel.is_empty() {
            return Ok(BatchExifCounts::default());
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new("UPDATE pictures AS p SET ");
        push_exif_column_assignments(&mut q, set, clear);
        q.push(
            "exif_sync_status = CASE \
               WHEN p.exif_sync_status IN ('unsupported_mime', 'unsupported_file') \
                 THEN p.exif_sync_status \
               ELSE 'pending_job_creation'::picture_exif_sync_status END",
        );
        q.push(", updated_at = (now() AT TIME ZONE 'utc'), last_pipeline_run_at = NULL WHERE ");
        Self::push_selection_where(&mut q, local_user_id, sel);
        q.push(
            " AND p.remote_picture_id IS NULL AND p.exif_sync_status <> 'extracting' \
             RETURNING p.exif_sync_status::text",
        );
        let rows: Vec<String> = q
            .build_query_scalar()
            .fetch_all(ex)
            .await
            .map_err(map_sqlx_error)?;
        let unsupported = rows
            .iter()
            .filter(|s| s.starts_with("unsupported_"))
            .count() as i64;
        Ok(BatchExifCounts {
            edited: rows.len() as i64 - unsupported,
            unsupported,
        })
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
