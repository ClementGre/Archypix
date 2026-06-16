mod common;

use archypix_back::domain::job::{ExifField, ExifOverrides};
use archypix_back::domain::picture::ExifSyncStatus;
use archypix_back::infra::error::AppError;
use archypix_back::infra::pipeline::PipelineWaker;
use archypix_back::repository::picture::PictureRepository;
use archypix_back::services::jobs;
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

fn gps_edit() -> (ExifOverrides, Vec<ExifField>) {
    (
        ExifOverrides {
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
    let waker = PipelineWaker::disconnected();

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
        None,
        None,
        None,
        None,
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
    let waker = PipelineWaker::disconnected();

    let (set, clear) = gps_edit();
    let result = jobs::edit_pictures_exif(&db, &waker, bob_id, &[alice_pic_id], set, clear).await;
    assert!(
        matches!(result, Err(AppError::NotFound)),
        "bob must not enqueue edit for alice's picture"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn enqueue_edit_rejects_still_processing_picture(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await; // no thumbnails_generated_at
    let waker = PipelineWaker::disconnected();

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
    let waker = PipelineWaker::disconnected();

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
    let waker = PipelineWaker::disconnected();

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
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::Unsupported);
    assert_eq!(picture.gps_lat, Some(45.0), "DB still updated");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn set_and_clear_conflict_is_rejected(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    make_editable(&db, pic_id).await;
    let waker = PipelineWaker::disconnected();

    let set = ExifOverrides {
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
