//! Sender-side tracking of per-picture presign tokens.
//!
//! Every `(outgoing_share, picture)` pair that has been announced to a recipient has one
//! row here, carrying a unique `picture_token`. The presign endpoint resolves a token
//! directly to a picture; the pipeline announcement step diffs current share coverage
//! against this table to decide what to announce / unannounce; revoking a share deletes
//! its rows (immediately invalidating every token it held).

use crate::infra::error::{AppError, map_sqlx_error};
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
    pub async fn find_stale_announcement_pictures(
        db: &sqlx::PgPool,
    ) -> Result<Vec<Uuid>, AppError> {
        sqlx::query_scalar!(
            r#"SELECT DISTINCT sa.picture_id
               FROM share_announcements sa
               JOIN pictures p ON p.id = sa.picture_id
               WHERE sa.announced_updated_at IS NOT NULL
                 AND sa.announced_updated_at < p.updated_at"#,
        )
        .fetch_all(db)
        .await
        .map_err(map_sqlx_error)
    }

    /// Update the token of an existing tracking row (token-refresh path).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::share::OutgoingShareRepository;
    use sqlx::PgPool;

    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

    async fn seed_user(db: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO users (id, username, email, display_name) VALUES ($1, $2, $3, $4)",
            id,
            format!("u_{}", &id.to_string()[..8]),
            format!("{}@t.com", id),
            "T",
        )
        .execute(db)
        .await
        .unwrap();
        id
    }

    async fn seed_picture(db: &PgPool, user_id: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO pictures (id, local_user_id) VALUES ($1, $2)",
            id,
            user_id,
        )
        .execute(db)
        .await
        .unwrap();
        id
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn insert_then_resolve_token(db: PgPool) {
        let owner = seed_user(&db).await;
        let pic = seed_picture(&db, owner).await;
        let share = OutgoingShareRepository::create(
            &db,
            owner,
            "Photos",
            "Test share",
            None,
            "bob",
            "other.com",
            true,
            false,
            true,
            None,
        )
        .await
        .unwrap();

        let token = ShareAnnouncementRepository::insert(&db, share.id, pic)
            .await
            .unwrap();
        let resolved = ShareAnnouncementRepository::find_picture_by_token(&db, token)
            .await
            .unwrap();
        assert_eq!(resolved, Some(pic));
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn insert_is_idempotent_keeps_token(db: PgPool) {
        let owner = seed_user(&db).await;
        let pic = seed_picture(&db, owner).await;
        let share = OutgoingShareRepository::create(
            &db,
            owner,
            "Photos",
            "Test share",
            None,
            "bob",
            "other.com",
            true,
            false,
            true,
            None,
        )
        .await
        .unwrap();

        let t1 = ShareAnnouncementRepository::insert(&db, share.id, pic)
            .await
            .unwrap();
        let t2 = ShareAnnouncementRepository::insert(&db, share.id, pic)
            .await
            .unwrap();
        assert_eq!(t1, t2, "token stable across re-insert");
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn delete_invalidates_token(db: PgPool) {
        let owner = seed_user(&db).await;
        let pic = seed_picture(&db, owner).await;
        let share = OutgoingShareRepository::create(
            &db,
            owner,
            "Photos",
            "Test share",
            None,
            "bob",
            "other.com",
            true,
            false,
            true,
            None,
        )
        .await
        .unwrap();

        let token = ShareAnnouncementRepository::insert(&db, share.id, pic)
            .await
            .unwrap();
        ShareAnnouncementRepository::delete(&db, share.id, pic)
            .await
            .unwrap();
        let resolved = ShareAnnouncementRepository::find_picture_by_token(&db, token)
            .await
            .unwrap();
        assert_eq!(resolved, None);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn delete_all_for_share_clears_every_token(db: PgPool) {
        let owner = seed_user(&db).await;
        let p1 = seed_picture(&db, owner).await;
        let p2 = seed_picture(&db, owner).await;
        let share = OutgoingShareRepository::create(
            &db,
            owner,
            "Photos",
            "Test share",
            None,
            "bob",
            "other.com",
            true,
            false,
            true,
            None,
        )
        .await
        .unwrap();

        let t1 = ShareAnnouncementRepository::insert(&db, share.id, p1)
            .await
            .unwrap();
        let t2 = ShareAnnouncementRepository::insert(&db, share.id, p2)
            .await
            .unwrap();
        ShareAnnouncementRepository::delete_all_for_share(&db, share.id)
            .await
            .unwrap();
        assert_eq!(
            ShareAnnouncementRepository::find_picture_by_token(&db, t1)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            ShareAnnouncementRepository::find_picture_by_token(&db, t2)
                .await
                .unwrap(),
            None
        );
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn find_downstream_for_pictures_returns_recipients(db: PgPool) {
        let owner = seed_user(&db).await;
        let pic = seed_picture(&db, owner).await;
        let share = OutgoingShareRepository::create(
            &db,
            owner,
            "Photos",
            "Test share",
            None,
            "carol",
            "carol.com",
            true,
            false,
            true,
            None,
        )
        .await
        .unwrap();
        ShareAnnouncementRepository::insert(&db, share.id, pic)
            .await
            .unwrap();

        let downstream = ShareAnnouncementRepository::find_downstream_for_pictures(&db, &[pic])
            .await
            .unwrap();
        assert_eq!(downstream.len(), 1);
        assert_eq!(downstream[0].recipient_username, "carol");
        // Owned picture (no remote_picture_id) → announce id falls back to the local id text.
        assert_eq!(downstream[0].announce_id, pic.to_string());
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn is_picture_tracked_reflects_membership(db: PgPool) {
        let owner = seed_user(&db).await;
        let pic = seed_picture(&db, owner).await;
        assert!(
            !ShareAnnouncementRepository::is_picture_tracked(&db, pic)
                .await
                .unwrap(),
            "untracked picture before any announce"
        );
        let share = OutgoingShareRepository::create(
            &db,
            owner,
            "Photos",
            "Test share",
            None,
            "bob",
            "other.com",
            true,
            false,
            true,
            None,
        )
        .await
        .unwrap();
        ShareAnnouncementRepository::insert(&db, share.id, pic)
            .await
            .unwrap();
        assert!(
            ShareAnnouncementRepository::is_picture_tracked(&db, pic)
                .await
                .unwrap(),
            "tracked after insert"
        );
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn find_stale_announcement_pictures_detects_lag(db: PgPool) {
        let owner = seed_user(&db).await;
        let pic = seed_picture(&db, owner).await;
        let share = OutgoingShareRepository::create(
            &db,
            owner,
            "Photos",
            "Test share",
            None,
            "bob",
            "other.com",
            true,
            false,
            true,
            None,
        )
        .await
        .unwrap();

        // Announce recording the picture's *current* updated_at → not stale.
        let now: chrono::NaiveDateTime =
            sqlx::query_scalar!("SELECT updated_at FROM pictures WHERE id = $1", pic)
                .fetch_one(&db)
                .await
                .unwrap();
        ShareAnnouncementRepository::insert_with_token(
            &db,
            share.id,
            pic,
            Uuid::new_v4(),
            Some(now),
        )
        .await
        .unwrap();
        assert!(
            ShareAnnouncementRepository::find_stale_announcement_pictures(&db)
                .await
                .unwrap()
                .is_empty(),
            "freshly announced picture is not stale"
        );

        // Bump the picture (the updated_at trigger moves it past announced_updated_at) → stale.
        sqlx::query!("UPDATE pictures SET blurhash = 'abc' WHERE id = $1", pic)
            .execute(&db)
            .await
            .unwrap();
        let stale = ShareAnnouncementRepository::find_stale_announcement_pictures(&db)
            .await
            .unwrap();
        assert_eq!(stale, vec![pic], "picture updated since announce is stale");
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn update_token_changes_resolution(db: PgPool) {
        let owner = seed_user(&db).await;
        let pic = seed_picture(&db, owner).await;
        let share = OutgoingShareRepository::create(
            &db,
            owner,
            "Photos",
            "Test share",
            None,
            "bob",
            "other.com",
            true,
            false,
            true,
            None,
        )
        .await
        .unwrap();

        let old = ShareAnnouncementRepository::insert(&db, share.id, pic)
            .await
            .unwrap();
        let new = Uuid::new_v4();
        ShareAnnouncementRepository::update_token(&db, share.id, pic, new)
            .await
            .unwrap();
        assert_eq!(
            ShareAnnouncementRepository::find_picture_by_token(&db, old)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            ShareAnnouncementRepository::find_picture_by_token(&db, new)
                .await
                .unwrap(),
            Some(pic)
        );
    }
}
