mod common;

use archypix_back::infra::settings::test_settings_with;
use archypix_back::services::users;
use archypix_common::error::AppError;
use sqlx::PgPool;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_user_rejects_uppercase_username(db: PgPool) {
    let result = users::create_user(
        &db,
        "Alice",
        "alice@test.com",
        "Alice",
        "password",
        false,
        None,
        None,
    )
    .await;
    assert!(matches!(result, Err(AppError::BadRequest(_))));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_user_rejects_empty_password(db: PgPool) {
    let result = users::create_user(
        &db,
        "alice",
        "alice@test.com",
        "Alice",
        "",
        false,
        None,
        None,
    )
        .await;
    assert!(matches!(result, Err(AppError::BadRequest(_))));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_user_fails_on_duplicate_username(db: PgPool) {
    users::create_user(
        &db,
        "alice",
        "alice@test.com",
        "Alice",
        "password1",
        false,
        None,
        None,
    )
    .await
    .unwrap();
    let result = users::create_user(
        &db,
        "alice",
        "alice2@test.com",
        "Alice2",
        "password2",
        false,
        None,
        None,
    )
    .await;
    assert!(
        matches!(result, Err(AppError::Conflict(_))),
        "duplicate username must return Conflict"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn find_local_user_id_returns_none_for_different_instance(db: PgPool) {
    let cache = common::InMemoryCache::new();
    let settings = test_settings_with(&[]);

    // Different instance → short-circuit before any DB hit
    let result = users::find_local_user_id(&cache, &db, &settings, "alice", "other.com")
        .await
        .unwrap();
    assert!(result.is_none());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn find_local_user_id_returns_some_for_existing_local_user(db: PgPool) {
    let cache = common::InMemoryCache::new();
    let settings = test_settings_with(&[]);
    let alice_id = common::seed_user(&db, "alice", "pass").await;

    let result = users::find_local_user_id(&cache, &db, &settings, "alice", "test.com")
        .await
        .unwrap();
    assert_eq!(result, Some(alice_id));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn find_local_user_id_returns_none_for_unknown_username(db: PgPool) {
    let cache = common::InMemoryCache::new();
    let settings = test_settings_with(&[]);

    let result = users::find_local_user_id(&cache, &db, &settings, "nobody", "test.com")
        .await
        .unwrap();
    assert!(result.is_none());
}
