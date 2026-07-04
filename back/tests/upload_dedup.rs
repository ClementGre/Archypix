//! Upload-time deduplication on the batch presign endpoint.
//!
//! A file whose client-computed SHA-256 already matches one of the caller's owned pictures is
//! reported as a duplicate (no S3 slot minted); any `initial_tags` are landed on the existing
//! picture. A trashed match is flagged `was_deleted` (no longer auto-restored — feature 15) and,
//! with an import label, tagged `<label>.AlreadyExisting.Deleted`. New files still get a slot.

mod common;

use archypix_back::infra::config::Config;
use archypix_back::infra::redis::Cache;
use archypix_back::infra::routine::RoutineHandle;
use archypix_back::repository::picture::PictureRepository;
use archypix_back::services::pictures::{BatchUploadFile, BatchUploadOutcome, begin_upload_batch};
use common::{InMemoryCache, MockStorage};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

fn config() -> Config {
    Config::test_defaults()
}

/// Distinct tag paths (ltree wire form) stored on a picture.
async fn picture_tags(db: &PgPool, picture_id: Uuid) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT tag_path::text FROM tags WHERE picture_id = $1 ORDER BY tag_path",
    )
    .bind(picture_id)
    .fetch_all(db)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn batch_presign_dedups_known_hash_and_tags_existing(db: PgPool) {
    let cfg = config();
    let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new());
    let storage = MockStorage::new();
    let waker = RoutineHandle::<uuid::Uuid>::disconnected();

    let user = common::seed_user(&db, "alice", "pw").await;

    // An existing owned picture whose bytes hash to `existing_hash`.
    let existing = common::seed_picture(&db, user).await;
    let existing_hash = "a".repeat(64);
    PictureRepository::set_file_hash(&db, existing, &existing_hash, Some(123))
        .await
        .unwrap();

    let files = vec![
        BatchUploadFile {
            filename: "dup.jpg".to_string(),
            file_hash: Some(existing_hash.clone()),
            size: None,
        },
        BatchUploadFile {
            filename: "fresh.jpg".to_string(),
            file_hash: Some("b".repeat(64)),
            size: None,
        },
    ];
    let tags = vec!["Photos.Trip".to_string()];

    let outcomes = begin_upload_batch(
        &db,
        cache.as_ref(),
        &storage,
        &cfg,
        user,
        &files,
        &tags,
        None,
        &waker,
    )
    .await
    .unwrap();

    assert_eq!(outcomes.len(), 2);
    match &outcomes[0] {
        BatchUploadOutcome::Duplicate {
            picture_id,
            was_deleted,
        } => {
            assert_eq!(*picture_id, existing);
            assert!(!was_deleted, "live duplicate is not deleted");
        }
        _ => panic!("first file should dedup to the existing picture"),
    }
    match &outcomes[1] {
        BatchUploadOutcome::New {
            presigned_url,
            picture_id,
        } => {
            assert!(!presigned_url.is_empty());
            assert_ne!(*picture_id, existing);
        }
        _ => panic!("second file should be a fresh upload slot"),
    }

    // The dedup target picked up the initial tag.
    assert_eq!(picture_tags(&db, existing).await, vec!["Photos.Trip"]);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn batch_presign_dedups_identical_files_within_one_batch(db: PgPool) {
    let cfg = config();
    let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new());
    let storage = MockStorage::new();
    let waker = RoutineHandle::<uuid::Uuid>::disconnected();

    let user = common::seed_user(&db, "carol", "pw").await;

    // Two files with the same hash, neither yet in the DB — only the first gets a slot.
    let hash = "d".repeat(64);
    let files = vec![
        BatchUploadFile {
            filename: "first.jpg".to_string(),
            file_hash: Some(hash.clone()),
            size: None,
        },
        BatchUploadFile {
            filename: "copy.jpg".to_string(),
            file_hash: Some(hash.clone()),
            size: None,
        },
    ];

    let outcomes = begin_upload_batch(
        &db,
        cache.as_ref(),
        &storage,
        &cfg,
        user,
        &files,
        &[],
        None,
        &waker,
    )
    .await
    .unwrap();

    let first_id = match &outcomes[0] {
        BatchUploadOutcome::New { picture_id, .. } => *picture_id,
        _ => panic!("first file should be a fresh upload slot"),
    };
    // The second identical file dedups onto the first's (not-yet-created) picture.
    match &outcomes[1] {
        BatchUploadOutcome::Duplicate { picture_id, .. } => assert_eq!(*picture_id, first_id),
        _ => panic!("second identical file should dedup within the batch"),
    }
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn batch_presign_flags_and_tags_trashed_duplicate_without_restoring(db: PgPool) {
    let cfg = config();
    let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new());
    let storage = MockStorage::new();
    let waker = RoutineHandle::<uuid::Uuid>::disconnected();

    let user = common::seed_user(&db, "bob", "pw").await;

    let trashed = common::seed_picture(&db, user).await;
    let hash = "c".repeat(64);
    PictureRepository::set_file_hash(&db, trashed, &hash, Some(50))
        .await
        .unwrap();
    PictureRepository::set_deleted(&db, user, trashed, true)
        .await
        .unwrap();

    let files = vec![BatchUploadFile {
        filename: "again.jpg".to_string(),
        file_hash: Some(hash),
        size: None,
    }];

    let outcomes = begin_upload_batch(
        &db,
        cache.as_ref(),
        &storage,
        &cfg,
        user,
        &files,
        &[],
        Some("Uploaded.2026_06_25_14_30"),
        &waker,
    )
    .await
    .unwrap();

    // Re-uploading a trashed photo dedups onto it, flags it deleted, but does NOT restore it.
    match &outcomes[0] {
        BatchUploadOutcome::Duplicate {
            picture_id,
            was_deleted,
        } => {
            assert_eq!(*picture_id, trashed);
            assert!(was_deleted, "the matched picture is in the trash");
        }
        _ => panic!("re-uploading a trashed photo should dedup onto it"),
    }
    let still = PictureRepository::find_by_id(&db, trashed)
        .await
        .unwrap()
        .unwrap();
    assert!(
        still.deleted_at.is_some(),
        "the trashed duplicate must stay trashed (no auto-restore)"
    );
    // It is tagged with the deleted marker so the user can find and restore it.
    assert_eq!(
        picture_tags(&db, trashed).await,
        vec!["Uploaded.2026_06_25_14_30.AlreadyExisting.Deleted"]
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn batch_presign_tags_live_duplicate_already_existing(db: PgPool) {
    let cfg = config();
    let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new());
    let storage = MockStorage::new();
    let waker = RoutineHandle::<uuid::Uuid>::disconnected();

    let user = common::seed_user(&db, "dave", "pw").await;

    let existing = common::seed_picture(&db, user).await;
    let hash = "e".repeat(64);
    PictureRepository::set_file_hash(&db, existing, &hash, Some(77))
        .await
        .unwrap();

    let files = vec![BatchUploadFile {
        filename: "again.jpg".to_string(),
        file_hash: Some(hash),
        size: None,
    }];

    begin_upload_batch(
        &db,
        cache.as_ref(),
        &storage,
        &cfg,
        user,
        &files,
        &[],
        Some("Uploaded.2026_06_25_14_30"),
        &waker,
    )
    .await
    .unwrap();

    // A live (non-deleted) duplicate is tagged AlreadyExisting (not the Deleted marker).
    assert_eq!(
        picture_tags(&db, existing).await,
        vec!["Uploaded.2026_06_25_14_30.AlreadyExisting"]
    );
}
