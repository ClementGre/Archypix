//! Integration tests for the "Better Sharing Support" feature, exercised on the same-backend
//! path (recipient_instance == global_domain) so no second server is needed. Cross-instance
//! variants are covered by the `federation` suite.
//!
//! Covered: future-picture announcement, per-picture unannounce, ShareBack auto-accept,
//! loop prevention, transitive sharing (Alice → Bob → Carol), and the per-picture token model.

mod common;

use archypix_back::domain::share::ShareStatus;
use archypix_back::infra::config::Config;
use archypix_back::infra::pipeline;
use archypix_back::infra::pipeline::PipelineWaker;
use archypix_back::infra::tasks::TaskQueue;
use archypix_back::repository::share::{IncomingShareRepository, OutgoingShareRepository};
use archypix_back::repository::tag::TagRepository;
use archypix_back::services::shares;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

// ── SharedToMe reserved-prefix protection (HTTP) ────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn patch_tags_rejects_shared_to_me_prefix(db: PgPool) {
    let cfg = config();
    let alice = common::seed_user(&db, "alice", "p").await;
    let pic = common::seed_picture(&db, alice).await;
    let token = common::federation::user_jwt(&cfg, "alice", alice);
    let app = archypix_back::api::routes(&cfg).with_state(common::test_app_state(db.clone(), &cfg));

    let body = serde_json::json!({
        "picture_ids": [pic],
        "add_tags": ["SharedToMe.fake"],
        "remove_tags": []
    });
    let req = Request::builder()
        .method("PATCH")
        .uri("/api/authenticated/tags")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn patch_tags_allows_normal_prefix(db: PgPool) {
    let cfg = config();
    let alice = common::seed_user(&db, "alice", "p").await;
    let pic = common::seed_picture(&db, alice).await;
    let token = common::federation::user_jwt(&cfg, "alice", alice);
    let app = archypix_back::api::routes(&cfg).with_state(common::test_app_state(db.clone(), &cfg));

    let body = serde_json::json!({
        "picture_ids": [pic],
        "add_tags": ["Photos.Travel"],
        "remove_tags": []
    });
    let req = Request::builder()
        .method("PATCH")
        .uri("/api/authenticated/tags")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

fn config() -> Config {
    Config::test_defaults()
}

/// Build the shared deps (cache, federation, task queue with spawned runner, pipeline notify).
async fn deps(db: &PgPool) -> (Config, Arc<common::InMemoryCache>, TaskQueue, PipelineWaker) {
    let config = config();
    let (fed, cache) = common::make_federation(&config);
    let (queue, notify) = common::test_task_queue(db, &config);
    // `make_federation` returns its own cache; reuse one cache for the share calls.
    let _ = fed;
    (config, cache, queue, notify)
}

/// Run the pipeline once for `user`. Delivery is inline (same-backend registers synchronously), so
/// no settle delay is needed. (`_queue` is kept for call-site symmetry with the share helpers.)
async fn run_pipeline_and_settle(db: &PgPool, _queue: &TaskQueue, config: &Config, user: Uuid) {
    let (fed, cache) = common::make_federation(config);
    let waker = PipelineWaker::disconnected();
    pipeline::run_once_for_user(db, &fed, cache.as_ref(), config, &waker, user)
        .await
        .unwrap();
}

/// Create an active same-backend share of `tag` from `sender` to `recipient`, accept it, and run
/// the sender's pipeline so its current pictures are announced (the initial announce is
/// pipeline-driven via the `pending_first_announcement` status). Returns the OutgoingShare id.
#[allow(clippy::too_many_arguments)]
async fn active_share(
    db: &PgPool,
    config: &Config,
    cache: &Arc<common::InMemoryCache>,
    queue: &TaskQueue,
    notify: &PipelineWaker,
    sender_id: Uuid,
    sender_name: &str,
    recipient_id: Uuid,
    recipient_name: &str,
    tag: &str,
    future: bool,
) -> Uuid {
    let (fed, _c) = common::make_federation(config);
    let share = shares::create_outgoing_share(
        db,
        cache.as_ref(),
        &fed,
        config,
        notify,
        sender_id,
        sender_name,
        tag,
        "Test share",
        None,
        recipient_name,
        &config.global_domain,
        true,
        false,
        future,
        None,
    )
    .await
    .unwrap();
    let incoming =
        IncomingShareRepository::find_by_outgoing_share(db, share.id, &config.global_domain)
            .await
            .unwrap()
            .unwrap();
    shares::accept_incoming_share(
        db,
        cache.as_ref(),
        &fed,
        config,
        notify,
        recipient_id,
        recipient_name,
        incoming.id,
    )
    .await
    .unwrap();
    // Initial announcement: sender's OutgoingShare is now `pending_first_announcement`; the
    // pipeline announces its coverage and flips it to Active.
    run_pipeline_and_settle(db, queue, config, sender_id).await;
    share.id
}

// ── Future-picture announcement ─────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn future_picture_added_after_accept_is_announced(db: PgPool) {
    let (config, cache, queue, notify) = deps(&db).await;
    let alice = common::seed_user(&db, "alice", "p").await;
    let bob = common::seed_user(&db, "bob", "p").await;

    // Active future share, no pictures yet.
    active_share(
        &db, &config, &cache, &queue, &notify, alice, "alice", bob, "bob", "Travel", true,
    )
    .await;
    assert_eq!(common::count_received_pictures(&db, bob).await, 0);

    // Alice adds a picture to the shared tag → pipeline announces it to Bob.
    common::seed_picture_with_tag(&db, alice, "Travel").await;
    run_pipeline_and_settle(&db, &queue, &config, alice).await;

    assert_eq!(
        common::count_received_pictures(&db, bob).await,
        1,
        "future picture must be announced to Bob"
    );
    let expected = common::shared_to_me_tag("alice", &config.global_domain, "Travel");
    assert!(
        common::received_picture_tags(&db, bob)
            .await
            .contains(&expected)
    );

    // The announcement stamps the incoming share with its timestamp and the (advisory) local
    // SharedToMe tag the pictures landed under.
    let bob_incoming = IncomingShareRepository::list_by_recipient(&db, bob)
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.sender_username == "alice")
        .expect("Bob must have an incoming share from Alice");
    assert!(
        bob_incoming.last_announcement_received_at.is_some(),
        "announcement must stamp last_announcement_received_at"
    );
    assert_eq!(
        bob_incoming.shared_tag_path.as_deref(),
        Some(expected.as_str()),
        "announcement must refresh the advisory shared_tag_path"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn future_false_share_does_not_announce_new_pictures(db: PgPool) {
    let (config, cache, queue, notify) = deps(&db).await;
    let alice = common::seed_user(&db, "alice", "p").await;
    let bob = common::seed_user(&db, "bob", "p").await;

    // future = false.
    active_share(
        &db, &config, &cache, &queue, &notify, alice, "alice", bob, "bob", "Travel", false,
    )
    .await;

    common::seed_picture_with_tag(&db, alice, "Travel").await;
    run_pipeline_and_settle(&db, &queue, &config, alice).await;

    assert_eq!(
        common::count_received_pictures(&db, bob).await,
        0,
        "future=false share must not announce new pictures"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn removing_tag_unannounces_picture(db: PgPool) {
    let (config, cache, queue, notify) = deps(&db).await;
    let alice = common::seed_user(&db, "alice", "p").await;
    let bob = common::seed_user(&db, "bob", "p").await;

    active_share(
        &db, &config, &cache, &queue, &notify, alice, "alice", bob, "bob", "Travel", true,
    )
    .await;
    let pic = common::seed_picture_with_tag(&db, alice, "Travel").await;
    run_pipeline_and_settle(&db, &queue, &config, alice).await;
    assert_eq!(common::count_received_pictures(&db, bob).await, 1);

    // Remove the tag → picture leaves coverage → unannounce.
    TagRepository::batch_remove(&db, alice, &[pic], &["Travel".to_string()])
        .await
        .unwrap();
    archypix_back::repository::pipeline::PipelineRepository::invalidate(&db, &[pic])
        .await
        .unwrap();
    run_pipeline_and_settle(&db, &queue, &config, alice).await;

    assert_eq!(
        common::count_received_pictures(&db, bob).await,
        0,
        "picture must be unannounced after leaving coverage"
    );
}

// ── Loop prevention ─────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn loop_prevention_does_not_reannounce_recipient_owned_picture(db: PgPool) {
    let (config, cache, queue, notify) = deps(&db).await;
    let alice = common::seed_user(&db, "alice", "p").await;
    let bob = common::seed_user(&db, "bob", "p").await;

    // Alice shares Travel to Bob (future), and Bob's OWN picture happens to carry Travel after
    // being received... simulate by giving Alice a received picture owned by Bob under Travel.
    // Insert a received picture on Alice owned by bob@global, tagged Travel.
    let pic = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO pictures (id, local_user_id, remote_picture_id, owner_username, owner_instance_domain)
         VALUES ($1, $2, $3, 'bob', $4)",
        pic,
        alice,
        Uuid::new_v4().to_string(),
        config.global_domain,
    )
        .execute(&db)
        .await
        .unwrap();
    TagRepository::batch_assign(&db, alice, &[pic], &["Travel".to_string()])
        .await
        .unwrap();

    active_share(
        &db, &config, &cache, &queue, &notify, alice, "alice", bob, "bob", "Travel", true,
    )
    .await;
    run_pipeline_and_settle(&db, &queue, &config, alice).await;

    // Bob must NOT receive his own picture back.
    assert_eq!(
        common::count_received_pictures(&db, bob).await,
        0,
        "loop prevention: Bob's own picture must not be announced back to Bob"
    );
}

// ── ShareBack ───────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn shareback_same_backend_auto_accepts_and_maps(db: PgPool) {
    let (config, cache, queue, notify) = deps(&db).await;
    let alice = common::seed_user(&db, "alice", "p").await;
    let bob = common::seed_user(&db, "bob", "p").await;
    let (fed, _c) = common::make_federation(&config);

    // Alice shares Travel → Bob with allow_share_back = true.
    let alice_share = shares::create_outgoing_share(
        &db,
        cache.as_ref(),
        &fed,
        &config,
        &notify,
        alice,
        "alice",
        "Travel",
        "Test share",
        None,
        "bob",
        &config.global_domain,
        true,
        false,
        true,
        None,
    )
    .await
    .unwrap();

    // Bob shares one of his own pictures back to Alice, referencing Alice's share.
    common::seed_picture_with_tag(&db, bob, "BobPhotos").await;
    shares::create_outgoing_share(
        &db,
        cache.as_ref(),
        &fed,
        &config,
        &notify,
        bob,
        "bob",
        "BobPhotos",
        "Test share",
        None,
        "alice",
        &config.global_domain,
        true,
        false,
        true,
        Some(alice_share.id),
    )
    .await
    .unwrap();

    // Bob's pictures are announced to Alice by Bob's pipeline (his OutgoingShare is now
    // `pending_first_announcement`).
    run_pipeline_and_settle(&db, &queue, &config, bob).await;

    // Alice's IncomingShare from Bob must be Active (auto-accepted), with a mapping service.
    let alice_incomings = IncomingShareRepository::list_by_recipient(&db, alice)
        .await
        .unwrap();
    let from_bob = alice_incomings
        .iter()
        .find(|i| i.sender_username == "bob")
        .expect("Alice must have an incoming share from Bob");
    assert_eq!(
        from_bob.status,
        ShareStatus::Active,
        "ShareBack must auto-accept"
    );
    assert!(
        from_bob.local_mapping_service_id.is_some(),
        "a SharedTagMappingService mapping rule must be linked"
    );
    assert_eq!(
        from_bob.shareback_of,
        Some(alice_share.id),
        "incoming ShareBack must record the original outgoing share it answers"
    );

    // Bob's outgoing ShareBack must record the same provenance for his own UI.
    let bob_shareback = OutgoingShareRepository::list_by_owner(&db, bob)
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.recipient_username == "alice")
        .expect("Bob must have an outgoing share to Alice");
    assert_eq!(
        bob_shareback.shareback_of,
        Some(alice_share.id),
        "outgoing ShareBack must record the original outgoing share it answers"
    );

    // The auto-mapping reintegrates the shared-back pictures into Alice's *original* shared tag
    // (§7.3: assign_tag = original outgoing share's tag_path).
    let mapping_assign: Option<String> = sqlx::query_scalar(
        "SELECT assign_tag::text FROM shared_tag_mapping_services WHERE incoming_share_id = $1",
    )
    .bind(from_bob.id)
    .fetch_optional(&db)
    .await
    .unwrap();
    assert_eq!(
        mapping_assign.as_deref(),
        Some("Travel"),
        "mapping must assign Alice's original shared tag"
    );

    // Bob's picture is registered on Alice's side.
    assert_eq!(common::count_received_pictures(&db, alice).await, 1);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn shareback_disallowed_stays_pending(db: PgPool) {
    let (config, cache, _queue, notify) = deps(&db).await;
    let alice = common::seed_user(&db, "alice", "p").await;
    let bob = common::seed_user(&db, "bob", "p").await;
    let (fed, _c) = common::make_federation(&config);

    // Alice shares with allow_share_back = false.
    let alice_share = shares::create_outgoing_share(
        &db,
        cache.as_ref(),
        &fed,
        &config,
        &notify,
        alice,
        "alice",
        "Travel",
        "Test share",
        None,
        "bob",
        &config.global_domain,
        false,
        false,
        true,
        None,
    )
    .await
    .unwrap();

    common::seed_picture_with_tag(&db, bob, "BobPhotos").await;
    shares::create_outgoing_share(
        &db,
        cache.as_ref(),
        &fed,
        &config,
        &notify,
        bob,
        "bob",
        "BobPhotos",
        "Test share",
        None,
        "alice",
        &config.global_domain,
        true,
        false,
        true,
        Some(alice_share.id),
    )
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    let alice_incomings = IncomingShareRepository::list_by_recipient(&db, alice)
        .await
        .unwrap();
    let from_bob = alice_incomings
        .iter()
        .find(|i| i.sender_username == "bob")
        .unwrap();
    assert_eq!(
        from_bob.status,
        ShareStatus::Pending,
        "ShareBack with allow_share_back=false must NOT auto-accept"
    );
    assert!(from_bob.local_mapping_service_id.is_none());
}

