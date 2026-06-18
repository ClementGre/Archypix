use crate::domain::user::User;
use crate::infra::config::Config;
use crate::infra::crypto::hash_password;
use crate::infra::error::{AppError, map_sqlx_error};
use crate::infra::redis::{Cache, RedisKey};
use crate::repository::auth::CredentialRepository;
use crate::repository::user::UserRepository;
use sqlx::PgPool;
use tracing::trace;
use uuid::Uuid;

/// Cache-aside lookup: returns the local UUID for `username@instance` if that user lives
/// on this backend, or `None` if they are remote.
///
/// `instance` is the user's global domain. If it differs from `config.global_domain` the
/// user is definitively remote and `None` is returned immediately.
///
/// Within the same global domain the lookup falls through to a cache-aside DB query.
/// A negative result (user not on this backend) is cached under the sentinel `"none"` to
/// avoid repeated DB hits when listing pictures owned by a remote user on the same domain.
pub async fn find_local_user_id(
    cache: &dyn Cache,
    db: &PgPool,
    config: &Config,
    username: &str,
    instance: &str,
) -> Result<Option<Uuid>, AppError> {
    if instance != config.global_domain {
        return Ok(None);
    }

    const NEGATIVE: &str = "none";
    let key = RedisKey::UserByUsername(username);

    if let Some(cached) = cache.get_str(key).await.ok().flatten() {
        return Ok(if cached == NEGATIVE {
            None
        } else {
            cached.parse::<Uuid>().ok()
        });
    }

    let found = UserRepository::find_by_username(db, username).await?;
    let value = found
        .as_ref()
        .map_or_else(|| NEGATIVE.to_string(), |u| u.id.to_string());
    let _ = cache
        .set_str_ex(key, &value, config.federation_backend_cache_ttl_secs)
        .await;

    Ok(found.map(|u| u.id))
}

pub async fn create_user(
    db: &PgPool,
    username: &str,
    email: &str,
    display_name: &str,
    password: &str,
    is_admin: bool,
) -> Result<User, AppError> {
    trace!(username, email, is_admin, "users: create_user");
    if username.trim().is_empty()
        || !username
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(AppError::BadRequest(
            "Username must be non-empty and match [a-z0-9_]".to_string(),
        ));
    }
    crate::domain::validation::validate_password(password).map_err(AppError::BadRequest)?;
    crate::domain::validation::validate_email(email).map_err(AppError::BadRequest)?;

    let password_hash = hash_password(password)?;

    let mut tx = db.begin().await.map_err(map_sqlx_error)?;
    let user = UserRepository::create(&mut *tx, username, email, display_name, is_admin).await?;
    CredentialRepository::upsert_password(&mut *tx, user.id, &password_hash).await?;
    tx.commit().await.map_err(map_sqlx_error)?;

    Ok(user)
}
