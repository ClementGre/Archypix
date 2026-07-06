//! Feature 09 — Trash, owner-deletion propagation & recipient EXIF overrides.
//!
//! Same-backend (Alice → Bob) share scenarios driven through the real pipeline announcement step,
//! plus repository-level coverage/purge checks. See `doc/features/09_trash_and_exif_overrides.md`.

mod common;

use archypix_back::domain::job::{ExifField, FullExif};
use archypix_back::infra::routine::Routine;
use archypix_back::infra::routine::RoutineHandle;
use archypix_back::infra::routine::pipeline::{self};
use archypix_back::infra::routine::purge_sweep::PurgeSweepRoutine;
use archypix_back::infra::settings::test_settings_with;
use archypix_back::repository::picture::PictureRepository;
use archypix_back::repository::share::IncomingShareRepository;
use archypix_back::repository::share_announcement::ShareAnnouncementRepository;
use archypix_back::services::{pictures, shares};
use sqlx::PgPool;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Alice shares `tag` with Bob (same backend) with `future = true`, Bob accepts, and Alice's
/// pipeline announces the coverage. Returns `(alice_pic_id, bob_id, alice_id)`.
async fn share_and_announce(db: &PgPool, tag: &str) -> (Uuid, Uuid, Uuid) {
    let settings = test_settings_with(&[]);
    let alice_id = common::seed_user(db, "alice", "pass").await;
    let bob_id = common::seed_user(db, "bob", "pass").await;
    let alice_pic = common::seed_picture_with_tag(db, alice_id, tag).await;

    let (fed, cache) = common::make_federation(&settings);
    let (_queue, notify) = common::test_task_queue(db, &settings);

    let share = shares::create_outgoing_share(
        db,
        cache.as_ref(),
        &fed,
        &settings,
        &notify,
        alice_id,
        "alice",
        tag,
        "Test share",
        None,
        "bob",
        "test.com",
        true,
        false,
        true, // future = true → active-delta re-announce on metadata/lifecycle change
        None,
    )
    .await
    .unwrap();
    let incoming = IncomingShareRepository::find_by_outgoing_share(db, share.id, "test.com")
        .await
        .unwrap()
        .unwrap();
    shares::accept_incoming_share(
        db,
        cache.as_ref(),
        &fed,
        &settings,
        &notify,
        bob_id,
        "bob",
        incoming.id,
    )
    .await
    .unwrap();
    pipeline::run_once_for_user(db, &fed, cache.as_ref(), &settings, &notify, alice_id)
        .await
        .unwrap();

    (alice_pic, bob_id, alice_id)
}

async fn run_alice(db: &PgPool, alice_id: Uuid) {
    let settings = test_settings_with(&[]);
    let (fed, cache) = common::make_federation(&settings);
    let (_queue, notify) = common::test_task_queue(db, &settings);
    pipeline::run_once_for_user(db, &fed, cache.as_ref(), &settings, &notify, alice_id)
        .await
        .unwrap();
}

/// Bob's single received row.
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

