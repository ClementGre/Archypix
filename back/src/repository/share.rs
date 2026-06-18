use crate::domain::share::{IncomingShare, OutgoingShare, ShareStatus};
use crate::infra::error::{AppError, map_sqlx_error};
use chrono::NaiveDateTime;
use sqlx::{Executor, Postgres};
use uuid::Uuid;

pub struct OutgoingShareRepository;

impl OutgoingShareRepository {
    #[allow(clippy::too_many_arguments)]
    pub async fn create<'e, E>(
        ex: E,
        owner_id: Uuid,
        tag_path: &str,
        name: &str,
        message: Option<&str>,
        recipient_username: &str,
        recipient_instance: &str,
        allow_share_back: bool,
        future: bool,
    ) -> Result<OutgoingShare, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            OutgoingShare,
            r#"INSERT INTO outgoing_shares
                   (owner_id, tag_path, name, message, recipient_username, recipient_instance, allow_share_back, future)
               VALUES ($1, $2::text::ltree, $3, $4, $5, $6, $7, $8)
               RETURNING id, owner_id, tag_path::text as "tag_path!",
                         name, message,
                         recipient_username, recipient_instance,
                         allow_share_back, future,
                         status as "status: ShareStatus",
                         created_at, revoked_at"#,
            owner_id,
            tag_path,
            name,
            message,
            recipient_username,
            recipient_instance,
            allow_share_back,
            future,
        )
            .fetch_one(ex)
            .await
            .map_err(map_sqlx_error)
    }

    pub async fn get_by_id<'e, E>(ex: E, share_id: Uuid) -> Result<Option<OutgoingShare>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            OutgoingShare,
            r#"SELECT id, owner_id, tag_path::text as "tag_path!",
                      name, message,
                      recipient_username, recipient_instance,
                      allow_share_back, future,
                      status as "status: ShareStatus",
                      created_at, revoked_at
               FROM outgoing_shares WHERE id = $1"#,
            share_id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// List the owner's active shares that auto-announce new pictures (`future = true`).
    /// Used by the pipeline announcement step to compute current coverage.
    pub async fn list_active_future_by_owner<'e, E>(
        ex: E,
        owner_id: Uuid,
    ) -> Result<Vec<OutgoingShare>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            OutgoingShare,
            r#"SELECT id, owner_id, tag_path::text as "tag_path!",
                      name, message,
                      recipient_username, recipient_instance,
                      allow_share_back, future,
                      status as "status: ShareStatus",
                      created_at, revoked_at
               FROM outgoing_shares
               WHERE owner_id = $1 AND status = 'active'::share_status AND future = true"#,
            owner_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// List the owner's shares needing a full reconcile pass: those awaiting their first
    /// announcement (`pending_first_announcement`) or recovering from a failed delivery
    /// (`errored`), whose backoff window (`next_retry_at`) has elapsed. The pipeline reconciles each
    /// one's full coverage against the tracking table and flips it to `active` once fully delivered.
    pub async fn list_announceable_by_owner<'e, E>(
        ex: E,
        owner_id: Uuid,
        now: NaiveDateTime,
    ) -> Result<Vec<OutgoingShare>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            OutgoingShare,
            r#"SELECT id, owner_id, tag_path::text as "tag_path!",
                      name, message,
                      recipient_username, recipient_instance,
                      allow_share_back, future,
                      status as "status: ShareStatus",
                      created_at, revoked_at
               FROM outgoing_shares
               WHERE owner_id = $1
                 AND status IN ('pending_first_announcement'::share_status, 'errored'::share_status)
                 AND (next_retry_at IS NULL OR next_retry_at <= $2)"#,
            owner_id,
            now,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Find the owner's active shares whose `tag_path` is exactly or under `prefix` (an ltree
    /// path). Used by transitive revocation: when a directly re-shared `SharedToMe.*` tag is
    /// fully revoked upstream, the matching downstream shares are auto-revoked.
    pub async fn find_by_tag_prefix<'e, E>(
        ex: E,
        owner_id: Uuid,
        prefix_ltree: &str,
    ) -> Result<Vec<OutgoingShare>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            OutgoingShare,
            r#"SELECT id, owner_id, tag_path::text as "tag_path!",
                      name, message,
                      recipient_username, recipient_instance,
                      allow_share_back, future,
                      status as "status: ShareStatus",
                      created_at, revoked_at
               FROM outgoing_shares
               WHERE owner_id = $1
                 AND status = 'active'::share_status
                 AND tag_path <@ $2::text::ltree"#,
            owner_id,
            prefix_ltree,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    pub async fn set_status<'e, E>(
        ex: E,
        share_id: Uuid,
        status: ShareStatus,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"UPDATE outgoing_shares
               SET status = $2,
                   revoked_at = CASE WHEN $2 = 'revoked'::share_status
                                     THEN now() AT TIME ZONE 'utc'
                                     ELSE revoked_at
                                END
               WHERE id = $1"#,
            share_id,
            status as ShareStatus,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// A reconcile pass fully delivered the share: flip to `active` and clear the backoff.
    pub async fn mark_announce_success<'e, E>(ex: E, share_id: Uuid) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"UPDATE outgoing_shares
               SET status = 'active'::share_status, last_error_at = NULL, next_retry_at = NULL
               WHERE id = $1"#,
            share_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// A delivery failed: record the error and set the backoff. `demote_to_errored` is true when the
    /// share was `active` (the incremental path failed) — it moves to `errored` so the next pass is
    /// a full reconcile; otherwise the status (`pending_first_announcement`/`errored`) is kept.
    pub async fn mark_announce_failure<'e, E>(
        ex: E,
        share_id: Uuid,
        demote_to_errored: bool,
        next_retry_at: NaiveDateTime,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"UPDATE outgoing_shares
               SET status = CASE WHEN $2 THEN 'errored'::share_status ELSE status END,
                   last_error_at = now() AT TIME ZONE 'utc',
                   next_retry_at = $3
               WHERE id = $1"#,
            share_id,
            demote_to_errored,
            next_retry_at,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    pub async fn list_by_owner<'e, E>(ex: E, owner_id: Uuid) -> Result<Vec<OutgoingShare>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            OutgoingShare,
            r#"SELECT id, owner_id, tag_path::text as "tag_path!",
                      name, message,
                      recipient_username, recipient_instance,
                      allow_share_back, future,
                      status as "status: ShareStatus",
                      created_at, revoked_at
               FROM outgoing_shares WHERE owner_id = $1 ORDER BY created_at DESC"#,
            owner_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }
}

