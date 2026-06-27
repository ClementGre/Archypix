mod common;

use archypix_back::clients::federation::models::AnnouncedPicture;
use archypix_back::domain::share::ShareStatus;
use archypix_back::infra::config::Config;
use archypix_back::infra::routine::RoutineHandle;
use archypix_back::infra::routine::pipeline;
use archypix_back::repository::share::{IncomingShareRepository, OutgoingShareRepository};
use archypix_back::services::shares;
use sqlx::PgPool;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

// ── helpers ───────────────────────────────────────────────────────────────────

fn config() -> Config {
    Config::test_defaults()
}

/// Accept a same-backend incoming share, then drive the sender's pipeline so its pictures are
/// announced (the initial announce is pipeline-driven: accept moves the OutgoingShare to
/// `pending_first_announcement`, the pipeline announces its coverage and flips it to `active`,
/// and the spawned task runner registers the received pictures).
async fn accept_and_announce(
    db: &PgPool,
    config: &Config,
    acceptor_id: uuid::Uuid,
    acceptor_name: &str,
    incoming_id: uuid::Uuid,
    sender_id: uuid::Uuid,
) {
    let (fed, cache) = common::make_federation(config);
    let (_queue, notify) = common::test_task_queue(db, config);
    shares::accept_incoming_share(
        db,
        cache.as_ref(),
        &fed,
        config,
        &notify,
        acceptor_id,
        acceptor_name,
        incoming_id,
    )
    .await
    .unwrap();
    pipeline::run_once_for_user(db, &fed, cache.as_ref(), config, &notify, sender_id)
        .await
        .unwrap();
}

/// Share Alice's `tag_path` with Bob (same backend — recipient_instance = global_domain).
async fn alice_shares_with_bob(
    db: &PgPool,
    alice_id: uuid::Uuid,
    tag_path: &str,
) -> archypix_back::domain::share::OutgoingShare {
    let config = config();
    let (fed, cache) = common::make_federation(&config);
    let notify = archypix_back::infra::routine::RoutineHandle::<uuid::Uuid>::disconnected();

    shares::create_outgoing_share(
        db,
        cache.as_ref(),
        &fed,
        &config,
        &notify,
        alice_id,
        "alice",
        tag_path,
        "Test share",
        None,
        "bob",
        "test.com", // same as global_domain → same-backend path
        false,
        false,
        false,
        None,
    )
    .await
    .unwrap()
}

