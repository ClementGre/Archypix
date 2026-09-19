//! Sender-side tracking of per-picture presign tokens.
//!
//! Every `(outgoing_share, picture)` pair that has been announced to a recipient has one
//! row here, carrying a unique `picture_token`. The presign endpoint resolves a token
//! directly to a picture; the pipeline announcement step diffs current share coverage
//! against this table to decide what to announce / unannounce; revoking a share deletes
//! its rows (immediately invalidating every token it held).

use archypix_common::error::{AppError, map_sqlx_error};
use sqlx::{Executor, Postgres};
use uuid::Uuid;

/// A downstream recipient that must be told a now-deleted picture is unannounced.
#[derive(Debug, Clone)]
pub struct DownstreamAnnouncement {
    pub outgoing_share_id: Uuid,
    /// The id the downstream recipient stored as `remote_picture_id` (original owner's id).
    pub announce_id: String,
    pub recipient_username: String,
    pub recipient_instance: String,
}

pub struct ShareAnnouncementRepository;

impl ShareAnnouncementRepository {
    /// Insert a tracking row for one picture, generating its token. Idempotent: if the row
    /// already exists the existing token is returned unchanged.
    #[tracing::instrument(skip(ex), fields(outgoing_share_id = %outgoing_share_id, picture_id = %picture_id))]
    pub async fn insert<'e, E>(
        ex: E,
        outgoing_share_id: Uuid,
        picture_id: Uuid,
    ) -> Result<Uuid, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_scalar!(
            r#"INSERT INTO share_announcements (outgoing_share_id, picture_id)
               VALUES ($1, $2)
               ON CONFLICT (outgoing_share_id, picture_id) DO UPDATE
                   SET picture_token = share_announcements.picture_token
               RETURNING picture_token"#,
            outgoing_share_id,
            picture_id,
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Insert/refresh a tracking row with an explicit token and the picture's `updated_at` at the
    /// moment of this successful (re-)announce. The token must equal the upstream `incoming_share`
    /// tag token for received/transitive pictures. `announced_updated_at` gates metadata
    /// re-announce (§10.3): a later `pictures.updated_at` triggers another announce.
    #[tracing::instrument(skip(ex), fields(outgoing_share_id = %outgoing_share_id, picture_id = %picture_id))]
    pub async fn insert_with_token<'e, E>(
        ex: E,
        outgoing_share_id: Uuid,
        picture_id: Uuid,
        picture_token: Uuid,
        announced_updated_at: Option<chrono::NaiveDateTime>,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"INSERT INTO share_announcements
                   (outgoing_share_id, picture_id, picture_token, announced_updated_at)
               VALUES ($1, $2, $3, $4)
               ON CONFLICT (outgoing_share_id, picture_id)
               DO UPDATE SET picture_token = EXCLUDED.picture_token,
                             announced_updated_at = EXCLUDED.announced_updated_at"#,
            outgoing_share_id,
            picture_id,
            picture_token,
            announced_updated_at,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Picture ids currently covered by one share — those carrying a tag at-or-under the share's
    /// `tag_path`. Loop prevention is applied inline (pictures whose original owner is the share
    /// recipient are excluded). `picture_ids = None` returns the full coverage.
    #[tracing::instrument(skip(ex, picture_ids), fields(owner_id = %owner_id))]
    pub async fn coverage_for_share<'e, E>(
        ex: E,
        owner_id: Uuid,
        tag_path_ltree: &str,
        recipient_username: &str,
        recipient_instance: &str,
        picture_ids: Option<&[Uuid]>,
    ) -> Result<Vec<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if matches!(picture_ids, Some(ids) if ids.is_empty()) {
            return Ok(vec![]);
        }
        let rows = sqlx::query_scalar!(
            r#"SELECT DISTINCT t.picture_id
               FROM tags t
               JOIN pictures p ON p.id = t.picture_id
               WHERE p.local_user_id = $1
                 AND t.tag_path <@ $2::text::ltree
                 AND ($3::uuid[] IS NULL OR t.picture_id = ANY($3::uuid[]))
                 AND (
                       p.owner_username IS DISTINCT FROM $4
                    OR p.owner_instance_domain IS DISTINCT FROM $5
                 )"#,
            owner_id,
            tag_path_ltree,
            picture_ids as Option<&[Uuid]>,
            recipient_username,
            recipient_instance,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(rows)
    }

    /// Tracking rows for one share: `(picture_id, picture_token, announced_updated_at)`.
    /// `picture_ids = None` returns all of the share's rows (full reconcile); `Some(ids)` restricts
    /// to those pictures.
    #[tracing::instrument(skip(ex, picture_ids), fields(outgoing_share_id = %outgoing_share_id))]
    pub async fn tracking_for_share<'e, E>(
        ex: E,
        outgoing_share_id: Uuid,
        picture_ids: Option<&[Uuid]>,
    ) -> Result<Vec<(Uuid, Uuid, Option<chrono::NaiveDateTime>)>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if matches!(picture_ids, Some(ids) if ids.is_empty()) {
            return Ok(vec![]);
        }
        let rows = sqlx::query!(
            r#"SELECT picture_id, picture_token, announced_updated_at
               FROM share_announcements
               WHERE outgoing_share_id = $1
                 AND ($2::uuid[] IS NULL OR picture_id = ANY($2::uuid[]))"#,
            outgoing_share_id,
            picture_ids as Option<&[Uuid]>,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(rows
            .into_iter()
            .map(|r| (r.picture_id, r.picture_token, r.announced_updated_at))
            .collect())
    }

    /// Whether `picture_id` appears in any share's tracking table — i.e. it has been announced to
    /// at least one recipient. The worker-completion re-announce path checks this before marking a
    /// picture dirty: a not-yet-announced picture has nothing to refresh (its first announce will
    /// carry the now-complete metadata), so only tracked pictures are worth waking the pipeline for.
    #[tracing::instrument(skip(ex), fields(picture_id = %picture_id))]
    pub async fn is_picture_tracked<'e, E>(ex: E, picture_id: Uuid) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let exists = sqlx::query_scalar!(
            r#"SELECT EXISTS(
                   SELECT 1 FROM share_announcements WHERE picture_id = $1
               ) AS "exists!""#,
            picture_id,
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(exists)
    }

    /// Picture ids whose last announce lags the picture (`announced_updated_at < pictures.updated_at`)
    /// — the announce-stale set. The recovery sweep marks these dirty so the pipeline re-announces
    /// the refreshed metadata. This is the race-free correctness backstop for the worker-completion
    /// fast path: even if a fast-path wake lost the race against a picture's first announce, a
    /// tracking row that trails its picture is eventually reconciled here.
    ///
    /// Rows of `revoked`/`tombstoned` shares are excluded as a precaution: those shares are never
    /// re-announced, so a lingering stale row must not re-dirty the picture forever (both paths delete
    /// their rows, so this is defence-in-depth).
    #[tracing::instrument(skip(db))]
    pub async fn find_stale_announcement_pictures(
        db: &sqlx::PgPool,
    ) -> Result<Vec<Uuid>, AppError> {
        sqlx::query_scalar!(
            r#"SELECT DISTINCT sa.picture_id
               FROM share_announcements sa
               JOIN pictures p ON p.id = sa.picture_id
               JOIN outgoing_shares os ON os.id = sa.outgoing_share_id
               WHERE sa.announced_updated_at IS NOT NULL
                 AND sa.announced_updated_at < p.updated_at
                 AND os.status NOT IN ('revoked'::share_status, 'tombstoned'::share_status)"#,
        )
        .fetch_all(db)
        .await
        .map_err(map_sqlx_error)
    }

    /// Update the token of an existing tracking row (token-refresh path).
    #[tracing::instrument(skip(ex), fields(outgoing_share_id = %outgoing_share_id, picture_id = %picture_id))]
    pub async fn update_token<'e, E>(
        ex: E,
        outgoing_share_id: Uuid,
        picture_id: Uuid,
        new_token: Uuid,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"UPDATE share_announcements SET picture_token = $3
               WHERE outgoing_share_id = $1 AND picture_id = $2"#,
            outgoing_share_id,
            picture_id,
            new_token,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Delete the tracking row for one `(share, picture)` pair (picture left coverage).
    #[tracing::instrument(skip(ex), fields(outgoing_share_id = %outgoing_share_id, picture_id = %picture_id))]
    pub async fn delete<'e, E>(
        ex: E,
        outgoing_share_id: Uuid,
        picture_id: Uuid,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"DELETE FROM share_announcements
               WHERE outgoing_share_id = $1 AND picture_id = $2"#,
            outgoing_share_id,
            picture_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Delete all tracking rows for a set of pictures, across every share. Used by
    /// `cleanup_incoming_share` after the downstream unannounce tasks are enqueued.
    #[tracing::instrument(skip(ex, picture_ids))]
    pub async fn delete_for_pictures<'e, E>(ex: E, picture_ids: &[Uuid]) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if picture_ids.is_empty() {
            return Ok(());
        }
        sqlx::query!(
            r#"DELETE FROM share_announcements WHERE picture_id = ANY($1::uuid[])"#,
            picture_ids as &[Uuid],
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Delete every tracking row for a share. Used by `revoke_outgoing_share` — all of the
    /// share's tokens die at once.
    #[tracing::instrument(skip(ex), fields(outgoing_share_id = %outgoing_share_id))]
    pub async fn delete_all_for_share<'e, E>(ex: E, outgoing_share_id: Uuid) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"DELETE FROM share_announcements WHERE outgoing_share_id = $1"#,
            outgoing_share_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Resolve a `picture_token` to the *owned* picture it grants access to. The presign
    /// endpoint's only authorization check: an unknown token (or one that only matches a relayed,
    /// non-owned tracking row) yields `None`. Filtering to owned pictures disambiguates the case
    /// where a relayer copied an upstream token into its own tracking row on the same instance.
    #[tracing::instrument(skip(ex))]
    pub async fn find_picture_by_token<'e, E>(
        ex: E,
        picture_token: Uuid,
    ) -> Result<Option<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_scalar!(
            r#"SELECT sa.picture_id
               FROM share_announcements sa
               JOIN pictures p ON p.id = sa.picture_id
               WHERE sa.picture_token = $1
                 AND p.remote_picture_id IS NULL
               LIMIT 1"#,
            picture_token,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Find downstream recipients of a set of deleted pictures, so `cleanup_incoming_share`
    /// can enqueue `UnannounceSharedPictures` tasks before deleting the tracking rows.
    #[tracing::instrument(skip(ex, picture_ids))]
    pub async fn find_downstream_for_pictures<'e, E>(
        ex: E,
        picture_ids: &[Uuid],
    ) -> Result<Vec<DownstreamAnnouncement>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if picture_ids.is_empty() {
            return Ok(vec![]);
        }
        // Must be called *before* the picture rows are deleted so `remote_picture_id` is
        // available; falls back to the local id text for owned pictures (remote_picture_id NULL).
        let rows = sqlx::query!(
            r#"SELECT sa.outgoing_share_id,
                      COALESCE(p.remote_picture_id, sa.picture_id::text) AS "announce_id!",
                      os.recipient_username, os.recipient_instance
               FROM share_announcements sa
               JOIN outgoing_shares os ON os.id = sa.outgoing_share_id
               LEFT JOIN pictures p ON p.id = sa.picture_id
               WHERE sa.picture_id = ANY($1::uuid[])"#,
            picture_ids as &[Uuid],
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)?;

        Ok(rows
            .into_iter()
            .map(|r| DownstreamAnnouncement {
                outgoing_share_id: r.outgoing_share_id,
                announce_id: r.announce_id,
                recipient_username: r.recipient_username,
                recipient_instance: r.recipient_instance,
            })
            .collect())
    }
}