pub struct IncomingShareRepository;

impl IncomingShareRepository {
    #[allow(clippy::too_many_arguments)]
    pub async fn create<'e, E>(
        ex: E,
        recipient_id: Uuid,
        sender_username: &str,
        sender_instance: &str,
        name: &str,
        message: Option<&str>,
        outgoing_share_id: Uuid,
        allow_share_back: bool,
    ) -> Result<IncomingShare, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            IncomingShare,
            r#"INSERT INTO incoming_shares
                   (recipient_id, sender_username, sender_instance, name, message, outgoing_share_id, allow_share_back)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (recipient_id, sender_username, sender_instance, outgoing_share_id)
               DO UPDATE SET status = incoming_shares.status,
                             allow_share_back = EXCLUDED.allow_share_back,
                             name = EXCLUDED.name,
                             message = EXCLUDED.message
               RETURNING id, recipient_id, sender_username, sender_instance,
                         name, message, outgoing_share_id,
                         local_mapping_service_id,
                         status as "status: ShareStatus",
                         allow_share_back, created_at, revoked_at"#,
            recipient_id,
            sender_username,
            sender_instance,
            name,
            message,
            outgoing_share_id,
            allow_share_back,
        )
            .fetch_one(ex)
            .await
            .map_err(map_sqlx_error)
    }

    pub async fn list_by_recipient<'e, E>(
        ex: E,
        recipient_id: Uuid,
    ) -> Result<Vec<IncomingShare>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            IncomingShare,
            r#"SELECT id, recipient_id, sender_username, sender_instance,
                      name, message, outgoing_share_id,
                      local_mapping_service_id,
                      status as "status: ShareStatus",
                      allow_share_back, created_at, revoked_at
               FROM incoming_shares WHERE recipient_id = $1 ORDER BY created_at DESC"#,
            recipient_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    pub async fn set_status<'e, E>(
        ex: E,
        share_id: Uuid,
        status: ShareStatus,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"UPDATE incoming_shares
               SET status = $2,
                   revoked_at = CASE WHEN $2 = 'revoked'::share_status
                                     THEN now() AT TIME ZONE 'utc'
                                     ELSE revoked_at
                                END
               WHERE id = $1"#,
            share_id,
            status as ShareStatus,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    pub async fn get_by_id<'e, E>(ex: E, share_id: Uuid) -> Result<Option<IncomingShare>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            IncomingShare,
            r#"SELECT id, recipient_id, sender_username, sender_instance,
                      name, message, outgoing_share_id,
                      local_mapping_service_id,
                      status as "status: ShareStatus",
                      allow_share_back, created_at, revoked_at
               FROM incoming_shares WHERE id = $1"#,
            share_id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Find the incoming share for a given outgoing_share_id from a specific sender instance.
    /// Used by the recipient backend when the sender announces pictures after share acceptance.
    pub async fn find_by_outgoing_share<'e, E>(
        ex: E,
        outgoing_share_id: Uuid,
        sender_instance: &str,
    ) -> Result<Option<IncomingShare>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            IncomingShare,
            r#"SELECT id, recipient_id, sender_username, sender_instance,
                      name, message, outgoing_share_id,
                      local_mapping_service_id,
                      status as "status: ShareStatus",
                      allow_share_back, created_at, revoked_at
               FROM incoming_shares
               WHERE outgoing_share_id = $1 AND sender_instance = $2"#,
            outgoing_share_id,
            sender_instance,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Link an incoming share to the local `SharedTagMappingService` mapping created for it
    /// (ShareBack auto-accept). Stored so the frontend can surface the mapping.
    pub async fn set_local_mapping_service<'e, E>(
        ex: E,
        share_id: Uuid,
        service_id: Uuid,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"UPDATE incoming_shares SET local_mapping_service_id = $2 WHERE id = $1"#,
            share_id,
            service_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::share::ShareStatus;
    use sqlx::PgPool;
    use uuid::Uuid;

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

    // ── OutgoingShare ─────────────────────────────────────────────────────────

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn create_outgoing_share_defaults_to_pending(db: PgPool) {
        let owner = seed_user(&db).await;
        let share = OutgoingShareRepository::create(
            &db,
            owner,
            "Photos.Travel",
            "Test share",
            None,
            "bob",
            "other.com",
            true,
            true,
        )
        .await
        .unwrap();

        assert_eq!(share.status, ShareStatus::Pending);
        assert_eq!(share.owner_id, owner);
        assert_eq!(share.recipient_username, "bob");
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn set_status_outgoing_transitions_correctly(db: PgPool) {
        let owner = seed_user(&db).await;
        let share = OutgoingShareRepository::create(
            &db,
            owner,
            "Photos",
            "Test share",
            None,
            "bob",
            "other.com",
            true,
            true,
        )
        .await
        .unwrap();

        OutgoingShareRepository::set_status(&db, share.id, ShareStatus::Active)
            .await
            .unwrap();

        let updated = OutgoingShareRepository::get_by_id(&db, share.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, ShareStatus::Active);
    }

    // ── IncomingShare ─────────────────────────────────────────────────────────

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn create_incoming_share_defaults_to_pending(db: PgPool) {
        let sender = seed_user(&db).await;
        let recipient = seed_user(&db).await;
        let outgoing = OutgoingShareRepository::create(
            &db,
            sender,
            "Photos",
            "Test share",
            None,
            "recipient",
            "this.com",
            true,
            true,
        )
        .await
        .unwrap();

        let incoming = IncomingShareRepository::create(
            &db,
            recipient,
            "sender",
            "other.com",
            "Test share",
            None,
            outgoing.id,
            true,
        )
        .await
        .unwrap();

        assert_eq!(incoming.status, ShareStatus::Pending);
        assert_eq!(incoming.outgoing_share_id, outgoing.id);
        assert!(incoming.allow_share_back);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn find_by_outgoing_share_returns_correct_record(db: PgPool) {
        let sender = seed_user(&db).await;
        let recipient = seed_user(&db).await;
        let outgoing = OutgoingShareRepository::create(
            &db,
            sender,
            "Photos",
            "Test share",
            None,
            "recipient",
            "this.com",
            true,
            true,
        )
        .await
        .unwrap();

        IncomingShareRepository::create(
            &db,
            recipient,
            "sender",
            "other.com",
            "Test share",
            None,
            outgoing.id,
            false,
        )
        .await
        .unwrap();

        let found = IncomingShareRepository::find_by_outgoing_share(&db, outgoing.id, "other.com")
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().recipient_id, recipient);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn list_active_future_by_owner_filters_correctly(db: PgPool) {
        let owner = seed_user(&db).await;
        // active + future → included
        let s1 = OutgoingShareRepository::create(
            &db,
            owner,
            "Photos",
            "Test share",
            None,
            "bob",
            "other.com",
            true,
            true,
        )
        .await
        .unwrap();
        OutgoingShareRepository::set_status(&db, s1.id, ShareStatus::Active)
            .await
            .unwrap();
        // active but future=false → excluded
        let s2 = OutgoingShareRepository::create(
            &db,
            owner,
            "Images",
            "Test share",
            None,
            "bob",
            "other.com",
            true,
            false,
        )
        .await
        .unwrap();
        OutgoingShareRepository::set_status(&db, s2.id, ShareStatus::Active)
            .await
            .unwrap();
        // pending → excluded
        OutgoingShareRepository::create(
            &db,
            owner,
            "Docs",
            "Test share",
            None,
            "bob",
            "other.com",
            true,
            true,
        )
        .await
        .unwrap();

        let found = OutgoingShareRepository::list_active_future_by_owner(&db, owner)
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, s1.id);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn find_by_tag_prefix_matches_exact_and_descendants(db: PgPool) {
        let owner = seed_user(&db).await;
        let exact = OutgoingShareRepository::create(
            &db,
            owner,
            "SharedToMe.alice_AT_x.Travel",
            "Test share",
            None,
            "carol",
            "carol.com",
            true,
            true,
        )
        .await
        .unwrap();
        OutgoingShareRepository::set_status(&db, exact.id, ShareStatus::Active)
            .await
            .unwrap();
        let deeper = OutgoingShareRepository::create(
            &db,
            owner,
            "SharedToMe.alice_AT_x.Travel.France",
            "Test share",
            None,
            "carol",
            "carol.com",
            true,
            true,
        )
        .await
        .unwrap();
        OutgoingShareRepository::set_status(&db, deeper.id, ShareStatus::Active)
            .await
            .unwrap();
        // Unrelated tag → not matched
        let other = OutgoingShareRepository::create(
            &db,
            owner,
            "Photos.Holidays",
            "Test share",
            None,
            "carol",
            "carol.com",
            true,
            true,
        )
        .await
        .unwrap();
        OutgoingShareRepository::set_status(&db, other.id, ShareStatus::Active)
            .await
            .unwrap();

        let found =
            OutgoingShareRepository::find_by_tag_prefix(&db, owner, "SharedToMe.alice_AT_x.Travel")
                .await
                .unwrap();
        let ids: Vec<Uuid> = found.iter().map(|s| s.id).collect();
        assert!(ids.contains(&exact.id));
        assert!(ids.contains(&deeper.id));
        assert!(!ids.contains(&other.id));
    }
}