// ── create_outgoing_share ─────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_outgoing_share_same_backend_creates_incoming_share(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let _bob_id = common::seed_user(&db, "bob", "pass").await;

    let share = alice_shares_with_bob(&db, alice_id, "vacation").await;

    let outgoing = OutgoingShareRepository::get_by_id(&db, share.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outgoing.status, ShareStatus::Pending);
    assert_eq!(outgoing.recipient_username, "bob");

    // The IncomingShare must be auto-created in the same transaction.
    let incoming = IncomingShareRepository::find_by_outgoing_share(&db, share.id, "test.com")
        .await
        .unwrap()
        .expect("incoming share must be created for same-backend recipient");
    assert_eq!(incoming.status, ShareStatus::Pending);
    assert_eq!(incoming.sender_username, "alice");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_outgoing_share_propagates_name_and_message_same_backend(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    common::seed_user(&db, "bob", "pass").await;
    let config = config();
    let (fed, cache) = common::make_federation(&config);
    let notify = RoutineHandle::<uuid::Uuid>::disconnected();

    let share = shares::create_outgoing_share(
        &db,
        cache.as_ref(),
        &fed,
        &config,
        &notify,
        alice_id,
        "alice",
        "vacation",
        "Alps 2024",
        Some("Hope you enjoy these!"),
        "bob",
        "test.com",
        false,
        false,
        false,
        None,
    )
    .await
    .unwrap();

    // The outgoing share carries the name/message…
    assert_eq!(share.name, "Alps 2024");
    assert_eq!(share.message.as_deref(), Some("Hope you enjoy these!"));

    // …and they are propagated to the recipient's IncomingShare.
    let incoming = IncomingShareRepository::find_by_outgoing_share(&db, share.id, "test.com")
        .await
        .unwrap()
        .expect("incoming share must exist");
    assert_eq!(incoming.name, "Alps 2024");
    assert_eq!(incoming.message.as_deref(), Some("Hope you enjoy these!"));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_outgoing_share_rejects_blank_name(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    common::seed_user(&db, "bob", "pass").await;
    let config = config();
    let (fed, cache) = common::make_federation(&config);
    let notify = RoutineHandle::<uuid::Uuid>::disconnected();

    let result = shares::create_outgoing_share(
        &db,
        cache.as_ref(),
        &fed,
        &config,
        &notify,
        alice_id,
        "alice",
        "vacation",
        "   ",
        None,
        "bob",
        "test.com",
        false,
        false,
        false,
        None,
    )
    .await;
    assert!(
        matches!(
            result,
            Err(archypix_back::infra::error::AppError::BadRequest(_))
        ),
        "blank share name must be rejected, got {result:?}"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_outgoing_share_rejects_invalid_recipient_instance(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let config = config();
    let (fed, cache) = common::make_federation(&config);
    let notify = RoutineHandle::<uuid::Uuid>::disconnected();

    // `localhost` is a forbidden federation target (SSRF guard, 07_security_audit.md §2.4).
    let result = shares::create_outgoing_share(
        &db,
        cache.as_ref(),
        &fed,
        &config,
        &notify,
        alice_id,
        "alice",
        "vacation",
        "Test share",
        None,
        "bob",
        "localhost",
        false,
        false,
        false,
        None,
    )
    .await;
    assert!(
        matches!(
            result,
            Err(archypix_back::infra::error::AppError::BadRequest(_))
        ),
        "localhost recipient must be rejected, got {result:?}"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_outgoing_share_enforces_pending_cap(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    common::seed_user(&db, "bob", "pass").await;

    let mut config = config();
    config.max_pending_outgoing_shares = 1;
    let (fed, cache) = common::make_federation(&config);
    let notify = RoutineHandle::<uuid::Uuid>::disconnected();

    // First pending share is accepted.
    shares::create_outgoing_share(
        &db,
        cache.as_ref(),
        &fed,
        &config,
        &notify,
        alice_id,
        "alice",
        "vacation",
        "Test share",
        None,
        "bob",
        "test.com",
        false,
        false,
        false,
        None,
    )
    .await
    .expect("first share within cap");

    // Second one exceeds the cap → 429.
    let result = shares::create_outgoing_share(
        &db,
        cache.as_ref(),
        &fed,
        &config,
        &notify,
        alice_id,
        "alice",
        "trips",
        "Test share",
        None,
        "bob",
        "test.com",
        false,
        false,
        false,
        None,
    )
    .await;
    assert!(
        matches!(
            result,
            Err(archypix_back::infra::error::AppError::TooManyRequests(_))
        ),
        "second pending share must hit the cap, got {result:?}"
    );
}

// ── accept_incoming_share ─────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn accept_incoming_share_registers_pictures(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let bob_id = common::seed_user(&db, "bob", "pass").await;

    // Alice has a picture tagged "vacation".
    common::seed_picture_with_tag(&db, alice_id, "vacation").await;

    let share = alice_shares_with_bob(&db, alice_id, "vacation").await;
    let incoming = IncomingShareRepository::find_by_outgoing_share(&db, share.id, "test.com")
        .await
        .unwrap()
        .unwrap();

    accept_and_announce(&db, &config(), bob_id, "bob", incoming.id, alice_id).await;

    assert_eq!(
        common::count_received_pictures(&db, bob_id).await,
        1,
        "Bob must have one received picture"
    );

    let expected_tag = common::shared_to_me_tag("alice", "test.com", "vacation");
    let tags = common::received_picture_tags(&db, bob_id).await;
    assert!(
        tags.contains(&expected_tag),
        "SharedToMe tag must be assigned; got: {:?}",
        tags
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn accept_incoming_share_is_idempotent(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let bob_id = common::seed_user(&db, "bob", "pass").await;
    common::seed_picture_with_tag(&db, alice_id, "vacation").await;

    let share = alice_shares_with_bob(&db, alice_id, "vacation").await;
    let incoming = IncomingShareRepository::find_by_outgoing_share(&db, share.id, "test.com")
        .await
        .unwrap()
        .unwrap();

    let config = config();
    let (fed, cache) = common::make_federation(&config);

    // First accept → announces the picture via the pipeline.
    accept_and_announce(&db, &config, bob_id, "bob", incoming.id, alice_id).await;

    // Second accept — must be a no-op (share already Active; no duplicate pictures).
    shares::accept_incoming_share(
        &db,
        cache.as_ref(),
        &fed,
        &config,
        &archypix_back::infra::routine::RoutineHandle::<uuid::Uuid>::disconnected(),
        bob_id,
        "bob",
        incoming.id,
    )
    .await
    .unwrap();

    assert_eq!(
        common::count_received_pictures(&db, bob_id).await,
        1,
        "still exactly one received picture"
    );
}

// ── revoke_outgoing_share ─────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn revoke_outgoing_share_removes_shared_tags(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let bob_id = common::seed_user(&db, "bob", "pass").await;
    common::seed_picture_with_tag(&db, alice_id, "vacation").await;

    let share = alice_shares_with_bob(&db, alice_id, "vacation").await;
    let incoming = IncomingShareRepository::find_by_outgoing_share(&db, share.id, "test.com")
        .await
        .unwrap()
        .unwrap();

    let config = config();
    let (fed, cache) = common::make_federation(&config);

    // Bob accepts → pipeline announces → picture + tag appear
    accept_and_announce(&db, &config, bob_id, "bob", incoming.id, alice_id).await;
    assert_eq!(common::count_received_pictures(&db, bob_id).await, 1);

    // Alice revokes → tag removed, unreachable received picture deleted
    let (tq, notify) = common::test_task_queue(&db, &config);
    shares::revoke_outgoing_share(
        &db,
        cache.as_ref(),
        &fed,
        &config,
        &tq,
        &notify,
        alice_id,
        "alice",
        share.id,
    )
    .await
    .unwrap();

    assert_eq!(
        common::count_received_pictures(&db, bob_id).await,
        0,
        "received picture must be deleted after revocation"
    );
    assert!(
        common::received_picture_tags(&db, bob_id).await.is_empty(),
        "SharedToMe tags must be gone"
    );

    let outgoing = OutgoingShareRepository::get_by_id(&db, share.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outgoing.status, ShareStatus::Revoked);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn revoke_outgoing_share_before_accept_leaves_no_incoming_tags(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let _bob_id = common::seed_user(&db, "bob", "pass").await;

    let share = alice_shares_with_bob(&db, alice_id, "vacation").await;
    let config = config();
    let (fed, cache) = common::make_federation(&config);

    // Revoke immediately, before Bob accepts (no received pictures yet)
    let (tq, notify) = common::test_task_queue(&db, &config);
    shares::revoke_outgoing_share(
        &db,
        cache.as_ref(),
        &fed,
        &config,
        &tq,
        &notify,
        alice_id,
        "alice",
        share.id,
    )
    .await
    .unwrap();

    let outgoing = OutgoingShareRepository::get_by_id(&db, share.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outgoing.status, ShareStatus::Revoked);
}

// ── reject_incoming_share ─────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn reject_incoming_share_pending_tombstones_outgoing(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let bob_id = common::seed_user(&db, "bob", "pass").await;

    let share = alice_shares_with_bob(&db, alice_id, "vacation").await;
    let incoming = IncomingShareRepository::find_by_outgoing_share(&db, share.id, "test.com")
        .await
        .unwrap()
        .unwrap();

    let config = config();
    let (fed, cache) = common::make_federation(&config);

    // Bob rejects a pending share
    let (tq, notify) = common::test_task_queue(&db, &config);
    shares::reject_incoming_share(
        &db,
        cache.as_ref(),
        &fed,
        &config,
        &tq,
        &notify,
        bob_id,
        "bob",
        incoming.id,
    )
    .await
    .unwrap();

    let incoming_after = IncomingShareRepository::get_by_id(&db, incoming.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(incoming_after.status, ShareStatus::Tombstoned);

    // Same-backend: sender's OutgoingShare must also be tombstoned
    let outgoing_after = OutgoingShareRepository::get_by_id(&db, share.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outgoing_after.status, ShareStatus::Tombstoned);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn reject_incoming_share_active_removes_tags(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let bob_id = common::seed_user(&db, "bob", "pass").await;
    common::seed_picture_with_tag(&db, alice_id, "vacation").await;

    let share = alice_shares_with_bob(&db, alice_id, "vacation").await;
    let incoming = IncomingShareRepository::find_by_outgoing_share(&db, share.id, "test.com")
        .await
        .unwrap()
        .unwrap();

    let config = config();
    let (fed, cache) = common::make_federation(&config);

    // Bob accepts first → pipeline announces the picture
    accept_and_announce(&db, &config, bob_id, "bob", incoming.id, alice_id).await;
    assert_eq!(common::count_received_pictures(&db, bob_id).await, 1);

    // Then rejects → cleanup must run
    let (tq, notify) = common::test_task_queue(&db, &config);
    shares::reject_incoming_share(
        &db,
        cache.as_ref(),
        &fed,
        &config,
        &tq,
        &notify,
        bob_id,
        "bob",
        incoming.id,
    )
    .await
    .unwrap();

    assert_eq!(
        common::count_received_pictures(&db, bob_id).await,
        0,
        "received picture must be deleted on rejection"
    );
    let incoming_after = IncomingShareRepository::get_by_id(&db, incoming.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(incoming_after.status, ShareStatus::Tombstoned);
}

// ── register_received_pictures ────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn register_received_pictures_is_idempotent(db: PgPool) {
    use archypix_back::domain::tag::TagPath;
    use archypix_back::services::shares::register_received_pictures;
    use uuid::Uuid;

    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let bob_id = common::seed_user(&db, "bob", "pass").await;

    let share = alice_shares_with_bob(&db, alice_id, "vacation").await;
    let incoming = IncomingShareRepository::find_by_outgoing_share(&db, share.id, "test.com")
        .await
        .unwrap()
        .unwrap();

    let shared_tag = TagPath::shared_to_me("alice", "test.com", &TagPath::from_ltree("vacation"));
    let pics = vec![AnnouncedPicture {
        picture_id: Uuid::new_v4().to_string(),
        owner_username: "alice".to_string(),
        owner_instance_domain: "test.com".to_string(),
        picture_token: Uuid::new_v4(),
        filename: None,
        mime_type: None,
        file_size: None,
        file_hash: None,
        content_hash: None,
        thumbnails_generated_at: None,
        width: None,
        height: None,
        blurhash: None,
        exif: archypix_back::domain::job::FullExif {
            gps_lat: Some(45.92),
            gps_lng: Some(6.87),
            gps_alt: Some(1200),
            orientation: Some(6),
            camera: archypix_back::domain::job::CameraExif {
                camera_brand: Some("Canon".to_string()),
                ..Default::default()
            },
            ..Default::default()
        },
        owner_deleted_at: None,
        owner_purge_at: None,
    }];

    // Register twice — second call must be a no-op (ON CONFLICT DO UPDATE / DO NOTHING)
    let n1 = register_received_pictures(&db, bob_id, incoming.id, &shared_tag, &pics)
        .await
        .unwrap();
    let n2 = register_received_pictures(&db, bob_id, incoming.id, &shared_tag, &pics)
        .await
        .unwrap();

    assert_eq!(n1, 1);
    assert_eq!(n2, 1); // function returns slice length, not inserted count
    assert_eq!(
        common::count_received_pictures(&db, bob_id).await,
        1,
        "only one picture row must exist"
    );

    // Geo/EXIF metadata from the announcement must persist on the recipient row (§10.2).
    let (lat, lng, alt, orient): (Option<f64>, Option<f64>, Option<i32>, Option<i16>) =
        sqlx::query_as(
            "SELECT gps_lat, gps_lng, gps_alt, orientation FROM pictures
         WHERE local_user_id = $1 AND remote_picture_id IS NOT NULL",
        )
        .bind(bob_id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(lat, Some(45.92));
    assert_eq!(lng, Some(6.87));
    assert_eq!(alt, Some(1200));
    assert_eq!(orient, Some(6));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn register_received_pictures_persists_hash_and_thumbnail_ts(db: PgPool) {
    use archypix_back::domain::tag::TagPath;
    use archypix_back::services::shares::register_received_pictures;
    use uuid::Uuid;

    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let bob_id = common::seed_user(&db, "bob", "pass").await;
    let share = alice_shares_with_bob(&db, alice_id, "vacation").await;
    let incoming = IncomingShareRepository::find_by_outgoing_share(&db, share.id, "test.com")
        .await
        .unwrap()
        .unwrap();
    let shared_tag = TagPath::shared_to_me("alice", "test.com", &TagPath::from_ltree("vacation"));

    let thumb_ts = chrono::NaiveDate::from_ymd_opt(2024, 6, 1)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap();
    let mut pic = AnnouncedPicture {
        picture_id: Uuid::new_v4().to_string(),
        owner_username: "alice".to_string(),
        owner_instance_domain: "test.com".to_string(),
        picture_token: Uuid::new_v4(),
        filename: None,
        mime_type: None,
        file_size: Some(4242),
        file_hash: Some("deadbeef".to_string()),
        content_hash: None,
        thumbnails_generated_at: Some(thumb_ts),
        width: None,
        height: None,
        blurhash: None,
        exif: Default::default(),
        owner_deleted_at: None,
        owner_purge_at: None,
    };
    register_received_pictures(&db, bob_id, incoming.id, &shared_tag, &[pic.clone()])
        .await
        .unwrap();

    let (hash, ts, size): (Option<String>, Option<chrono::NaiveDateTime>, Option<i64>) =
        sqlx::query_as(
            "SELECT file_hash, thumbnails_generated_at, file_size FROM pictures
             WHERE local_user_id = $1 AND remote_picture_id IS NOT NULL",
        )
        .bind(bob_id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(
        hash.as_deref(),
        Some("deadbeef"),
        "file_hash propagated to recipient"
    );
    assert_eq!(ts, Some(thumb_ts), "thumbnails_generated_at propagated");
    assert_eq!(size, Some(4242));

    // A later announce that fills the hash (was null) must refresh it via ON CONFLICT.
    pic.file_hash = Some("cafef00d".to_string());
    register_received_pictures(&db, bob_id, incoming.id, &shared_tag, &[pic])
        .await
        .unwrap();
    let hash2: Option<String> = sqlx::query_scalar(
        "SELECT file_hash FROM pictures WHERE local_user_id = $1 AND remote_picture_id IS NOT NULL",
    )
    .bind(bob_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(
        hash2.as_deref(),
        Some("cafef00d"),
        "re-announce refreshes file_hash"
    );
}

// ── cleanup_incoming_share ────────────────────────────────────────────────────

/// A picture tagged with both "vacation" and "trip" appears in two separate shares.
/// Revoking share1 ("vacation") removes its incoming_share tag, but the picture
/// still has share2's ("trip") incoming_share tag, so it must NOT be deleted.
#[sqlx::test(migrator = "MIGRATOR")]
async fn cleanup_incoming_share_deletes_unreachable_pictures_only(db: PgPool) {
    use archypix_back::repository::tag::TagRepository;

    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let bob_id = common::seed_user(&db, "bob", "pass").await;

    // One picture tagged with both "vacation" and "trip"
    let pic_id = common::seed_picture(&db, alice_id).await;
    TagRepository::batch_assign(
        &db,
        alice_id,
        &[pic_id],
        &["vacation".to_string(), "trip".to_string()],
    )
    .await
    .unwrap();

    // Share 1: "vacation" → Bob
    let share1 = alice_shares_with_bob(&db, alice_id, "vacation").await;
    let incoming1 = IncomingShareRepository::find_by_outgoing_share(&db, share1.id, "test.com")
        .await
        .unwrap()
        .unwrap();

    let config = config();
    let (fed, cache) = common::make_federation(&config);
    let notify = archypix_back::infra::routine::RoutineHandle::<uuid::Uuid>::disconnected();

    // Share 2: "trip" → Bob (different tag — no unique-constraint conflict)
    let share2 = shares::create_outgoing_share(
        &db,
        cache.as_ref(),
        &fed,
        &config,
        &notify,
        alice_id,
        "alice",
        "trip",
        "Test share",
        None,
        "bob",
        "test.com",
        false,
        false,
        false,
        None,
    )
    .await
    .unwrap();
    let incoming2 = IncomingShareRepository::find_by_outgoing_share(&db, share2.id, "test.com")
        .await
        .unwrap()
        .unwrap();

    // Bob accepts both → same received picture row, two incoming_share tags (announced by the
    // sender's pipeline).
    accept_and_announce(&db, &config, bob_id, "bob", incoming1.id, alice_id).await;
    accept_and_announce(&db, &config, bob_id, "bob", incoming2.id, alice_id).await;

    assert_eq!(
        common::count_received_pictures(&db, bob_id).await,
        1,
        "same remote picture → only one received row"
    );

    // Revoke share1 → removes its incoming_share tag, but share2's tag remains
    let (tq, notify) = common::test_task_queue(&db, &config);
    shares::revoke_outgoing_share(
        &db,
        cache.as_ref(),
        &fed,
        &config,
        &tq,
        &notify,
        alice_id,
        "alice",
        share1.id,
    )
    .await
    .unwrap();

    assert_eq!(
        common::count_received_pictures(&db, bob_id).await,
        1,
        "picture still reachable via share2 must not be deleted"
    );
}