#[sqlx::test(migrator = "MIGRATOR")]
async fn owner_trash_announces_lifecycle_then_restore_clears(db: PgPool) {
    let (alice_pic, bob_id, alice_id) = share_and_announce(&db, "vacation").await;
    let before = bob_received(&db, bob_id).await;
    assert!(
        before.owner_deleted_at.is_none(),
        "no lifecycle flag before owner trashes"
    );

    // Owner trashes the shared picture → kept in coverage, re-announced with the lifecycle flag.
    pictures::trash_picture(
        &db,
        &RoutineHandle::<Uuid>::disconnected(),
        alice_id,
        alice_pic,
    )
    .await
    .unwrap();
    run_alice(&db, alice_id).await;

    assert_eq!(
        common::count_received_pictures(&db, bob_id).await,
        1,
        "owner trash keeps the picture at the recipient (grace window, not removed)"
    );
    let trashed = bob_received(&db, bob_id).await;
    assert!(
        trashed.owner_deleted_at.is_some(),
        "owner_deleted_at announced to recipient"
    );
    assert!(
        trashed.owner_purge_at.is_some(),
        "owner_purge_at (derived) announced to recipient"
    );
    assert!(
        trashed.deleted_at.is_none(),
        "recipient's own local trash is untouched"
    );

    // Owner restores before purge → re-announce clears the lifecycle flag.
    pictures::restore_picture(
        &db,
        &RoutineHandle::<Uuid>::disconnected(),
        alice_id,
        alice_pic,
    )
    .await
    .unwrap();
    run_alice(&db, alice_id).await;
    let restored = bob_received(&db, bob_id).await;
    assert!(
        restored.owner_deleted_at.is_none(),
        "restore clears owner_deleted_at"
    );
    assert!(
        restored.owner_purge_at.is_none(),
        "restore clears owner_purge_at"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn recipient_override_is_sticky_owner_edit_flows_through(db: PgPool) {
    let (alice_pic, bob_id, alice_id) = share_and_announce(&db, "vacation").await;
    let bob_pic = bob_received(&db, bob_id).await.id;

    // Bob overrides gps_lat locally (DB-only; no edit_picture job).
    pictures::override_received_exif(
        &db,
        &RoutineHandle::<Uuid>::disconnected(),
        bob_id,
        bob_pic,
        FullExif {
            gps_lat: Some(89.0),
            ..Default::default()
        },
        vec![],
        vec![],
    )
    .await
    .unwrap();
    let edit_jobs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM jobs WHERE picture_id = $1 AND job_type = 'edit_picture'",
    )
    .bind(bob_pic)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(
        edit_jobs, 0,
        "a recipient override never enqueues an edit_picture job"
    );
    assert_eq!(bob_received(&db, bob_id).await.gps_lat, Some(89.0));

    // Owner edits the same picture: gps_lat (overridden) + captured_at (not overridden).
    let cap = chrono::NaiveDate::from_ymd_opt(2024, 9, 9)
        .unwrap()
        .and_hms_opt(9, 0, 0)
        .unwrap();
    sqlx::query!(
        "UPDATE pictures SET gps_lat = 10.0, captured_at = $2, last_pipeline_run_at = NULL WHERE id = $1",
        alice_pic,
        cap,
    )
        .execute(&db)
        .await
        .unwrap();
    run_alice(&db, alice_id).await;

    let merged = bob_received(&db, bob_id).await;
    assert_eq!(
        merged.gps_lat,
        Some(89.0),
        "override stays sticky across an owner re-announce"
    );
    assert_eq!(
        merged.captured_at,
        Some(cap),
        "owner edit to a non-overridden field flows through"
    );
    // The owner snapshot reflects the owner's value even though the merged column is the override.
    let remote = merged.remote_exif_data.as_ref().unwrap();
    assert_eq!(remote.0.gps_lat, Some(10.0));

    // Bob clears the override → the owner's value flows through again.
    pictures::override_received_exif(
        &db,
        &RoutineHandle::<Uuid>::disconnected(),
        bob_id,
        bob_pic,
        FullExif::default(),
        vec![],
        vec![ExifField::GpsLat],
    )
    .await
    .unwrap();
    assert_eq!(
        bob_received(&db, bob_id).await.gps_lat,
        Some(10.0),
        "cleared override reveals owner value"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn recipient_can_override_a_field_to_empty_and_it_is_sticky(db: PgPool) {
    let (alice_pic, bob_id, alice_id) = share_and_announce(&db, "vacation").await;

    // Owner has a gps_lat; announce it so Bob's merged value reflects the owner.
    sqlx::query("UPDATE pictures SET gps_lat = 45.0, last_pipeline_run_at = NULL WHERE id = $1")
        .bind(alice_pic)
        .execute(&db)
        .await
        .unwrap();
    run_alice(&db, alice_id).await;
    let bob_pic = bob_received(&db, bob_id).await;
    assert_eq!(
        bob_pic.gps_lat,
        Some(45.0),
        "owner value flows through before override"
    );

    // Bob overrides gps_lat to *empty* (not just un-claim) — the owner still has a value.
    pictures::override_received_exif(
        &db,
        &RoutineHandle::<Uuid>::disconnected(),
        bob_id,
        bob_pic.id,
        FullExif::default(),
        vec![ExifField::GpsLat],
        vec![],
    )
    .await
    .unwrap();
    let after = bob_received(&db, bob_id).await;
    assert_eq!(
        after.gps_lat, None,
        "empty override shadows the owner value with emptiness"
    );
    assert_eq!(
        after.remote_exif_data.as_ref().and_then(|r| r.0.gps_lat),
        Some(45.0),
        "the owner snapshot still holds the owner value"
    );
    // The empty claim is stored as an explicit null (present key), not an absent key.
    assert_eq!(
        after
            .local_exif_overrides
            .as_ref()
            .and_then(|j| j.0.get("gps_lat")),
        Some(&serde_json::Value::Null),
        "empty claim is a present null key in the override JSON"
    );

    // Owner edits a *different* field and re-announces; the empty claim stays sticky.
    let cap = chrono::NaiveDate::from_ymd_opt(2024, 9, 9)
        .unwrap()
        .and_hms_opt(9, 0, 0)
        .unwrap();
    sqlx::query("UPDATE pictures SET captured_at = $2, last_pipeline_run_at = NULL WHERE id = $1")
        .bind(alice_pic)
        .bind(cap)
        .execute(&db)
        .await
        .unwrap();
    run_alice(&db, alice_id).await;
    let merged = bob_received(&db, bob_id).await;
    assert_eq!(
        merged.gps_lat, None,
        "empty override stays sticky across an owner re-announce"
    );
    assert_eq!(
        merged.captured_at,
        Some(cap),
        "non-claimed owner field still flows through"
    );

    // Bob clears the empty claim → the owner's value flows through again.
    pictures::override_received_exif(
        &db,
        &RoutineHandle::<Uuid>::disconnected(),
        bob_id,
        bob_pic.id,
        FullExif::default(),
        vec![],
        vec![ExifField::GpsLat],
    )
    .await
    .unwrap();
    assert_eq!(
        bob_received(&db, bob_id).await.gps_lat,
        Some(45.0),
        "clearing the empty claim reveals the owner value"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn recipient_local_trash_does_not_drop_share_coverage(db: PgPool) {
    // A relayer's local trash of a received picture must not remove it from share coverage
    // (coverage is by tag membership, not local deleted_at). 09 §7.
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let bob_id = common::seed_user(&db, "bob", "pass").await;
    // Bob "received" a picture (simulate by an owned-by-Bob picture under a tag he re-shares).
    let pic = common::seed_picture_with_tag(&db, bob_id, "SharedToMe.alice.vacation").await;
    PictureRepository::set_deleted(&db, bob_id, pic, true)
        .await
        .unwrap();

    let covered = ShareAnnouncementRepository::coverage_for_share(
        &db,
        bob_id,
        "SharedToMe.alice.vacation",
        "carol",
        "carol.com",
        None,
    )
    .await
    .unwrap();
    assert!(
        covered.contains(&pic),
        "locally-trashed picture stays in share coverage"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn find_purgeable_respects_retention_and_owner_only(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let recent = common::seed_picture(&db, alice_id).await;
    let old = common::seed_picture(&db, alice_id).await;
    PictureRepository::set_deleted(&db, alice_id, recent, true)
        .await
        .unwrap();
    PictureRepository::set_deleted(&db, alice_id, old, true)
        .await
        .unwrap();
    // Backdate `old` well past the default 30-day retention.
    sqlx::query!(
        "UPDATE pictures SET deleted_at = (now() at time zone 'utc') - INTERVAL '40 days' WHERE id = $1",
        old,
    )
        .execute(&db)
        .await
        .unwrap();

    let purgeable = PictureRepository::find_purgeable(&db, 100).await.unwrap();
    let ids: Vec<Uuid> = purgeable.iter().map(|(id, _)| *id).collect();
    assert!(
        ids.contains(&old),
        "retention-expired owned picture is purgeable"
    );
    assert!(
        !ids.contains(&recent),
        "recently-trashed picture is not yet purgeable"
    );

    // A shorter retention makes the recently-trashed one purgeable too (derived, no backfill).
    sqlx::query!(
        "UPDATE pictures SET deleted_at = (now() at time zone 'utc') - INTERVAL '2 days' WHERE id = $1",
        recent,
    )
        .execute(&db)
        .await
        .unwrap();
    archypix_back::repository::user_settings::UserSettingsRepository::upsert(
        &db,
        alice_id,
        None,
        Some(1),
    )
    .await
    .unwrap();
    let purgeable = PictureRepository::find_purgeable(&db, 100).await.unwrap();
    let ids: Vec<Uuid> = purgeable.iter().map(|(id, _)| *id).collect();
    assert!(
        ids.contains(&recent),
        "retention change shortens the derived purge deadline"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn purge_sweep_removes_owned_row_and_tracking(db: PgPool) {
    use std::sync::Arc;

    let settings = test_settings_with(&[]);
    let (alice_pic, _bob_id, alice_id) = share_and_announce(&db, "vacation").await;
    // A tracking row must exist after the announce.
    let tracked_before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM share_announcements WHERE picture_id = $1")
            .bind(alice_pic)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(tracked_before, 1);

    // Trash and backdate past retention.
    pictures::trash_picture(
        &db,
        &RoutineHandle::<Uuid>::disconnected(),
        alice_id,
        alice_pic,
    )
    .await
    .unwrap();
    sqlx::query!(
        "UPDATE pictures SET deleted_at = (now() at time zone 'utc') - INTERVAL '40 days' WHERE id = $1",
        alice_pic,
    )
        .execute(&db)
        .await
        .unwrap();

    let (queue, _notify) = common::test_task_queue(&db, &settings);
    let cache: Arc<dyn archypix_back::infra::redis::Cache> = Arc::new(common::InMemoryCache::new());
    let task = PurgeSweepRoutine::new(
        db.clone(),
        Arc::new(common::MockStorage::new()),
        cache,
        queue,
        settings.clone(),
    );
    task.run(()).await.unwrap();

    let row = PictureRepository::find_by_id(&db, alice_pic).await.unwrap();
    assert!(row.is_none(), "purged owned picture row is hard-deleted");
    let tracked_after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM share_announcements WHERE picture_id = $1")
            .bind(alice_pic)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(
        tracked_after, 0,
        "purge deletes the share_announcements tracking rows"
    );
}
