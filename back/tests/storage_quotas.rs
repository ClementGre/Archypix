//! Storage-quota accounting & enforcement (feature 22).
//!
//! Covers the trigger-maintained `user_storage` counters, the reconcile recompute, the effective
//! usage (committed + reserved) math, and the enforcement points (`begin_upload` reservation +
//! `complete_upload` hard check + `copy_picture`).

mod common;

use archypix_back::infra::config::Config;
use archypix_back::infra::error::AppError;
use archypix_back::infra::redis::Cache;
use archypix_back::infra::s3::Storage;
use archypix_back::repository::user_storage::UserStorageRepository;
use archypix_back::services::storage;
use common::{InMemoryCache, MockStorage};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

fn config() -> Config {
    Config::test_defaults()
}

/// Insert an owned picture with a concrete `file_size`.
async fn seed_sized_picture(db: &PgPool, user_id: Uuid, size: i64) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO pictures (id, local_user_id, file_size) VALUES ($1, $2, $3)",
        id,
        user_id,
        size,
    )
    .execute(db)
    .await
    .unwrap();
    id
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn triggers_track_originals_versions_and_trash(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;

    let pic = seed_sized_picture(&db, user, 1000).await;
    assert_eq!(
        UserStorageRepository::get(&db, user)
            .await
            .unwrap()
            .billed_total(),
        1000
    );

    // A version bills into versions_bytes.
    sqlx::query!(
        "INSERT INTO picture_versions (id, picture_id, version_number, file_size) VALUES ($1,$2,1,300)",
        Uuid::new_v4(),
        pic,
    )
        .execute(&db)
        .await
        .unwrap();
    let s = UserStorageRepository::get(&db, user).await.unwrap();
    assert_eq!((s.originals_bytes, s.versions_bytes), (1000, 300));
    assert_eq!(s.billed_total(), 1300);

    // Trash moves both the original and the version into the trashed buckets (byte-neutral total).
    sqlx::query!("UPDATE pictures SET deleted_at = now() WHERE id = $1", pic)
        .execute(&db)
        .await
        .unwrap();
    let s = UserStorageRepository::get(&db, user).await.unwrap();
    assert_eq!(
        (
            s.originals_bytes,
            s.originals_trashed_bytes,
            s.versions_bytes,
            s.versions_trashed_bytes
        ),
        (0, 1000, 0, 300)
    );
    assert_eq!(s.reclaimable_trash_bytes(), 1300);

    // Hard delete frees everything.
    sqlx::query!("DELETE FROM pictures WHERE id = $1", pic)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        UserStorageRepository::get(&db, user)
            .await
            .unwrap()
            .billed_total(),
        0
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn received_pictures_are_never_billed(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    sqlx::query!(
        "INSERT INTO pictures (id, local_user_id, file_size, remote_picture_id, owner_username, owner_instance_domain)
         VALUES ($1,$2,$3,'remote-1','bob','other.test')",
        Uuid::new_v4(),
        user,
        5_000_000i64,
    )
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        UserStorageRepository::get(&db, user)
            .await
            .unwrap()
            .billed_total(),
        0
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn reconcile_matches_trigger_counters(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    seed_sized_picture(&db, user, 700).await;
    let trashed = seed_sized_picture(&db, user, 200).await;
    sqlx::query!(
        "UPDATE pictures SET deleted_at = now() WHERE id = $1",
        trashed
    )
    .execute(&db)
    .await
    .unwrap();

    let before = UserStorageRepository::get(&db, user).await.unwrap();

    // Corrupt the counters, then reconcile back to truth.
    sqlx::query!(
        "UPDATE user_storage SET originals_bytes = 999999 WHERE user_id = $1",
        user
    )
    .execute(&db)
    .await
    .unwrap();
    UserStorageRepository::reconcile_all(&db).await.unwrap();

    let after = UserStorageRepository::get(&db, user).await.unwrap();
    assert_eq!(after.originals_bytes, before.originals_bytes);
    assert_eq!(
        after.originals_trashed_bytes,
        before.originals_trashed_bytes
    );
    assert_eq!(after.billed_total(), 900);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn quota_and_reservation_math(db: PgPool) {
    let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new());
    let cfg = config();
    let user = common::seed_user(&db, "alice", "pw").await;
    seed_sized_picture(&db, user, 1000).await;

    // Unlimited by default → everything fits.
    assert!(
        storage::fits(cache.as_ref(), &db, user, 1_000_000_000)
            .await
            .unwrap()
    );

    // Cap at 1500 bytes: 400 more fits (1000 + 400), 600 does not.
    UserStorageRepository::set_quota(&db, user, Some(1500))
        .await
        .unwrap();
    storage::invalidate_committed(cache.as_ref(), user).await;
    assert!(storage::fits(cache.as_ref(), &db, user, 400).await.unwrap());
    assert!(!storage::fits(cache.as_ref(), &db, user, 600).await.unwrap());

    // A reservation counts against the effective usage: reserve 400 → only 100 headroom left.
    let pic = Uuid::new_v4();
    storage::reserve(cache.as_ref(), &cfg, user, pic, 400)
        .await
        .unwrap();
    assert!(storage::fits(cache.as_ref(), &db, user, 100).await.unwrap());
    assert!(!storage::fits(cache.as_ref(), &db, user, 200).await.unwrap());

    // Releasing the reservation restores the headroom.
    storage::release(cache.as_ref(), user, pic).await;
    assert!(storage::fits(cache.as_ref(), &db, user, 400).await.unwrap());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn complete_upload_hard_check_rejects_over_quota(db: PgPool) {
    let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new());
    let cfg = config();
    let storage_mock = MockStorage::new();
    let user = common::seed_user(&db, "alice", "pw").await;

    UserStorageRepository::set_quota(&db, user, Some(500))
        .await
        .unwrap();

    // Presign a slot, then stage bytes larger than the quota under the staging key.
    let (picture_id, _url) = archypix_back::services::pictures::begin_upload(
        &db,
        cache.as_ref(),
        &storage_mock,
        &cfg,
        user,
        "big.jpg",
        None, // no declared size → coarse gate only, hard check is the backstop
    )
    .await
    .unwrap();
    let staging_key = format!("staging/{}/{}", user, picture_id);
    storage_mock
        .put_object(&cfg.s3_bucket_staging, &staging_key, vec![0u8; 800], None)
        .await
        .unwrap();

    let err = archypix_back::services::pictures::complete_upload(
        &db,
        cache.as_ref(),
        &storage_mock,
        &cfg,
        user,
        picture_id,
        archypix_back::services::pictures::UploadMetadata {
            mime_type: None,
            file_size: Some(800),
            file_hash: None,
            width: None,
            height: None,
            exif_data: None,
            captured_at: None,
            initial_tags: None,
            upload_label: None,
            defer_pipeline: true,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, AppError::PayloadTooLarge(_)), "got {err:?}");

    // No row created, and the promoted object was cleaned up (no orphan bytes).
    assert_eq!(
        UserStorageRepository::get(&db, user)
            .await
            .unwrap()
            .billed_total(),
        0
    );
    let pics_key = format!("{}/{}", user, picture_id);
    assert!(
        storage_mock
            .get(&cfg.s3_bucket_pictures, &pics_key)
            .is_none()
    );
}
