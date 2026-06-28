//! Content-dedup repository queries (feature 11 §5).
//!
//! Rows are grouped per user by a **content key**: `content_hash` when present, else
//! `'fh:' || file_hash` (the non-strippable-format fallback, §7.6). Rows with neither hash do not
//! group. The reconciler ([`crate::infra::routine::pipeline::dedup`]) keeps exactly one live survivor per
//! group and hides the rest as `content_dedupe`; `manual`/`boomerang` rows are content rejections it
//! never touches but does respect.

use crate::domain::picture::DeletedReason;
use crate::infra::error::{AppError, map_sqlx_error};
use chrono::NaiveDateTime;
use sqlx::{Executor, Postgres};
use uuid::Uuid;

/// One row of a content-dedup group, with the bits survivor selection (§5.1) needs.
#[derive(Debug, Clone)]
pub struct DedupRow {
    pub id: Uuid,
    pub deleted_at: Option<NaiveDateTime>,
    pub deleted_reason: Option<DeletedReason>,
    /// `remote_picture_id IS NULL` — the caller owns the file.
    pub is_owned: bool,
    /// `copy_source_picture_id IS NOT NULL` — a physical copy rather than an original.
    pub is_copy: bool,
    /// Received rows only: the owner's soft-delete (a soon-to-vanish original is a worse survivor).
    pub owner_deleted_at: Option<NaiveDateTime>,
}

/// A row in a content-dedup group, as surfaced to the owner by the copies endpoint (feature 11
/// §5.5). Carries both hashes (so the client can show "same content, EXIF-only difference" vs
/// "different content"), the dedup state, the last-edit time, and the owner / provenance identity.
#[derive(Debug, Clone)]
pub struct CopyRow {
    pub id: Uuid,
    pub content_hash: Option<String>,
    pub file_hash: Option<String>,
    pub deleted_reason: Option<DeletedReason>,
    pub deleted_at: Option<NaiveDateTime>,
    pub updated_at: NaiveDateTime,
    pub is_owned: bool,
    pub owner_username: Option<String>,
    pub owner_instance_domain: Option<String>,
    pub owner_deleted_at: Option<NaiveDateTime>,
    pub copy_source_owner_username: Option<String>,
    pub copy_source_owner_instance: Option<String>,
    pub copy_source_picture_id: Option<String>,
    pub filename: Option<String>,
}

pub struct DedupRepository;

