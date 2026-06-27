//! Feature 10 — Recipient EXIF editing with owner propagation.
//!
//! Same-backend (Alice owner → Bob recipient) scenarios driven through the real owner apply +
//! pipeline re-announce. The propose path short-circuits to a direct owner service call when the
//! owner is local, so these exercise the full owner-authoritative flow without HTTP. See
//! `doc/features/10_recipient_exif_editing.md`.

mod common;

use archypix_back::domain::job::{ExifField, FullExif};
use archypix_back::infra::config::Config;
use archypix_back::infra::error::AppError;
use archypix_back::infra::routine::RoutineHandle;
use archypix_back::infra::routine::pipeline::{self};
use archypix_back::repository::picture::PictureRepository;
use archypix_back::repository::share::IncomingShareRepository;
use archypix_back::services::{federation, pictures, shares};
use sqlx::PgPool;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

fn config() -> Config {
    Config::test_defaults()
}

/// Alice shares `tag` with Bob (same backend), `allow_exif_edit = allow_edit`, Bob accepts, and
/// Alice's pipeline announces the coverage. The owner picture is marked extraction-complete and
/// JPEG so it is editable. Returns `(alice_pic_id, bob_pic_id, bob_id, alice_id, outgoing_id)`.
async fn editable_share(
    db: &PgPool,
    tag: &str,
    allow_edit: bool,
) -> (Uuid, Uuid, Uuid, Uuid, Uuid) {
    let cfg = config();
    let alice_id = common::seed_user(db, "alice", "pass").await;
    let bob_id = common::seed_user(db, "bob", "pass").await;
    let alice_pic = common::seed_picture_with_tag(db, alice_id, tag).await;
    // Owner edits require a thumbnailed (extraction-complete) JPEG (04 §11.2 / §8).
    sqlx::query!(
        "UPDATE pictures SET thumbnails_generated_at = now() at time zone 'utc', mime_type = 'image/jpeg' WHERE id = $1",
        alice_pic,
    )
        .execute(db)
        .await
        .unwrap();

    let (fed, cache) = common::make_federation(&cfg);
    let (_queue, notify) = common::test_task_queue(db, &cfg);

    let share = shares::create_outgoing_share(
        db,
        cache.as_ref(),
        &fed,
        &cfg,
        &notify,
        alice_id,
        "alice",
        tag,
        "Test share",
        None,
        "bob",
        "test.com",
        true,
        allow_edit,
        true,
        None,
    )
    .await
    .unwrap();
    let incoming = IncomingShareRepository::find_by_outgoing_share(db, share.id, "test.com")
        .await
        .unwrap()
        .unwrap();
    // The grant propagates to the recipient's incoming share.
    assert_eq!(incoming.allow_exif_edit, allow_edit);

    shares::accept_incoming_share(
        db,
        cache.as_ref(),
        &fed,
        &cfg,
        &notify,
        bob_id,
        "bob",
        incoming.id,
    )
    .await
    .unwrap();
    pipeline::run_once_for_user(db, &fed, cache.as_ref(), &cfg, &notify, alice_id)
        .await
        .unwrap();

    let bob_pic = bob_received(db, bob_id).await.id;
    (alice_pic, bob_pic, bob_id, alice_id, share.id)
}

async fn run_alice(db: &PgPool, alice_id: Uuid) {
    let cfg = config();
    let (fed, cache) = common::make_federation(&cfg);
    let (_queue, notify) = common::test_task_queue(db, &cfg);
    pipeline::run_once_for_user(db, &fed, cache.as_ref(), &cfg, &notify, alice_id)
        .await
        .unwrap();
}

async fn bob_received(db: &PgPool, bob_id: Uuid) -> archypix_back::domain::picture::Picture {
    let id: Uuid = sqlx::query_scalar(
        "SELECT id FROM pictures WHERE local_user_id = $1 AND remote_picture_id IS NOT NULL",
    )
    .bind(bob_id)
    .fetch_one(db)
    .await
    .unwrap();
    PictureRepository::find_by_id(db, id)
        .await
        .unwrap()
        .unwrap()
}

