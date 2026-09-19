use crate::MIGRATOR;
use crate::common::seed_user_bare;
use archypix_back::repository::share::*;
use archypix_back::domain::share::ShareStatus;
use sqlx::PgPool;
use uuid::Uuid;



// ── OutgoingShare ─────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_outgoing_share_defaults_to_pending(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    let share = OutgoingShareRepository::create(
        &db,
        owner,
        "Photos.Travel",
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

    assert_eq!(share.status, ShareStatus::Pending);
    assert_eq!(share.owner_id, owner);
    assert_eq!(share.recipient_username, "bob");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn set_status_outgoing_transitions_correctly(db: PgPool) {
    let owner = seed_user_bare(&db).await;
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

    OutgoingShareRepository::set_status(&db, share.id, ShareStatus::Active)
        .await
        .unwrap();

    let updated = OutgoingShareRepository::get_by_id(&db, share.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.status, ShareStatus::Active);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn set_status_stamps_closed_at_on_terminal(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    // Tombstoned (rejected) must record a close timestamp, just like revoked.
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
    assert!(share.revoked_at.is_none());

    OutgoingShareRepository::set_status(&db, share.id, ShareStatus::Tombstoned)
        .await
        .unwrap();
    let closed = OutgoingShareRepository::get_by_id(&db, share.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(closed.status, ShareStatus::Tombstoned);
    assert!(
        closed.revoked_at.is_some(),
        "tombstoned share must carry a close timestamp"
    );
}

// ── IncomingShare ─────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_incoming_share_defaults_to_pending(db: PgPool) {
    let sender = seed_user_bare(&db).await;
    let recipient = seed_user_bare(&db).await;
    let outgoing = OutgoingShareRepository::create(
        &db,
        sender,
        "Photos",
        "Test share",
        None,
        "recipient",
        "this.com",
        true,
        false,
        true,
        None,
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
        false,
        true,
        Some("SharedToMe.sender_AT_other_DOT_com.Photos"),
        None,
    )
    .await
    .unwrap();

    assert_eq!(incoming.status, ShareStatus::Pending);
    assert_eq!(incoming.outgoing_share_id, outgoing.id);
    assert!(incoming.allow_share_back);
    assert!(incoming.future);
    assert_eq!(
        incoming.shared_tag_path.as_deref(),
        Some("SharedToMe.sender_AT_other_DOT_com.Photos")
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn find_by_outgoing_share_returns_correct_record(db: PgPool) {
    let sender = seed_user_bare(&db).await;
    let recipient = seed_user_bare(&db).await;
    let outgoing = OutgoingShareRepository::create(
        &db,
        sender,
        "Photos",
        "Test share",
        None,
        "recipient",
        "this.com",
        true,
        false,
        true,
        None,
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
        false,
        false,
        None,
        None,
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
async fn list_active_by_owner_filters_correctly(db: PgPool) {
    let owner = seed_user_bare(&db).await;
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
        false,
        true,
        None,
    )
    .await
    .unwrap();
    OutgoingShareRepository::set_status(&db, s1.id, ShareStatus::Active)
        .await
        .unwrap();
    // active + future=false → included (still propagates metadata/deletion to tracked pictures)
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
        false,
        None,
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
        false,
        true,
        None,
    )
    .await
    .unwrap();

    let found = OutgoingShareRepository::list_active_by_owner(&db, owner)
        .await
        .unwrap();
    let ids: std::collections::HashSet<Uuid> = found.iter().map(|s| s.id).collect();
    assert_eq!(found.len(), 2);
    assert!(ids.contains(&s1.id), "active future=true share included");
    assert!(ids.contains(&s2.id), "active future=false share included");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn find_by_tag_prefix_matches_exact_and_descendants(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    let exact = OutgoingShareRepository::create(
        &db,
        owner,
        "SharedToMe.alice_AT_x.Travel",
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
        false,
        true,
        None,
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
        false,
        true,
        None,
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
