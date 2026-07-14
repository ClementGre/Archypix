//! Public-share persistence (feature 27): CRUD + the live coverage queries.
//!
//! Coverage is never announced/tracked — every read resolves it fresh: a picture is covered by a
//! public share iff it is owned by the share owner, carries a tag `<@ tag_path`, and is neither
//! trashed nor a hidden content-dedup row (`deleted_at IS NULL` covers both). The full-page listing
//! reuses `PictureRepository::list` (owner as the local user + an include predicate); this module adds
//! the single-picture coverage check and the contribution queries.

use crate::domain::picture::Picture;
use crate::domain::public_share::{PublicShare, PublicShareStatus};
use crate::repository::picture::PictureRepository;
use archypix_common::error::{AppError, map_sqlx_error};
use chrono::NaiveDateTime;
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;

pub struct PublicShareRepository;

impl PublicShareRepository {
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip(ex), fields(owner_id = %owner_id))]
    pub async fn create<'e, E>(
        ex: E,
        owner_id: Uuid,
        tag_path: &str,
        name: &str,
        message: Option<&str>,
        token: &str,
        password_hash: Option<&str>,
        expires_at: Option<NaiveDateTime>,
        allow_originals: bool,
        allow_upload: bool,
        allow_share_back: bool,
        conv_allow_exif_edit: bool,
        conv_future: bool,
    ) -> Result<PublicShare, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            PublicShare,
            r#"INSERT INTO public_shares
                   (owner_id, tag_path, name, message, token, password_hash, expires_at,
                    allow_originals, allow_upload, allow_share_back, conv_allow_exif_edit, conv_future)
               VALUES ($1, $2::text::ltree, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
               RETURNING id, owner_id, tag_path::text as "tag_path!",
                         name, message, token, password_hash, expires_at,
                         allow_originals, allow_upload, allow_share_back,
                         conv_allow_exif_edit, conv_future,
                         status as "status: PublicShareStatus",
                         created_at, revoked_at"#,
            owner_id,
            tag_path,
            name,
            message,
            token,
            password_hash,
            expires_at,
            allow_originals,
            allow_upload,
            allow_share_back,
            conv_allow_exif_edit,
            conv_future,
        )
            .fetch_one(ex)
            .await
            .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex))]
    pub async fn find_by_token<'e, E>(ex: E, token: &str) -> Result<Option<PublicShare>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            PublicShare,
            r#"SELECT id, owner_id, tag_path::text as "tag_path!",
                      name, message, token, password_hash, expires_at,
                      allow_originals, allow_upload, allow_share_back,
                      conv_allow_exif_edit, conv_future,
                      status as "status: PublicShareStatus",
                      created_at, revoked_at
               FROM public_shares WHERE token = $1"#,
            token,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex), fields(share_id = %id))]
    pub async fn find_by_id<'e, E>(ex: E, id: Uuid) -> Result<Option<PublicShare>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            PublicShare,
            r#"SELECT id, owner_id, tag_path::text as "tag_path!",
                      name, message, token, password_hash, expires_at,
                      allow_originals, allow_upload, allow_share_back,
                      conv_allow_exif_edit, conv_future,
                      status as "status: PublicShareStatus",
                      created_at, revoked_at
               FROM public_shares WHERE id = $1"#,
            id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex), fields(owner_id = %owner_id))]
    pub async fn list_by_owner<'e, E>(ex: E, owner_id: Uuid) -> Result<Vec<PublicShare>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            PublicShare,
            r#"SELECT id, owner_id, tag_path::text as "tag_path!",
                      name, message, token, password_hash, expires_at,
                      allow_originals, allow_upload, allow_share_back,
                      conv_allow_exif_edit, conv_future,
                      status as "status: PublicShareStatus",
                      created_at, revoked_at
               FROM public_shares WHERE owner_id = $1 ORDER BY created_at DESC"#,
            owner_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Update the editable fields of a public share (name/message/password/expiry + permission
    /// flags). `password_hash`/`expires_at` are passed through verbatim (caller decides keep vs clear).
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip(ex), fields(share_id = %id))]
    pub async fn update<'e, E>(
        ex: E,
        id: Uuid,
        owner_id: Uuid,
        name: &str,
        message: Option<&str>,
        password_hash: Option<&str>,
        expires_at: Option<NaiveDateTime>,
        allow_originals: bool,
        allow_upload: bool,
        allow_share_back: bool,
        conv_allow_exif_edit: bool,
        conv_future: bool,
    ) -> Result<Option<PublicShare>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            PublicShare,
            r#"UPDATE public_shares
               SET name = $3, message = $4, password_hash = $5, expires_at = $6,
                   allow_originals = $7, allow_upload = $8, allow_share_back = $9,
                   conv_allow_exif_edit = $10, conv_future = $11
               WHERE id = $1 AND owner_id = $2
               RETURNING id, owner_id, tag_path::text as "tag_path!",
                         name, message, token, password_hash, expires_at,
                         allow_originals, allow_upload, allow_share_back,
                         conv_allow_exif_edit, conv_future,
                         status as "status: PublicShareStatus",
                         created_at, revoked_at"#,
            id,
            owner_id,
            name,
            message,
            password_hash,
            expires_at,
            allow_originals,
            allow_upload,
            allow_share_back,
            conv_allow_exif_edit,
            conv_future,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Revoke a public share (owner-scoped). Idempotent; stamps `revoked_at` on the first revoke.
    /// Returns whether a row was updated.
    #[tracing::instrument(skip(ex), fields(share_id = %id))]
    pub async fn revoke<'e, E>(ex: E, id: Uuid, owner_id: Uuid) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let res = sqlx::query!(
            r#"UPDATE public_shares
               SET status = 'revoked'::public_share_status,
                   revoked_at = COALESCE(revoked_at, now() AT TIME ZONE 'utc')
               WHERE id = $1 AND owner_id = $2"#,
            id,
            owner_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected() > 0)
    }

    /// Fetch a single picture **iff** it is in the share's live coverage: owned by `owner_id`, tagged
    /// `<@ tag_path`, not trashed and not a hidden content-dedup row (`deleted_at IS NULL`). The
    /// coverage check that replaces the ownership check in the public presign/detail path (§6). A cheap
    /// `EXISTS` gate, then the shared `PictureRepository::find_by_id` load (no column duplication).
    #[tracing::instrument(skip(db), fields(owner_id = %owner_id, picture_id = %picture_id))]
    pub async fn find_covered_picture(
        db: &PgPool,
        owner_id: Uuid,
        tag_path: &str,
        picture_id: Uuid,
    ) -> Result<Option<Picture>, AppError> {
        let covered: bool = sqlx::query_scalar!(
            r#"SELECT EXISTS (
                 SELECT 1 FROM pictures p
                 WHERE p.id = $1 AND p.local_user_id = $2 AND p.deleted_at IS NULL
                   AND EXISTS (SELECT 1 FROM tags t WHERE t.picture_id = p.id
                                                      AND t.tag_path <@ $3::text::ltree)
               ) AS "e!""#,
            picture_id,
            owner_id,
            tag_path,
        )
        .fetch_one(db)
        .await
        .map_err(map_sqlx_error)?;
        if !covered {
            return Ok(None);
        }
        PictureRepository::find_by_id(db, picture_id).await
    }

    /// Of `candidate_ids`, the subset in the share's live coverage (owned by `owner_id`, tagged
    /// `<@ tag_path`, not deleted/hidden). Used to intersect a visitor's batch selection with coverage
    /// before aggregating EXIF, so a visitor can never aggregate outside the album (§6).
    #[tracing::instrument(skip(ex, candidate_ids), fields(owner_id = %owner_id, n = candidate_ids.len()))]
    pub async fn filter_covered_ids<'e, E>(
        ex: E,
        owner_id: Uuid,
        tag_path: &str,
        candidate_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if candidate_ids.is_empty() {
            return Ok(vec![]);
        }
        sqlx::query_scalar!(
            r#"SELECT p.id
               FROM pictures p
               WHERE p.id = ANY($1)
                 AND p.local_user_id = $2
                 AND p.deleted_at IS NULL
                 AND EXISTS (SELECT 1 FROM tags t WHERE t.picture_id = p.id
                                                    AND t.tag_path <@ $3::text::ltree)"#,
            candidate_ids,
            owner_id,
            tag_path,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Ids of the owner's live contributed pictures under a public share's tag — owned rows
    /// (`remote_picture_id IS NULL`) whose `creator` is a `#…` name (§7). When `contributor` is
    /// given, restrict to that exact `#name`. Used by "bulk-remove a contributor" (§9).
    #[tracing::instrument(skip(ex), fields(owner_id = %owner_id))]
    pub async fn contribution_ids<'e, E>(
        ex: E,
        owner_id: Uuid,
        tag_path: &str,
        contributor: Option<&str>,
    ) -> Result<Vec<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_scalar!(
            r#"SELECT p.id
               FROM pictures p
               WHERE p.local_user_id = $1
                 AND p.remote_picture_id IS NULL
                 AND p.deleted_at IS NULL
                 AND p.creator LIKE '#%'
                 AND ($3::text IS NULL OR p.creator = $3)
                 AND EXISTS (SELECT 1 FROM tags t WHERE t.picture_id = p.id
                                                    AND t.tag_path <@ $2::text::ltree)"#,
            owner_id,
            tag_path,
            contributor,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Rename a public share's `tag_path` for a tag-rename cascade (§14) — every share whose tag is
    /// `old` or under it gets its `old` prefix swapped for `new`. Returns the number renamed.
    #[tracing::instrument(skip(ex), fields(owner_id = %owner_id))]
    pub async fn rename_tag_subtree<'e, E>(
        ex: E,
        owner_id: Uuid,
        old_ltree: &str,
        new_ltree: &str,
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let res = sqlx::query!(
            r#"UPDATE public_shares
               SET tag_path = CASE WHEN tag_path = $2::text::ltree THEN $3::text::ltree
                                   ELSE $3::text::ltree || subpath(tag_path, nlevel($2::text::ltree)) END
               WHERE owner_id = $1 AND tag_path <@ $2::text::ltree"#,
            owner_id,
            old_ltree,
            new_ltree,
        )
            .execute(ex)
            .await
            .map_err(map_sqlx_error)?;
        Ok(res.rows_affected())
    }
}
