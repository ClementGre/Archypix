//! Upload-time deduplication on the batch presign endpoint.
//!
//! A file whose client-computed SHA-256 already matches one of the caller's owned pictures is
//! reported as a duplicate (no S3 slot minted); any `initial_tags` are landed on the existing
//! picture and a trashed match is restored. New files still get a normal slot.

mod common;

use archypix_back::infra::config::Config;
use archypix_back::infra::pipeline::PipelineWaker;
use archypix_back::infra::redis::Cache;
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
    let waker = PipelineWaker::disconnected();

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
        },
        BatchUploadFile {
            filename: "fresh.jpg".to_string(),
            file_hash: Some("b".repeat(64)),
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
        &waker,
    )
    .await
    .unwrap();

    assert_eq!(outcomes.len(), 2);
    match &outcomes[0] {
        BatchUploadOutcome::Duplicate { picture_id } => assert_eq!(*picture_id, existing),
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
    let waker = PipelineWaker::disconnected();

    let user = common::seed_user(&db, "carol", "pw").await;

    // Two files with the same hash, neither yet in the DB — only the first gets a slot.
    let hash = "d".repeat(64);
    let files = vec![
        BatchUploadFile {
            filename: "first.jpg".to_string(),
            file_hash: Some(hash.clone()),
        },
        BatchUploadFile {
            filename: "copy.jpg".to_string(),
            file_hash: Some(hash.clone()),
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
        BatchUploadOutcome::Duplicate { picture_id } => assert_eq!(*picture_id, first_id),
        _ => panic!("second identical file should dedup within the batch"),
    }
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn batch_presign_restores_trashed_duplicate(db: PgPool) {
    let cfg = config();
    let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new());
    let storage = MockStorage::new();
    let waker = PipelineWaker::disconnected();

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
    }];

    let outcomes = begin_upload_batch(
        &db,
        cache.as_ref(),
        &storage,
        &cfg,
        user,
        &files,
        &[],
        &waker,
    )
    .await
    .unwrap();

    // Re-uploading a trashed photo dedups onto it and brings it back from the trash.
    match &outcomes[0] {
        BatchUploadOutcome::Duplicate { picture_id } => assert_eq!(*picture_id, trashed),
        _ => panic!("re-uploading a trashed photo should dedup onto it"),
    }
    let restored = PictureRepository::find_by_id(&db, trashed)
        .await
        .unwrap()
        .unwrap();
    assert!(
        restored.deleted_at.is_none(),
        "the trashed duplicate should be restored"
    );
}
