mod common;

use archypix_back::infra::config::Config;
use archypix_back::infra::crypto::JwtService;
use archypix_back::services::auth;
use sqlx::PgPool;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

fn test_config() -> Config {
    Config::test_defaults()
}

fn test_jwt(config: &Config) -> JwtService {
    JwtService::new(&config.jwt_secret, &config.back_domain)
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn login_correct_credentials_returns_tokens(db: PgPool) {
    common::seed_user(&db, "alice", "secret123").await;
    let config = test_config();
    let jwt = test_jwt(&config);
    let cache = common::InMemoryCache::new();

    let tokens = auth::login(&db, &cache, &jwt, &config, "alice", "secret123")
        .await
        .expect("login should succeed");

    assert!(!tokens.access_token.is_empty());
    assert!(!tokens.refresh_token.is_empty());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn login_wrong_password_is_rejected(db: PgPool) {
    common::seed_user(&db, "alice", "secret123").await;
    let config = test_config();
    let jwt = test_jwt(&config);
    let cache = common::InMemoryCache::new();

    let result = auth::login(&db, &cache, &jwt, &config, "alice", "wrong_password").await;
    assert!(result.is_err());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn login_unknown_user_is_rejected(db: PgPool) {
    let config = test_config();
    let jwt = test_jwt(&config);
    let cache = common::InMemoryCache::new();

    let result = auth::login(&db, &cache, &jwt, &config, "nobody", "any").await;
    assert!(result.is_err());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn login_is_rate_limited_per_username(db: PgPool) {
    common::seed_user(&db, "alice", "secret123").await;
    let mut config = test_config();
    config.rate_limit_login_max = 2;
    let jwt = test_jwt(&config);
    let cache = common::InMemoryCache::new();

    // First two (wrong) attempts are allowed through to credential verification → Unauthorized.
    for _ in 0..2 {
        let r = auth::login(&db, &cache, &jwt, &config, "alice", "wrong").await;
        assert!(matches!(
            r,
            Err(archypix_back::infra::error::AppError::Unauthorized(_))
        ));
    }
    // The third attempt is throttled before any verification → TooManyRequests, even with the
    // correct password.
    let r = auth::login(&db, &cache, &jwt, &config, "alice", "secret123").await;
    assert!(
        matches!(
            r,
            Err(archypix_back::infra::error::AppError::TooManyRequests(_))
        ),
        "third attempt must be rate limited even with the correct password"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn refresh_rotates_token(db: PgPool) {
    common::seed_user(&db, "alice", "secret123").await;
    let config = test_config();
    let jwt = test_jwt(&config);
    let cache = common::InMemoryCache::new();

    let first = auth::login(&db, &cache, &jwt, &config, "alice", "secret123")
        .await
        .unwrap();

    let second = auth::refresh(&db, &jwt, &config, &first.refresh_token)
        .await
        .expect("refresh should succeed");

    assert_ne!(
        first.refresh_token, second.refresh_token,
        "token must rotate"
    );
    assert!(!second.access_token.is_empty());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn refresh_old_token_after_rotation_is_rejected(db: PgPool) {
    common::seed_user(&db, "alice", "secret123").await;
    let config = test_config();
    let jwt = test_jwt(&config);
    let cache = common::InMemoryCache::new();

    let first = auth::login(&db, &cache, &jwt, &config, "alice", "secret123")
        .await
        .unwrap();

    // Use refresh token once
    auth::refresh(&db, &jwt, &config, &first.refresh_token)
        .await
        .unwrap();

    // Reusing the old token must fail
    let result = auth::refresh(&db, &jwt, &config, &first.refresh_token).await;
    assert!(result.is_err(), "revoked token must be rejected");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn logout_specific_token_revokes_it(db: PgPool) {
    let user_id = common::seed_user(&db, "alice", "secret123").await;
    let config = test_config();
    let jwt = test_jwt(&config);
    let cache = common::InMemoryCache::new();

    let tokens = auth::login(&db, &cache, &jwt, &config, "alice", "secret123")
        .await
        .unwrap();

    auth::logout(&db, Some(user_id), Some(&tokens.refresh_token))
        .await
        .unwrap();

    let result = auth::refresh(&db, &jwt, &config, &tokens.refresh_token).await;
    assert!(result.is_err(), "logged-out token must be rejected");
}