impl DedupRepository {
    /// Content keys of `user_id` that may need reconciliation: a group with more than one row
    /// (survivor selection), or any group holding a `content_dedupe` row (a possible promotion). A
    /// lone live/`manual`/`boomerang` group is already consistent and never returned.
    #[tracing::instrument(skip(ex), fields(user_id = %user_id))]
    pub async fn find_candidate_keys<'e, E>(ex: E, user_id: Uuid) -> Result<Vec<String>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_scalar!(
            r#"SELECT g.key AS "key!"
               FROM (
                 SELECT COALESCE(content_hash, 'fh:' || file_hash) AS key,
                        COUNT(*) AS cnt,
                        bool_or(deleted_reason = 'content_dedupe'::picture_deleted_reason) AS has_dedupe,
                        bool_or(deleted_reason = 'boomerang'::picture_deleted_reason) AS has_boomerang,
                        bool_or(deleted_reason = 'manual'::picture_deleted_reason) AS has_manual
                 FROM pictures
                 WHERE local_user_id = $1
                   AND (content_hash IS NOT NULL OR file_hash IS NOT NULL)
                 GROUP BY COALESCE(content_hash, 'fh:' || file_hash)
               ) g
               -- multi-row (collapse) | hidden dedupe (promote) | boomerang-no-manual (new rep)
               WHERE g.cnt > 1 OR g.has_dedupe OR (g.has_boomerang AND NOT g.has_manual)"#,
            user_id,
        )
            .fetch_all(ex)
            .await
            .map_err(map_sqlx_error)
    }

    /// Users with a content group needing an action (collapse / rescue / new representative) — the
    /// recovery-sweep backstop for a lost wake. See doc/features/11 §5.
    #[tracing::instrument(skip(ex))]
    pub async fn find_users_needing_reconcile<'e, E>(ex: E) -> Result<Vec<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_scalar!(
            r#"SELECT DISTINCT g.local_user_id AS "id!"
               FROM (
                 SELECT local_user_id,
                        COUNT(*) FILTER (WHERE deleted_at IS NULL) AS live,
                        COUNT(*) FILTER (WHERE deleted_reason = 'content_dedupe'::picture_deleted_reason) AS dedupe,
                        COUNT(*) FILTER (WHERE deleted_reason = 'boomerang'::picture_deleted_reason) AS boomerang,
                        COUNT(*) FILTER (WHERE deleted_reason = 'manual'::picture_deleted_reason) AS manual,
                        bool_or(deleted_reason IN ('manual'::picture_deleted_reason,
                                                   'boomerang'::picture_deleted_reason)) AS blocked
                 FROM pictures
                 WHERE content_hash IS NOT NULL OR file_hash IS NOT NULL
                 GROUP BY local_user_id, COALESCE(content_hash, 'fh:' || file_hash)
               ) g
               WHERE g.live > 1
                  OR (g.live = 0 AND g.dedupe > 0 AND NOT g.blocked)
                  OR (g.live = 0 AND g.boomerang > 0 AND g.manual = 0)"#,
        )
            .fetch_all(ex)
            .await
            .map_err(map_sqlx_error)
    }

    /// The content key of a single picture (`content_hash`, else `'fh:'||file_hash`, else `None`).
    /// Used to reconcile just the group an arrival/manual-delete touched.
    #[tracing::instrument(skip(ex), fields(picture_id = %picture_id))]
    pub async fn content_key_of<'e, E>(ex: E, picture_id: Uuid) -> Result<Option<String>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let row = sqlx::query_scalar!(
            r#"SELECT COALESCE(content_hash, 'fh:' || file_hash) AS key
               FROM pictures WHERE id = $1"#,
            picture_id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(row.flatten())
    }

    /// The full content group (including hidden rows) of the picture `picture_id`, for the copies
    /// endpoint. Empty when the picture is not the caller's or has no content/file hash yet. Ordered
    /// live-first (the survivor), then by id.
    #[tracing::instrument(skip(ex), fields(user_id = %user_id, picture_id = %picture_id))]
    pub async fn list_content_group<'e, E>(
        ex: E,
        user_id: Uuid,
        picture_id: Uuid,
    ) -> Result<Vec<CopyRow>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            CopyRow,
            r#"SELECT g.id,
                      g.content_hash,
                      g.file_hash,
                      g.deleted_reason as "deleted_reason: DeletedReason",
                      g.deleted_at,
                      g.updated_at,
                      (g.remote_picture_id IS NULL) as "is_owned!",
                      g.owner_username,
                      g.owner_instance_domain,
                      g.owner_deleted_at,
                      g.copy_source_owner_username,
                      g.copy_source_owner_instance,
                      g.copy_source_picture_id,
                      g.filename
               FROM pictures g
               WHERE g.local_user_id = $1
                 AND COALESCE(g.content_hash, 'fh:' || g.file_hash) = (
                     SELECT COALESCE(content_hash, 'fh:' || file_hash)
                     FROM pictures WHERE id = $2 AND local_user_id = $1
                 )
               ORDER BY g.deleted_at NULLS FIRST, g.id"#,
            user_id,
            picture_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// All rows of `user_id` in the content group `key`.
    #[tracing::instrument(skip(ex), fields(user_id = %user_id))]
    pub async fn list_group_rows<'e, E>(
        ex: E,
        user_id: Uuid,
        key: &str,
    ) -> Result<Vec<DedupRow>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            DedupRow,
            r#"SELECT id,
                      deleted_at,
                      deleted_reason as "deleted_reason: DeletedReason",
                      (remote_picture_id IS NULL) as "is_owned!",
                      (copy_source_picture_id IS NOT NULL) as "is_copy!",
                      owner_deleted_at
               FROM pictures
               WHERE local_user_id = $1
                 AND COALESCE(content_hash, 'fh:' || file_hash) = $2"#,
            user_id,
            key,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// The live sibling (`deleted_at IS NULL`, `id <> picture_id`) of `picture_id`'s content group,
    /// if any. A stable group has at most one; the lowest id is chosen for determinism. Used by the
    /// "keep this copy" flow to find the previously-live picture whose curated manual tag set the new
    /// survivor should mirror (feature 11 §5.5).
    #[tracing::instrument(skip(ex), fields(user_id = %user_id, picture_id = %picture_id))]
    pub async fn live_id_in_group<'e, E>(
        ex: E,
        user_id: Uuid,
        picture_id: Uuid,
    ) -> Result<Option<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_scalar!(
            r#"SELECT p.id
               FROM pictures p, pictures src
               WHERE src.id = $2 AND src.local_user_id = $1
                 AND p.local_user_id = $1
                 AND p.id <> src.id
                 AND p.deleted_at IS NULL
                 AND COALESCE(p.content_hash, 'fh:' || p.file_hash)
                     = COALESCE(src.content_hash, 'fh:' || src.file_hash)
               ORDER BY p.id
               LIMIT 1"#,
            user_id,
            picture_id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Distinct `manual` tag paths held by the **other** rows in `picture_id`'s content group. The
    /// dedup reconciler unions these onto the live survivor so a hidden copy's curation is not lost
    /// (feature 11 §5.5).
    #[tracing::instrument(skip(ex), fields(user_id = %user_id, picture_id = %picture_id))]
    pub async fn group_manual_tag_paths<'e, E>(
        ex: E,
        user_id: Uuid,
        picture_id: Uuid,
    ) -> Result<Vec<String>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_scalar!(
            r#"SELECT DISTINCT t.tag_path::text as "tag_path!"
               FROM pictures src
               JOIN pictures sib ON sib.local_user_id = src.local_user_id
                 AND sib.id <> src.id
                 AND COALESCE(sib.content_hash, 'fh:' || sib.file_hash)
                     = COALESCE(src.content_hash, 'fh:' || src.file_hash)
               JOIN tags t ON t.picture_id = sib.id AND t.source = 'manual'::tag_source
               WHERE src.id = $2 AND src.local_user_id = $1"#,
            user_id,
            picture_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Hide a row as `content_dedupe` (set `deleted_at` if it was live; reason → `content_dedupe`).
    #[tracing::instrument(skip(ex), fields(picture_id = %id))]
    pub async fn set_content_dedupe<'e, E>(ex: E, id: Uuid) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"UPDATE pictures
               SET deleted_at = COALESCE(deleted_at, now() AT TIME ZONE 'utc'),
                   deleted_reason = 'content_dedupe'::picture_deleted_reason,
                   last_pipeline_run_at = NULL
               WHERE id = $1"#,
            id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Promote a **`content_dedupe`** row back to live (clears `deleted_at`/`deleted_reason`). The
    /// `deleted_reason = 'content_dedupe'` guard makes this a no-op on a `manual`/`boomerang` row, so
    /// a rejection is never accidentally surfaced.
    #[tracing::instrument(skip(ex), fields(picture_id = %id))]
    pub async fn promote_to_live<'e, E>(ex: E, id: Uuid) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"UPDATE pictures
               SET deleted_at = NULL,
                   deleted_reason = NULL,
                   last_pipeline_run_at = NULL
               WHERE id = $1 AND deleted_reason = 'content_dedupe'::picture_deleted_reason"#,
            id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Make a row the `manual` trash **representative** of its rejected group. Keeps `deleted_at`
    /// (sets it if the row was live). Used to promote a `boomerang` row when the previous manual
    /// representative disappears (purge/unannounce/permanent-delete), so the rejected content still
    /// shows exactly one entry in the trash.
    #[tracing::instrument(skip(ex), fields(picture_id = %id))]
    pub async fn set_manual<'e, E>(ex: E, id: Uuid) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"UPDATE pictures
               SET deleted_at = COALESCE(deleted_at, now() AT TIME ZONE 'utc'),
                   deleted_reason = 'manual'::picture_deleted_reason,
                   last_pipeline_run_at = NULL
               WHERE id = $1"#,
            id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Mark a row as a sticky `boomerang` rejection (§5.3). Sets `deleted_at` if it was live.
    #[tracing::instrument(skip(ex), fields(picture_id = %id))]
    pub async fn set_boomerang<'e, E>(ex: E, id: Uuid) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"UPDATE pictures
               SET deleted_at = COALESCE(deleted_at, now() AT TIME ZONE 'utc'),
                   deleted_reason = 'boomerang'::picture_deleted_reason,
                   last_pipeline_run_at = NULL
               WHERE id = $1"#,
            id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Reject the whole content group of `picture_id` (the delete path, §5.3): its priority copy
    /// (same ordering as the reconciler's `best()`) → `manual` representative, the rest → `boomerang`.
    /// Returns rows trashed (0 if the picture has no content/file hash — `set_deleted` handled it).
    /// See doc/features/11 §5.5.
    #[tracing::instrument(skip(ex), fields(user_id = %user_id, picture_id = %picture_id))]
    pub async fn reject_content_group<'e, E>(
        ex: E,
        user_id: Uuid,
        picture_id: Uuid,
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let res = sqlx::query!(
            r#"WITH grp AS (
                 SELECT p.id
                 FROM pictures p, pictures src
                 WHERE src.id = $2 AND src.local_user_id = $1
                   AND p.local_user_id = $1
                   AND COALESCE(p.content_hash, 'fh:' || p.file_hash)
                       = COALESCE(src.content_hash, 'fh:' || src.file_hash)
               ),
               rep AS (
                 SELECT p.id
                 FROM pictures p JOIN grp ON grp.id = p.id
                 ORDER BY (p.owner_deleted_at IS NOT NULL),
                          (p.remote_picture_id IS NOT NULL),
                          (p.copy_source_picture_id IS NOT NULL),
                          p.id
                 LIMIT 1
               )
               UPDATE pictures p
               SET deleted_at = COALESCE(p.deleted_at, now() AT TIME ZONE 'utc'),
                   deleted_reason = CASE WHEN p.id = (SELECT id FROM rep)
                                         THEN 'manual'::picture_deleted_reason
                                         ELSE 'boomerang'::picture_deleted_reason END,
                   last_pipeline_run_at = NULL
               WHERE p.id IN (SELECT id FROM grp)"#,
            user_id,
            picture_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected())
    }

    /// Inverse of [`boomerang_dedupe_siblings`](Self::boomerang_dedupe_siblings): on **restore** of a
    /// manual representative, flip its `boomerang` siblings back to `content_dedupe` so the rejection
    /// is lifted — otherwise, if the restored row later disappears, only boomerangs would remain and
    /// nothing would be promoted to live. Returns the number of siblings converted.
    #[tracing::instrument(skip(ex), fields(user_id = %user_id, picture_id = %picture_id))]
    pub async fn dedupe_boomerang_siblings<'e, E>(
        ex: E,
        user_id: Uuid,
        picture_id: Uuid,
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let res = sqlx::query!(
            r#"UPDATE pictures p
               SET deleted_reason = 'content_dedupe'::picture_deleted_reason,
                   last_pipeline_run_at = NULL
               FROM pictures src
               WHERE src.id = $2
                 AND p.local_user_id = $1
                 AND p.id <> src.id
                 AND p.deleted_reason = 'boomerang'::picture_deleted_reason
                 AND COALESCE(p.content_hash, 'fh:' || p.file_hash)
                     = COALESCE(src.content_hash, 'fh:' || src.file_hash)"#,
            user_id,
            picture_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected())
    }

    /// Set-based restore invariant: convert every `boomerang` row whose content group contains a
    /// **live** row back to `content_dedupe` (a boomerang can't coexist with a live survivor — the
    /// group is no longer rejected). Run after a batch restore, before the reconciler, so it doesn't
    /// see a stale "rejected" group and re-boomerang the restored row. Returns rows converted.
    #[tracing::instrument(skip(ex), fields(user_id = %user_id))]
    pub async fn dedupe_boomerang_in_live_groups<'e, E>(
        ex: E,
        user_id: Uuid,
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let res = sqlx::query!(
            r#"UPDATE pictures p
               SET deleted_reason = 'content_dedupe'::picture_deleted_reason,
                   last_pipeline_run_at = NULL
               WHERE p.local_user_id = $1
                 AND p.deleted_reason = 'boomerang'::picture_deleted_reason
                 AND EXISTS (
                   SELECT 1 FROM pictures m
                   WHERE m.local_user_id = $1
                     AND m.deleted_at IS NULL
                     AND m.id <> p.id
                     AND COALESCE(m.content_hash, 'fh:' || m.file_hash)
                         = COALESCE(p.content_hash, 'fh:' || p.file_hash)
                 )"#,
            user_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected())
    }

    /// Make `picture_id` the live survivor of its content group, hiding every sibling as
    /// `content_dedupe` (lifting any rejection). The reconciler leaves a correct single-live group
    /// untouched (it never reshuffles a stable survivor), so this user choice sticks without a pin
    /// flag. Returns false if the user holds no such picture.
    #[tracing::instrument(skip(ex), fields(user_id = %user_id, picture_id = %picture_id))]
    pub async fn set_survivor<'e, E>(
        ex: E,
        user_id: Uuid,
        picture_id: Uuid,
    ) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let res = sqlx::query!(
            r#"UPDATE pictures p
               SET deleted_at = CASE WHEN p.id = $2 THEN NULL
                                     ELSE COALESCE(p.deleted_at, now() AT TIME ZONE 'utc') END,
                   deleted_reason = CASE WHEN p.id = $2 THEN NULL
                                         ELSE 'content_dedupe'::picture_deleted_reason END,
                   last_pipeline_run_at = NULL
               FROM pictures src
               WHERE src.id = $2
                 AND src.local_user_id = $1
                 AND p.local_user_id = $1
                 AND COALESCE(p.content_hash, 'fh:' || p.file_hash)
                     = COALESCE(src.content_hash, 'fh:' || src.file_hash)"#,
            user_id,
            picture_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected() > 0)
    }

    /// Enforce the §5.3 invariant across **all** of `user_id`'s groups: every `content_dedupe` row
    /// sharing a content group with a `manual`-deleted row becomes `boomerang`. Idempotent — the
    /// set-based form used after a batch manual trash (the single-picture path uses the scoped
    /// [`boomerang_dedupe_siblings`](Self::boomerang_dedupe_siblings)).
    #[tracing::instrument(skip(ex), fields(user_id = %user_id))]
    pub async fn boomerang_dedupe_in_manual_groups<'e, E>(
        ex: E,
        user_id: Uuid,
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let res = sqlx::query!(
            r#"UPDATE pictures p
               SET deleted_reason = 'boomerang'::picture_deleted_reason,
                   last_pipeline_run_at = NULL
               WHERE p.local_user_id = $1
                 AND p.deleted_reason = 'content_dedupe'::picture_deleted_reason
                 AND EXISTS (
                   SELECT 1 FROM pictures m
                   WHERE m.local_user_id = $1
                     AND m.deleted_reason = 'manual'::picture_deleted_reason
                     AND m.id <> p.id
                     AND COALESCE(m.content_hash, 'fh:' || m.file_hash)
                         = COALESCE(p.content_hash, 'fh:' || p.file_hash)
                 )"#,
            user_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected())
    }
}
