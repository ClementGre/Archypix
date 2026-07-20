use crate::domain::auth::TokenType;
use crate::domain::user::User;
use crate::infra::crypto::{
    generate_refresh_token, hash_refresh_token, verify_password, verify_password_dummy, JwtService,
};
use crate::infra::ratelimit;
use crate::infra::redis::Cache;
use crate::infra::settings::keys;
use crate::repository::auth::{CredentialRepository, RefreshTokenRepository};
use crate::repository::user::UserRepository;
use archypix_common::error::AppError;
use archypix_common::settings::Settings;
use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

pub struct AuthTokens {
    pub access_token: String,
    pub refresh_token: String,
}

#[tracing::instrument(skip(db, cache, jwt, settings, password))]
pub async fn login(
    db: &PgPool,
    cache: &dyn Cache,
    jwt: &JwtService,
    settings: &Settings,
    username: &str,
    password: &str,
) -> Result<AuthTokens, AppError> {
    // Throttle credential-stuffing / brute-force per username
    ratelimit::check_categorized(
        cache,
        ratelimit::category::LOGIN,
        &format!("login:{username}"),
        settings.get(keys::RATE_LIMIT_LOGIN_MAX),
        settings.get(keys::RATE_LIMIT_LOGIN_WINDOW_SECS),
        settings.get(keys::RATE_LIMIT_EVENT_RETENTION_SECS),
    )
    .await?;

    let user = UserRepository::find_by_username(db, username).await?;
    let hash = match &user {
        Some(u) => CredentialRepository::get_password_hash(db, u.id).await?,
        None => None,
    };

    // Always run exactly one Argon2 verification: the response time does not reveal whether the username exists.
    let valid = match &hash {
        Some(h) => verify_password(password, h)?,
        None => {
            verify_password_dummy(password);
            false
        }
    };

    match (user, valid) {
        (Some(user), true) => issue_tokens(db, jwt, settings, &user).await,
        _ => Err(AppError::Unauthorized("Invalid credentials".to_string())),
    }
}

#[tracing::instrument(skip(db, jwt, settings, refresh_token_raw))]
pub async fn refresh(
    db: &PgPool,
    jwt: &JwtService,
    settings: &Settings,
    refresh_token_raw: &str,
) -> Result<AuthTokens, AppError> {
    let token_hash = hash_refresh_token(refresh_token_raw);
    let stored = RefreshTokenRepository::find_valid(db, &token_hash)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid refresh token".to_string()))?;

    RefreshTokenRepository::revoke(db, stored.id).await?;

    let user = UserRepository::find_by_id(db, stored.user_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized("User not found".to_string()))?;

    issue_tokens(db, jwt, settings, &user).await
}

#[tracing::instrument(skip(db, refresh_token_raw), fields(user_id = ?user_id))]
pub async fn logout(
    db: &PgPool,
    user_id: Option<Uuid>,
    refresh_token_raw: Option<&str>,
) -> Result<(), AppError> {
    if let Some(raw) = refresh_token_raw {
        let hash = hash_refresh_token(raw);
        if let Some(stored) = RefreshTokenRepository::find_valid(db, &hash).await? {
            RefreshTokenRepository::revoke(db, stored.id).await?;
        }
    } else if let Some(uid) = user_id {
        RefreshTokenRepository::revoke_all_for_user(db, uid).await?;
    }
    Ok(())
}

async fn issue_tokens(
    db: &PgPool,
    jwt: &JwtService,
    settings: &Settings,
    user: &User,
) -> Result<AuthTokens, AppError> {
    let access_token = jwt.issue(
        &user.username,
        Some(user.id),
        &settings.get(keys::GLOBAL_DOMAIN),
        TokenType::User,
        user.is_admin,
        &settings.get(keys::BACK_DOMAIN),
        settings.get(keys::ACCESS_TOKEN_TTL_SECS),
    )?;

    let refresh_token_raw = generate_refresh_token();
    let refresh_hash = hash_refresh_token(&refresh_token_raw);
    let expires_at = Utc::now() + Duration::seconds(settings.get(keys::REFRESH_TOKEN_TTL_SECS));
    RefreshTokenRepository::create(db, user.id, &refresh_hash, expires_at).await?;

    Ok(AuthTokens {
        access_token,
        refresh_token: refresh_token_raw,
    })
}
