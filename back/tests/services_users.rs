mod common;

use archypix_back::infra::config::Config;
use archypix_back::infra::error::AppError;
use archypix_back::services::users;
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
    )
    .await;
    assert!(matches!(result, Err(AppError::BadRequest(_))));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_user_rejects_empty_password(db: PgPool) {
    let result = users::create_user(&db, "alice", "alice@test.com", "Alice", "", false, None).await;
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
    let config = Config::test_defaults();

    // Different instance → short-circuit before any DB hit
    let result = users::find_local_user_id(&cache, &db, &config, "alice", "other.com")
        .await
        .unwrap();
    assert!(result.is_none());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn find_local_user_id_returns_some_for_existing_local_user(db: PgPool) {
    let cache = common::InMemoryCache::new();
    let config = Config::test_defaults();
    let alice_id = common::seed_user(&db, "alice", "pass").await;

    let result = users::find_local_user_id(&cache, &db, &config, "alice", "test.com")
        .await
        .unwrap();
    assert_eq!(result, Some(alice_id));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn find_local_user_id_returns_none_for_unknown_username(db: PgPool) {
    let cache = common::InMemoryCache::new();
    let config = Config::test_defaults();

    let result = users::find_local_user_id(&cache, &db, &config, "nobody", "test.com")
        .await
        .unwrap();
    assert!(result.is_none());
}