async fn propose(
    db: &PgPool,
    bob_id: Uuid,
    bob_pic: Uuid,
    set: FullExif,
    clear: Vec<ExifField>,
) -> Result<archypix_back::domain::picture::Picture, AppError> {
    let cfg = config();
    let (fed, cache) = common::make_federation(&cfg);
    pictures::propose_received_exif(
        db,
        cache.as_ref(),
        &cfg,
        &fed,
        &RoutineHandle::<uuid::Uuid>::disconnected(),
        bob_id,
        "bob",
        bob_pic,
        set,
        clear,
    )
    .await
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn propose_without_grant_is_forbidden(db: PgPool) {
    let (_alice_pic, bob_pic, bob_id, _alice_id, _os) =
        editable_share(&db, "vacation", false).await;
    let cap = chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let err = propose(
        &db,
        bob_id,
        bob_pic,
        FullExif {
            captured_at: Some(cap),
            ..Default::default()
        },
        vec![],
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, AppError::Forbidden(_)),
        "propose without an allow_exif_edit grant must be 403, got {err:?}"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn propose_applies_at_owner_and_propagates(db: PgPool) {
    let (alice_pic, bob_pic, bob_id, alice_id, _os) = editable_share(&db, "vacation", true).await;
    let cap = chrono::NaiveDate::from_ymd_opt(2024, 8, 3)
        .unwrap()
        .and_hms_opt(10, 15, 0)
        .unwrap();

    // Bob proposes captured_at; same-backend → applied at the owner synchronously.
    propose(
        &db,
        bob_id,
        bob_pic,
        FullExif {
            captured_at: Some(cap),
            ..Default::default()
        },
        vec![],
    )
    .await
    .unwrap();

    // The owner's authoritative row is updated (it owns the result, LWW).
    let owner = PictureRepository::find_by_id(&db, alice_pic)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        owner.captured_at,
        Some(cap),
        "owner auto-applied the proposal"
    );

    // Re-announce propagates to the recipient (and would to all recipients).
    run_alice(&db, alice_id).await;
    let bob = bob_received(&db, bob_id).await;
    assert_eq!(
        bob.captured_at,
        Some(cap),
        "owner-applied value flows back to the proposing recipient"
    );
    assert_eq!(
        bob.remote_exif_data.as_ref().unwrap().0.captured_at,
        Some(cap),
        "owner snapshot carries the applied value"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn escalate_clears_local_override(db: PgPool) {
    let (alice_pic, bob_pic, bob_id, alice_id, _os) = editable_share(&db, "vacation", true).await;

    // Bob first overrides gps_lat locally (private).
    pictures::override_received_exif(
        &db,
        &RoutineHandle::<uuid::Uuid>::disconnected(),
        bob_id,
        bob_pic,
        FullExif {
            gps_lat: Some(99.0),
            ..Default::default()
        },
        vec![],
    )
    .await
    .unwrap();
    assert_eq!(
        bob_received(&db, bob_id)
            .await
            .local_exif_overrides
            .as_ref()
            .unwrap()
            .0
            .gps_lat,
        Some(99.0)
    );

    // Then escalates the same field to a proposal → the per-field override is cleared.
    propose(
        &db,
        bob_id,
        bob_pic,
        FullExif {
            gps_lat: Some(10.0),
            ..Default::default()
        },
        vec![],
    )
    .await
    .unwrap();
    let after = bob_received(&db, bob_id).await;
    let still_overridden = after
        .local_exif_overrides
        .as_ref()
        .map(|j| j.0.gps_lat)
        .unwrap_or(None);
    assert_eq!(
        still_overridden, None,
        "escalating a field to a proposal clears its local override"
    );
    assert_eq!(
        PictureRepository::find_by_id(&db, alice_pic)
            .await
            .unwrap()
            .unwrap()
            .gps_lat,
        Some(10.0),
        "owner applied the proposed value"
    );

    // The owner value now flows through unshadowed after re-announce.
    run_alice(&db, alice_id).await;
    assert_eq!(bob_received(&db, bob_id).await.gps_lat, Some(10.0));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn owner_rejects_when_grant_revoked_in_flight(db: PgPool) {
    let (alice_pic, _bob_pic, _bob_id, _alice_id, os) = editable_share(&db, "vacation", true).await;
    // Owner toggles the grant off (10 §3): future proposals are rejected at the owner.
    sqlx::query!(
        "UPDATE outgoing_shares SET allow_exif_edit = false WHERE id = $1",
        os,
    )
    .execute(&db)
    .await
    .unwrap();

    // The owner-side handler re-verifies the grant (never trusts the wire) → 403.
    let err = federation::receive_picture_edit_request(
        &db,
        &RoutineHandle::<uuid::Uuid>::disconnected(),
        &alice_pic.to_string(),
        "bob",
        "test.com",
        FullExif {
            orientation: Some(3),
            ..Default::default()
        },
        vec![],
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, AppError::Forbidden(_)),
        "a revoked grant must reject the proposal at the owner, got {err:?}"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn owner_rejects_edit_for_uncovered_recipient(db: PgPool) {
    // A recipient with no covering grant cannot edit, even naming a real owned picture.
    let (alice_pic, _bob_pic, _bob_id, _alice_id, _os) =
        editable_share(&db, "vacation", true).await;
    let err = federation::receive_picture_edit_request(
        &db,
        &RoutineHandle::<uuid::Uuid>::disconnected(),
        &alice_pic.to_string(),
        "mallory",
        "evil.com",
        FullExif {
            orientation: Some(3),
            ..Default::default()
        },
        vec![],
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, AppError::Forbidden(_)),
        "an uncovered requester must be rejected, got {err:?}"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn propose_on_owned_picture_is_rejected(db: PgPool) {
    // The recipient endpoint rejects owned pictures (use /edit instead).
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic = common::seed_picture_with_tag(&db, alice_id, "vacation").await;
    let err = propose(&db, alice_id, pic, FullExif::default(), vec![])
        .await
        .unwrap_err();
    assert!(
        matches!(err, AppError::BadRequest(_)),
        "proposing on an owned picture must be 400, got {err:?}"
    );
}