// ── Transitive sharing (Alice → Bob → Carol) ────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn transitive_share_propagates_and_forwards_token(db: PgPool) {
    let (config, cache, queue, notify) = deps(&db).await;
    let alice = common::seed_user(&db, "alice", "p").await;
    let bob = common::seed_user(&db, "bob", "p").await;
    let carol = common::seed_user(&db, "carol", "p").await;

    // Alice shares Travel → Bob, with a picture already present.
    let pic = common::seed_picture_with_tag(&db, alice, "Travel").await;
    active_share(
        &db, &config, &cache, &queue, &notify, alice, "alice", bob, "bob", "Travel", true,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(common::count_received_pictures(&db, bob).await, 1);

    // Bob re-shares the SharedToMe.alice.Travel tag → Carol.
    let bob_tag = common::shared_to_me_tag("alice", &config.global_domain, "Travel");
    active_share(
        &db, &config, &cache, &queue, &notify, bob, "bob", carol, "carol", &bob_tag, true,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Carol receives the picture, owned by Alice (original owner), with Alice's forwarded token.
    assert_eq!(common::count_received_pictures(&db, carol).await, 1);
    let carol_owner: Option<String> = sqlx::query_scalar(
        "SELECT owner_username FROM pictures WHERE local_user_id = $1 AND remote_picture_id IS NOT NULL",
    )
        .bind(carol)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(
        carol_owner.as_deref(),
        Some("alice"),
        "Carol must fetch from Alice directly"
    );

    // Carol's forwarded token resolves to Alice's picture on Alice's share_announcements.
    let carol_pic: Uuid = sqlx::query_scalar(
        "SELECT id FROM pictures WHERE local_user_id = $1 AND remote_picture_id IS NOT NULL",
    )
    .bind(carol)
    .fetch_one(&db)
    .await
    .unwrap();
    let token = TagRepository::find_active_picture_token(&db, carol_pic)
        .await
        .unwrap()
        .expect("Carol's received picture must carry a forwarded token");
    let resolved =
        archypix_back::repository::share_announcement::ShareAnnouncementRepository::find_picture_by_token(&db, token)
            .await
            .unwrap();
    assert_eq!(
        resolved,
        Some(pic),
        "forwarded token must resolve to Alice's original picture"
    );
}

// ── Transitive revocation (direct SharedToMe re-share) ──────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn transitive_revocation_cascades_to_carol(db: PgPool) {
    let (config, cache, queue, notify) = deps(&db).await;
    let alice = common::seed_user(&db, "alice", "p").await;
    let bob = common::seed_user(&db, "bob", "p").await;
    let carol = common::seed_user(&db, "carol", "p").await;
    let (fed, _c) = common::make_federation(&config);

    common::seed_picture_with_tag(&db, alice, "Travel").await;
    let alice_share = active_share(
        &db, &config, &cache, &queue, &notify, alice, "alice", bob, "bob", "Travel", true,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;

    let bob_tag = common::shared_to_me_tag("alice", &config.global_domain, "Travel");
    let bob_share = active_share(
        &db, &config, &cache, &queue, &notify, bob, "bob", carol, "carol", &bob_tag, true,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(common::count_received_pictures(&db, carol).await, 1);

    // Alice revokes → Bob's received picture deleted → Bob's share to Carol auto-revoked.
    shares::revoke_outgoing_share(
        &db,
        cache.as_ref(),
        &fed,
        &config,
        &queue,
        &notify,
        alice,
        "alice",
        alice_share,
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;

    assert_eq!(
        common::count_received_pictures(&db, bob).await,
        0,
        "Bob's picture gone"
    );
    let bob_out = OutgoingShareRepository::get_by_id(&db, bob_share)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bob_out.status,
        ShareStatus::Revoked,
        "Bob's downstream share must be auto-revoked"
    );
    assert_eq!(
        common::count_received_pictures(&db, carol).await,
        0,
        "Carol's picture gone"
    );
}
