mod common;

use archypix_back::domain::job::{ExifField, FullExif};
use archypix_back::domain::picture::ExifSyncStatus;
use archypix_back::infra::routine::RoutineHandle;
use archypix_back::repository::picture::PictureRepository;
use archypix_back::services::jobs;
use archypix_common::error::AppError;
use sqlx::PgPool;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Mark a seeded picture as fully extracted and EXIF-capable so it passes the edit preflight.
async fn make_editable(db: &PgPool, picture_id: Uuid) {
    sqlx::query!(
        "UPDATE pictures
         SET mime_type = 'image/jpeg', thumbnails_generated_at = (now() AT TIME ZONE 'utc')
         WHERE id = $1",
        picture_id,
    )
    .execute(db)
    .await
    .unwrap();
}

fn gps_edit() -> (FullExif, Vec<ExifField>) {
    (
        FullExif {
            gps_lat: Some(45.0),
            gps_lng: Some(6.0),
            ..Default::default()
        },
        vec![],
    )
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn list_picture_jobs_rejects_wrong_owner(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let bob_id = common::seed_user(&db, "bob", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;

    let result = jobs::list_picture_jobs(&db, pic_id, bob_id).await;
    assert!(
        matches!(result, Err(AppError::NotFound)),
        "bob must not see alice's picture jobs"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn enqueue_edit_rejects_received_picture(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let bob_id = common::seed_user(&db, "bob", "pass").await;
    let alice_pic_id = common::seed_picture(&db, alice_id).await;
    let waker = RoutineHandle::<Uuid>::disconnected();

    // Create a received picture for Bob that points at Alice's picture.
    let received = PictureRepository::create_received(
        &db,
        bob_id,
        &alice_pic_id.to_string(),
        "alice",
        "test.com",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None, // content_hash
        &FullExif::default(),
        None,
        None,
        None, // creator
        None, // remote_updated_at
    )
    .await
    .unwrap();

    let (set, clear) = gps_edit();
    let result = jobs::edit_pictures_exif(&db, &waker, bob_id, &[received.id], set, clear).await;
    assert!(
        matches!(result, Err(AppError::BadRequest(_))),
        "editing a received picture must return BadRequest"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn enqueue_edit_rejects_picture_not_owned_by_user(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let bob_id = common::seed_user(&db, "bob", "pass").await;
    let alice_pic_id = common::seed_picture(&db, alice_id).await;
    make_editable(&db, alice_pic_id).await;
    let waker = RoutineHandle::<Uuid>::disconnected();

    let (set, clear) = gps_edit();
    let result = jobs::edit_pictures_exif(&db, &waker, bob_id, &[alice_pic_id], set, clear).await;
    assert!(
        matches!(result, Err(AppError::NotFound)),
        "bob must not enqueue edit for alice's picture"
    );
}

/// Feature 33 §4.1: `extracting` is the only state that refuses edits.
#[sqlx::test(migrator = "MIGRATOR")]
async fn enqueue_edit_rejects_still_processing_picture(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    make_editable(&db, pic_id).await;
    PictureRepository::set_exif_sync_status(&db, pic_id, ExifSyncStatus::Extracting)
        .await
        .unwrap();
    let waker = RoutineHandle::<Uuid>::disconnected();

    let (set, clear) = gps_edit();
    let result = jobs::edit_pictures_exif(&db, &waker, alice_id, &[pic_id], set, clear).await;
    assert!(
        matches!(result, Err(AppError::Conflict(_))),
        "editing a still-extracting picture must return Conflict (409)"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn edit_for_owned_picture_creates_job_and_marks_pending(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    make_editable(&db, pic_id).await;
    let waker = RoutineHandle::<Uuid>::disconnected();

    let (set, clear) = gps_edit();
    let outcome = jobs::edit_pictures_exif(&db, &waker, alice_id, &[pic_id], set, clear)
        .await
        .unwrap();

    assert_eq!(outcome.updated, vec![pic_id]);
    assert_eq!(outcome.jobs.len(), 1, "one reconcile job enqueued");
    assert!(outcome.unsupported.is_empty());

    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::Pending);
    assert_eq!(picture.gps_lat, Some(45.0));
    // The edit re-dirties the picture for pipeline re-evaluation.
    let last_run: Option<chrono::NaiveDateTime> = sqlx::query_scalar!(
        "SELECT last_pipeline_run_at FROM pictures WHERE id = $1",
        pic_id
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(last_run.is_none(), "edit must reset last_pipeline_run_at");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn edit_unsupported_format_is_db_only_no_job(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    sqlx::query!(
        "UPDATE pictures
         SET mime_type = 'image/gif', thumbnails_generated_at = (now() AT TIME ZONE 'utc')
         WHERE id = $1",
        pic_id,
    )
    .execute(&db)
    .await
    .unwrap();
    let waker = RoutineHandle::<Uuid>::disconnected();

    let (set, clear) = gps_edit();
    let outcome = jobs::edit_pictures_exif(&db, &waker, alice_id, &[pic_id], set, clear)
        .await
        .unwrap();

    assert!(
        outcome.jobs.is_empty(),
        "no reconcile job for unsupported format"
    );
    assert_eq!(outcome.unsupported, vec![pic_id]);

    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::UnsupportedMime);
    assert_eq!(picture.gps_lat, Some(45.0), "DB still updated");
}

/// An unknown MIME is attempted, not pre-judged: it is a gap in our metadata, not evidence about
/// the file. The edit path must agree with `ingest_exif_status` and the worker's read, all three of
/// which treat `None` as "try it" — stamping the terminal `unsupported_mime` from missing
/// information would suppress every future job on a guess (feature 33 §10).
#[sqlx::test(migrator = "MIGRATOR")]
async fn edit_of_unknown_mime_is_attempted_not_pre_judged(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    sqlx::query!(
        "UPDATE pictures
         SET mime_type = NULL, thumbnails_generated_at = (now() AT TIME ZONE 'utc'),
             exif_sync_status = 'synced'
         WHERE id = $1",
        pic_id,
    )
    .execute(&db)
    .await
    .unwrap();
    assert_eq!(
        archypix_back::services::jobs::ingest_exif_status(None),
        ExifSyncStatus::Extracting,
        "ingest attempts an unknown MIME",
    );
    let waker = RoutineHandle::<Uuid>::disconnected();

    let (set, clear) = gps_edit();
    let outcome = jobs::edit_pictures_exif(&db, &waker, alice_id, &[pic_id], set, clear)
        .await
        .unwrap();

    assert_eq!(
        outcome.jobs.len(),
        1,
        "the write is attempted so the worker can return a real verdict"
    );
    assert!(outcome.unsupported.is_empty());

    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::Pending);
}

/// A worker verdict of `unsupported` is terminal (feature 31 §6): re-editing such a picture must
/// not flip it back to `pending` and enqueue a job that can only fail the same way.
#[sqlx::test(migrator = "MIGRATOR")]
async fn edit_of_worker_marked_unsupported_enqueues_no_job(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    // A jpeg: the MIME preflight alone would call this supported.
    sqlx::query!(
        "UPDATE pictures
         SET mime_type = 'image/jpeg',
             thumbnails_generated_at = (now() AT TIME ZONE 'utc'),
             exif_sync_status = 'unsupported_mime'::picture_exif_sync_status
         WHERE id = $1",
        pic_id,
    )
    .execute(&db)
    .await
    .unwrap();
    let waker = RoutineHandle::<Uuid>::disconnected();

    let (set, clear) = gps_edit();
    let outcome = jobs::edit_pictures_exif(&db, &waker, alice_id, &[pic_id], set, clear)
        .await
        .unwrap();

    assert!(
        outcome.jobs.is_empty(),
        "terminal state must not re-enqueue"
    );
    assert_eq!(outcome.unsupported, vec![pic_id]);

    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::UnsupportedMime);
    assert_eq!(picture.gps_lat, Some(45.0), "DB still updated");
}

/// A format the worker touches for neither EXIF nor thumbnails never gets `thumbnails_generated_at`
/// stamped, so the still-processing gate must not block its DB-only edit (regression: previously
/// permanently rejected with 409).
#[sqlx::test(migrator = "MIGRATOR")]
async fn edit_unsupported_format_without_thumbnails_is_allowed(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    // image/svg+xml: not EXIF-capable and not thumbnailable → thumbnails_generated_at stays NULL.
    sqlx::query!(
        "UPDATE pictures SET mime_type = 'image/svg+xml' WHERE id = $1",
        pic_id,
    )
    .execute(&db)
    .await
    .unwrap();
    let waker = RoutineHandle::<Uuid>::disconnected();

    let (set, clear) = gps_edit();
    let outcome = jobs::edit_pictures_exif(&db, &waker, alice_id, &[pic_id], set, clear)
        .await
        .unwrap();

    assert_eq!(outcome.unsupported, vec![pic_id]);
    assert!(
        outcome.jobs.is_empty(),
        "no reconcile job for unsupported format"
    );

    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::UnsupportedMime);
    assert_eq!(picture.gps_lat, Some(45.0), "DB still updated");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn set_and_clear_conflict_is_rejected(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    make_editable(&db, pic_id).await;
    let waker = RoutineHandle::<Uuid>::disconnected();

    let set = FullExif {
        gps_lat: Some(45.0),
        gps_lng: Some(6.0),
        ..Default::default()
    };
    // Clearing GPS expands to lat+lng+alt, colliding with the set above.
    let result = jobs::edit_pictures_exif(
        &db,
        &waker,
        alice_id,
        &[pic_id],
        set,
        vec![ExifField::GpsAlt],
    )
    .await;
    assert!(
        matches!(result, Err(AppError::BadRequest(_))),
        "a field in both set and clear must be rejected"
    );
}

// ── Feature 31: revert to the physical file state ─────────────────────────────

/// Record a physical-file EXIF snapshot for `picture_id` (what a worker read back from S3).
async fn set_file_exif(db: &PgPool, picture_id: Uuid, snapshot: serde_json::Value) {
    sqlx::query!(
        "UPDATE pictures SET file_exif = $2 WHERE id = $1",
        picture_id,
        snapshot,
    )
    .execute(db)
    .await
    .unwrap();
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn revert_to_file_restores_the_snapshot_and_syncs(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    make_editable(&db, pic_id).await;
    set_file_exif(
        &db,
        pic_id,
        serde_json::json!({"gps_lat": 10.0, "gps_lng": 20.0, "camera_brand": "Canon"}),
    )
    .await;
    let waker = RoutineHandle::<Uuid>::disconnected();

    // The DB has moved on (a failed write left it diverged from the file).
    let (set, clear) = gps_edit();
    jobs::edit_pictures_exif(&db, &waker, alice_id, &[pic_id], set, clear)
        .await
        .unwrap();
    PictureRepository::set_exif_sync_status(&db, pic_id, ExifSyncStatus::WriteFailed)
        .await
        .unwrap();
    // …and no job is in flight any more (the failed one is terminal).
    sqlx::query!(
        "UPDATE jobs SET status = 'failed' WHERE picture_id = $1",
        pic_id
    )
    .execute(&db)
    .await
    .unwrap();

    let reverted = jobs::revert_picture_exif_to_file(&db, &waker, alice_id, pic_id)
        .await
        .unwrap();

    assert_eq!(reverted.exif_sync_status, ExifSyncStatus::Synced);
    assert_eq!(reverted.gps_lat, Some(10.0));
    assert_eq!(reverted.gps_lng, Some(20.0));
    assert_eq!(
        reverted.exif_data.0.camera_brand.as_deref(),
        Some("Canon"),
        "camera keys come back from the snapshot too"
    );
    // The reverted row keeps its file snapshot and is re-dirtied for the pipeline.
    assert!(reverted.file_exif.is_some());
    let last_run: Option<chrono::NaiveDateTime> = sqlx::query_scalar!(
        "SELECT last_pipeline_run_at FROM pictures WHERE id = $1",
        pic_id
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(last_run.is_none(), "revert must reset last_pipeline_run_at");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn revert_without_a_file_snapshot_conflicts(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    make_editable(&db, pic_id).await;
    let waker = RoutineHandle::<Uuid>::disconnected();

    let result = jobs::revert_picture_exif_to_file(&db, &waker, alice_id, pic_id).await;
    assert!(matches!(result, Err(AppError::Conflict(_))));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn revert_while_a_reconcile_is_in_flight_conflicts(db: PgPool) {
    // The in-flight job would write its pre-revert target back to the file and re-diverge the row.
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    make_editable(&db, pic_id).await;
    set_file_exif(&db, pic_id, serde_json::json!({"gps_lat": 10.0})).await;
    let waker = RoutineHandle::<Uuid>::disconnected();

    let (set, clear) = gps_edit();
    jobs::edit_pictures_exif(&db, &waker, alice_id, &[pic_id], set, clear)
        .await
        .unwrap();

    let result = jobs::revert_picture_exif_to_file(&db, &waker, alice_id, pic_id).await;
    assert!(matches!(result, Err(AppError::Conflict(_))));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn revert_rejects_a_foreign_picture(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let bob_id = common::seed_user(&db, "bob", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    set_file_exif(&db, pic_id, serde_json::json!({"gps_lat": 10.0})).await;
    let waker = RoutineHandle::<Uuid>::disconnected();

    let result = jobs::revert_picture_exif_to_file(&db, &waker, bob_id, pic_id).await;
    assert!(matches!(result, Err(AppError::NotFound)));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn resync_is_allowed_from_write_failed(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    make_editable(&db, pic_id).await;
    PictureRepository::set_exif_sync_status(&db, pic_id, ExifSyncStatus::WriteFailed)
        .await
        .unwrap();
    let waker = RoutineHandle::<Uuid>::disconnected();

    let job = jobs::resync_picture_exif(&db, &waker, alice_id, pic_id)
        .await
        .unwrap();
    assert_eq!(job.picture_id, Some(pic_id));
    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        picture.exif_sync_status,
        ExifSyncStatus::Pending,
        "retry puts the picture back in the sync queue"
    );
}

// ── Feature 33 §8: re-extraction ─────────────────────────────────────────────

/// Re-extraction makes the **file** authoritative (31 §5), so a row holding an unsynced DB edit is
/// refused rather than silently losing it.
#[sqlx::test(migrator = "MIGRATOR")]
async fn reextract_refuses_rows_with_unsynced_edits(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let waker = RoutineHandle::<Uuid>::disconnected();

    for status in [
        ExifSyncStatus::Pending,
        ExifSyncStatus::PendingJobCreation,
        ExifSyncStatus::WriteFailed,
    ] {
        let pic_id = common::seed_picture(&db, alice_id).await;
        make_editable(&db, pic_id).await;
        PictureRepository::set_exif_sync_status(&db, pic_id, status)
            .await
            .unwrap();
        let result = jobs::reextract_picture_exif(&db, &waker, alice_id, pic_id).await;
        assert!(
            matches!(result, Err(AppError::Conflict(_))),
            "{status:?} must be refused"
        );
    }
}

/// `extract_failed` is exactly the state a re-extract exists to settle.
#[sqlx::test(migrator = "MIGRATOR")]
async fn reextract_is_allowed_from_extract_failed(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    make_editable(&db, pic_id).await;
    PictureRepository::set_exif_sync_status(&db, pic_id, ExifSyncStatus::ExtractFailed)
        .await
        .unwrap();
    let waker = RoutineHandle::<Uuid>::disconnected();

    let job = jobs::reextract_picture_exif(&db, &waker, alice_id, pic_id)
        .await
        .unwrap();
    assert_eq!(job.picture_id, Some(pic_id));
    assert!(
        job.idempotency_key.is_none(),
        "the permanent initial-extraction key would 409 every re-extraction (§8)"
    );
    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::Extracting);

    // The in-flight guard takes the key's place: a second call conflicts.
    let again = jobs::reextract_picture_exif(&db, &waker, alice_id, pic_id).await;
    assert!(matches!(again, Err(AppError::Conflict(_))));
}
