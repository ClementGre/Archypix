//! End-to-end federation protocol tests.
//!
//! Two real Axum servers share a `PgPool` (logically partitioned by `global_domain`).
//! Tests call service functions directly — A's `FederationClient` makes real TCP calls
//! to B, and vice versa. The test itself never constructs HTTP requests.

use crate::common;

use archypix_back::clients::federation::FederationClient;
use archypix_back::domain::share::ShareStatus;
use archypix_back::infra::config::Config;
use archypix_back::infra::pipeline::PipelineWaker;
use archypix_back::repository::share::{IncomingShareRepository, OutgoingShareRepository};
use archypix_back::services::shares::{
    accept_incoming_share, create_outgoing_share, reject_incoming_share, revoke_outgoing_share,
};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

pub(crate) static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
/// Run the sender's pipeline so a `pending_first_announcement` share announces its pictures. The
/// pipeline delivers cross-instance announcements inline via `fed` (which resolves the recipient
/// backend from its pre-seeded cache), so the round-trip completes synchronously.
async fn settle_sender(db: &PgPool, cfg: &Config, fed: &FederationClient, sender_id: Uuid) {
    let cache = std::sync::Arc::new(common::InMemoryCache::new());
    let waker = PipelineWaker::disconnected();
    archypix_back::infra::pipeline::run_once_for_user(
        db,
        fed,
        cache.as_ref(),
        cfg,
        &waker,
        sender_id,
    )
    .await
    .unwrap();
}

/// A throwaway pipeline notify for accept calls whose same-backend wake path is unused.
fn dummy_notify() -> PipelineWaker {
    PipelineWaker::disconnected()
}

// ── Setup ─────────────────────────────────────────────────────────────────────

