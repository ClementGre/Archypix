use archypix_common::error::{AppError, map_sqlx_error};
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;
use super::*;

impl PictureRepository {
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
    #[tracing::instrument(skip(db, sel), fields(user_id = %local_user_id))]
    pub async fn count_owned_unsupported_selection(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
    ) -> Result<i64, AppError> {
        if sel.is_empty() {
            return Ok(0);
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT COUNT(*) FROM pictures p WHERE p.remote_picture_id IS NULL \
             AND p.exif_sync_status IN ('unsupported_mime', 'unsupported_file') AND ",
        );
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

}