/// Spawn both backends, wire their backend-URL caches to bypass WebFinger,
/// and seed alice (on A) and bob (on B).
///
/// Returns `(cache_a, cfg_a, cache_b, cfg_b, alice_id, bob_id)`.
async fn spawn_pair(
    db: PgPool,
) -> (
    Arc<common::InMemoryCache>,
    Config,
    Arc<common::InMemoryCache>,
    Config,
    Uuid,
    Uuid,
) {
    let (addr_a, cache_a, cfg_a) =
        common::federation::spawn_backend(db.clone(), common::federation::config_a()).await;
    let (addr_b, cache_b, cfg_b) =
        common::federation::spawn_backend(db.clone(), common::federation::config_b()).await;

    common::federation::seed_backend_url(&cache_a, "bob", "b.test", &format!("http://{addr_b}"))
        .await;
    common::federation::seed_backend_url(&cache_b, "alice", "a.test", &format!("http://{addr_a}"))
        .await;

    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let bob_id = common::seed_user(&db, "bob", "pass").await;

    (cache_a, cfg_a, cache_b, cfg_b, alice_id, bob_id)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// A's `FederationClient::get_or_wait_federation_token` drives the full handshake:
///   A → auth/request on B → B issues JWT → B → auth/grant on A → token in A's cache.
#[sqlx::test(migrator = "MIGRATOR")]
async fn auth_handshake_grants_token_to_requester(db: PgPool) {
    let (cache_a, cfg_a, _, _, _, _) = spawn_pair(db).await;
    let fed_a = common::federation::make_client(&cfg_a, &cache_a);

    let token = fed_a
        .get_or_wait_federation_token("alice", "bob", "b.test")
        .await
        .expect("handshake must complete and return a token");

    assert!(
        !token.is_empty(),
        "returned federation token must be non-empty"
    );
}

/// An `auth/grant` with no matching pending request nonce is rejected (token-cache poisoning
/// guard, 07_security_audit.md §2.1); a grant that echoes the stored nonce is accepted.
#[sqlx::test(migrator = "MIGRATOR")]
async fn auth_grant_rejects_unsolicited_nonce(db: PgPool) {
    let (cache_a, cfg_a, _, _, _, _) = spawn_pair(db).await;
    let fed_a = common::federation::make_client(&cfg_a, &cache_a);

    // No pending nonce stored for "b.test" → unsolicited grant rejected.
    let unsolicited = fed_a
        .store_federation_token("b.test", "attacker-token", 60, "bogus-nonce")
        .await;
    assert!(
        matches!(
            unsolicited,
            Err(archypix_back::infra::error::AppError::Unauthorized(_))
        ),
        "unsolicited grant must be rejected, got {unsolicited:?}"
    );

    // With a matching pending nonce, the grant is accepted.
    common::federation::seed_auth_nonce(&cache_a, "b.test", "the-nonce").await;
    fed_a
        .store_federation_token("b.test", "legit-token", 60, "the-nonce")
        .await
        .expect("grant matching the pending nonce must be accepted");
}

/// Alice creates a share → Bob gets a Pending IncomingShare → Bob accepts →
/// A marks the OutgoingShare Active and announces pictures → Bob has the picture.
#[sqlx::test(migrator = "MIGRATOR")]
async fn cross_instance_share_announce_and_accept_propagates_pictures(db: PgPool) {
    let (cache_a, cfg_a, cache_b, cfg_b, alice_id, bob_id) = spawn_pair(db.clone()).await;
    let fed_a = common::federation::make_client(&cfg_a, &cache_a);
    let fed_b = common::federation::make_client(&cfg_b, &cache_b);

    common::seed_picture_with_tag(&db, alice_id, "vacation").await;

    let share = create_outgoing_share(
        &db,
        &*cache_a,
        &fed_a,
        &cfg_a,
        &PipelineWaker::disconnected(),
        alice_id,
        "alice",
        "vacation",
        "Test share",
        None,
        "bob",
        "b.test",
        false,
        false,
        None,
    )
    .await
    .unwrap();

    let bob_incoming = IncomingShareRepository::list_by_recipient(&db, bob_id)
        .await
        .unwrap();
    assert_eq!(bob_incoming.len(), 1);
    assert_eq!(bob_incoming[0].status, ShareStatus::Pending);
    assert_eq!(bob_incoming[0].outgoing_share_id, share.id);

    accept_incoming_share(
        &db,
        &*cache_b,
        &fed_b,
        &cfg_b,
        &dummy_notify(),
        bob_id,
        "bob",
        bob_incoming[0].id,
    )
    .await
    .unwrap();

    // Alice's OutgoingShare is now `pending_first_announcement`; run her pipeline to announce.
    settle_sender(&db, &cfg_a, &fed_a, alice_id).await;

    let outgoing = OutgoingShareRepository::get_by_id(&db, share.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outgoing.status, ShareStatus::Active);

    assert_eq!(common::count_received_pictures(&db, bob_id).await, 1);
    let tags = common::received_picture_tags(&db, bob_id).await;
    let expected = common::shared_to_me_tag("alice", "a.test", "vacation");
    assert!(tags.contains(&expected), "got: {tags:?}");
}

/// After the full announce + accept + pictures flow, Alice revokes.
/// Bob's received pictures must be deleted.
#[sqlx::test(migrator = "MIGRATOR")]
async fn cross_instance_revoke_removes_received_pictures(db: PgPool) {
    let (cache_a, cfg_a, cache_b, cfg_b, alice_id, bob_id) = spawn_pair(db.clone()).await;
    let fed_a = common::federation::make_client(&cfg_a, &cache_a);
    let fed_b = common::federation::make_client(&cfg_b, &cache_b);

    common::seed_picture_with_tag(&db, alice_id, "vacation").await;

    let share = create_outgoing_share(
        &db,
        &*cache_a,
        &fed_a,
        &cfg_a,
        &PipelineWaker::disconnected(),
        alice_id,
        "alice",
        "vacation",
        "Test share",
        None,
        "bob",
        "b.test",
        false,
        false,
        None,
    )
    .await
    .unwrap();

    let incoming_id = IncomingShareRepository::list_by_recipient(&db, bob_id)
        .await
        .unwrap()[0]
        .id;

    accept_incoming_share(
        &db,
        &*cache_b,
        &fed_b,
        &cfg_b,
        &dummy_notify(),
        bob_id,
        "bob",
        incoming_id,
    )
    .await
    .unwrap();

    settle_sender(&db, &cfg_a, &fed_a, alice_id).await;
    assert_eq!(
        common::count_received_pictures(&db, bob_id).await,
        1,
        "precondition"
    );

    let (tq_a, notify_a) = common::test_task_queue_with_federation(&db, &cfg_a, fed_a.clone());
    revoke_outgoing_share(
        &db, &*cache_a, &fed_a, &cfg_a, &tq_a, &notify_a, alice_id, "alice", share.id,
    )
    .await
    .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    let outgoing = OutgoingShareRepository::get_by_id(&db, share.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outgoing.status, ShareStatus::Revoked);
    assert_eq!(common::count_received_pictures(&db, bob_id).await, 0);
}

/// Alice announces a share, Bob accepts and receives pictures, then Bob rejects it
/// Expected outcome: received pictures deleted, both shares Tombstoned.
#[sqlx::test(migrator = "MIGRATOR")]
async fn cross_instance_reject_active_share_cleans_up_received_pictures(db: PgPool) {
    let (cache_a, cfg_a, cache_b, cfg_b, alice_id, bob_id) = spawn_pair(db.clone()).await;
    let fed_a = common::federation::make_client(&cfg_a, &cache_a);
    let fed_b = common::federation::make_client(&cfg_b, &cache_b);

    common::seed_picture_with_tag(&db, alice_id, "vacation").await;

    // Bring the share to Active with pictures propagated (same setup as the revoke test).
    let share = create_outgoing_share(
        &db,
        &*cache_a,
        &fed_a,
        &cfg_a,
        &PipelineWaker::disconnected(),
        alice_id,
        "alice",
        "vacation",
        "Test share",
        None,
        "bob",
        "b.test",
        false,
        false,
        None,
    )
    .await
    .unwrap();

    let incoming_id = IncomingShareRepository::list_by_recipient(&db, bob_id)
        .await
        .unwrap()[0]
        .id;

    accept_incoming_share(
        &db,
        &*cache_b,
        &fed_b,
        &cfg_b,
        &dummy_notify(),
        bob_id,
        "bob",
        incoming_id,
    )
    .await
    .unwrap();

    settle_sender(&db, &cfg_a, &fed_a, alice_id).await;
    assert_eq!(
        common::count_received_pictures(&db, bob_id).await,
        1,
        "precondition"
    );

    // Bob rejects the now-Active share — triggers cleanup then federation notify.
    let (tq_b, notify_b) = common::test_task_queue(&db, &cfg_b);
    reject_incoming_share(
        &db,
        &*cache_b,
        &fed_b,
        &cfg_b,
        &tq_b,
        &notify_b,
        bob_id,
        "bob",
        incoming_id,
    )
    .await
    .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    // Bob's received pictures are gone.
    assert_eq!(common::count_received_pictures(&db, bob_id).await, 0);

    // Bob's IncomingShare is Tombstoned.
    let bob_incoming = IncomingShareRepository::list_by_recipient(&db, bob_id)
        .await
        .unwrap();
    assert_eq!(bob_incoming[0].status, ShareStatus::Tombstoned);

    // Alice's OutgoingShare is Tombstoned.
    let outgoing = OutgoingShareRepository::get_by_id(&db, share.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outgoing.status, ShareStatus::Tombstoned);
}

/// Cross-instance ShareBack must not deadlock (the recipient does **not** call back into the
/// initiator's still-open share-creation transaction; rule 2 — it returns `auto_accepted` and the
/// initiator announces its own pictures inline). Alice shares to Bob with `allow_share_back`; Bob
/// shares a tag of his own back to Alice referencing her share. Alice auto-accepts and receives
/// Bob's picture without any further round-trip.
#[sqlx::test(migrator = "MIGRATOR")]
async fn cross_instance_shareback_auto_accepts_without_deadlock(db: PgPool) {
    let (cache_a, cfg_a, cache_b, cfg_b, alice_id, bob_id) = spawn_pair(db.clone()).await;
    let fed_a = common::federation::make_client(&cfg_a, &cache_a);
    let fed_b = common::federation::make_client(&cfg_b, &cache_b);
    let notify = PipelineWaker::disconnected();

    // Alice shares "vacation" to Bob with ShareBack allowed.
    common::seed_picture_with_tag(&db, alice_id, "vacation").await;
    let alice_share = create_outgoing_share(
        &db,
        &*cache_a,
        &fed_a,
        &cfg_a,
        &notify,
        alice_id,
        "alice",
        "vacation",
        "Test share",
        None,
        "bob",
        "b.test",
        true,  // allow_share_back
        false, // future
        None,
    )
    .await
    .unwrap();

    // Bob shares his own tag "mystuff" back to Alice, referencing Alice's share.
    common::seed_picture_with_tag(&db, bob_id, "mystuff").await;
    let bob_share = create_outgoing_share(
        &db,
        &*cache_b,
        &fed_b,
        &cfg_b,
        &notify,
        bob_id,
        "bob",
        "mystuff",
        "Test share",
        None,
        "alice",
        "a.test",
        false,
        false,
        Some(alice_share.id), // shareback_of references Alice's original OutgoingShare
    )
    .await
    .expect("cross-instance ShareBack must complete (no deadlock)");

    // Bob's OutgoingShare is now `pending_first_announcement`; his pipeline announces his pictures
    // to Alice and flips the share to Active.
    settle_sender(&db, &cfg_b, &fed_b, bob_id).await;

    let bob_outgoing = OutgoingShareRepository::get_by_id(&db, bob_share.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bob_outgoing.status, ShareStatus::Active);

    // Alice auto-accepted: her IncomingShare from Bob is Active with a linked mapping service.
    let alice_incoming: Vec<_> = IncomingShareRepository::list_by_recipient(&db, alice_id)
        .await
        .unwrap()
        .into_iter()
        .filter(|s| s.sender_username == "bob")
        .collect();
    assert_eq!(
        alice_incoming.len(),
        1,
        "Alice has one incoming share from Bob"
    );
    assert_eq!(alice_incoming[0].status, ShareStatus::Active);
    assert!(
        alice_incoming[0].local_mapping_service_id.is_some(),
        "ShareBack auto-accept must wire up a SharedTagMappingService mapping"
    );

    // Alice received Bob's picture under the SharedToMe.bob tag.
    assert_eq!(common::count_received_pictures(&db, alice_id).await, 1);
    let tags = common::received_picture_tags(&db, alice_id).await;
    let expected = common::shared_to_me_tag("bob", "b.test", "mystuff");
    assert!(tags.contains(&expected), "got: {tags:?}");
}

/// Alice announces a share; Bob rejects it.
/// Alice's OutgoingShare must be Tombstoned.
#[sqlx::test(migrator = "MIGRATOR")]
async fn cross_instance_reject_tombstones_outgoing_share(db: PgPool) {
    let (cache_a, cfg_a, cache_b, cfg_b, alice_id, bob_id) = spawn_pair(db.clone()).await;
    let fed_a = common::federation::make_client(&cfg_a, &cache_a);
    let fed_b = common::federation::make_client(&cfg_b, &cache_b);

    let share = create_outgoing_share(
        &db,
        &*cache_a,
        &fed_a,
        &cfg_a,
        &PipelineWaker::disconnected(),
        alice_id,
        "alice",
        "vacation",
        "Test share",
        None,
        "bob",
        "b.test",
        false,
        false,
        None,
    )
    .await
    .unwrap();

    let incoming_id = IncomingShareRepository::list_by_recipient(&db, bob_id)
        .await
        .unwrap()[0]
        .id;

    let (tq_b, notify_b) = common::test_task_queue(&db, &cfg_b);
    reject_incoming_share(
        &db,
        &*cache_b,
        &fed_b,
        &cfg_b,
        &tq_b,
        &notify_b,
        bob_id,
        "bob",
        incoming_id,
    )
    .await
    .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    let bob_incoming = IncomingShareRepository::list_by_recipient(&db, bob_id)
        .await
        .unwrap();
    assert_eq!(bob_incoming[0].status, ShareStatus::Tombstoned);

    let outgoing = OutgoingShareRepository::get_by_id(&db, share.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outgoing.status, ShareStatus::Tombstoned);
    assert_eq!(common::count_received_pictures(&db, bob_id).await, 0);
}
