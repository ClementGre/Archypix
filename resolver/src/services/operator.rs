//! Operator dashboard credential (feature 23 §5.1): a single root-style token → short-lived
//! `ResolverAdminSession` JWT + a 1-month auto-rotating refresh token. No user accounts — the token
//! *is* the credential.

use crate::config::{Config, setting_keys as sk};
use crate::repository;
use archypix_common::auth::JwtService;
use archypix_common::auth::TokenType;
use archypix_common::error::AppError;
use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

const SESSION_TTL_SECS: i64 = 900;
const REFRESH_TTL_DAYS: i64 = 30;

fn argon_hash(secret: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(secret.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::InternalServerError(e.to_string()))
}

fn argon_verify(secret: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|p| {
            Argon2::default()
                .verify_password(secret.as_bytes(), &p)
                .is_ok()
        })
        .unwrap_or(false)
}

fn is_argon_hash(s: &str) -> bool {
    s.starts_with("$argon2")
}

fn random_token() -> String {
    let b: [u8; 32] = rand::random();
    hex::encode(b)
}
fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::new().chain_update(s.as_bytes()).finalize())
}

/// Seed the operator credential at startup if absent. `RESOLVER_ADMIN_TOKEN` may be plaintext or an
/// argon2 hash; unset ⇒ generate one and print it **once**.
pub async fn ensure_seeded(db: &PgPool, config: &Config) -> anyhow::Result<()> {
    let current_hash = repository::get_admin(db).await?.map(|c| c.token_hash);

    let token_hash = match config.get(sk::RESOLVER_ADMIN_TOKEN) {
        Some(t) if is_argon_hash(&t) => t,
        Some(t) => {
            tracing::warn!("RESOLVER_ADMIN_TOKEN is plaintext; storing an argon2 hash of it.");
            argon_hash(&t).map_err(|e| anyhow::anyhow!(e.to_string()))?
        }
        None => {
            let generated = random_token();
            let hash = argon_hash(&generated).map_err(|e| anyhow::anyhow!(e.to_string()))?;
            if Some(&hash) != current_hash.as_ref() {
                tracing::warn!(
                    "No RESOLVER_ADMIN_TOKEN set — generated operator token (shown once):\n\n    {generated}\n"
                );
            }
            hash
        }
    };
    if Some(&token_hash) != current_hash.as_ref() {
        tracing::info!("Seeding operator credential in database");
        repository::upsert_admin_token(db, &token_hash).await?;
    }
    Ok(())
}

pub struct Session {
    pub session_token: String,
    pub refresh_token: String,
    pub expires_in_secs: i64,
}

/// Verify the operator token and mint a session + refresh token.
pub async fn login(
    db: &PgPool,
    jwt: &JwtService,
    global_domain: &str,
    token: &str,
) -> Result<Session, AppError> {
    let cred = repository::get_admin(db)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Operator credential not initialised".to_string()))?;
    if !argon_verify(token, &cred.token_hash) {
        return Err(AppError::Unauthorized("Invalid operator token".to_string()));
    }
    mint_session(db, jwt, global_domain).await
}

/// Verify + rotate a refresh token, minting a fresh session + refresh token.
pub async fn refresh(
    db: &PgPool,
    jwt: &JwtService,
    global_domain: &str,
    refresh_token: &str,
) -> Result<Session, AppError> {
    let cred = repository::get_admin(db)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Operator credential not initialised".to_string()))?;
    let ok = cred
        .refresh_token_hash
        .as_deref()
        .map(|h| h == sha256_hex(refresh_token))
        .unwrap_or(false)
        && cred
            .refresh_expires_at
            .map(|e| e > Utc::now())
            .unwrap_or(false);
    if !ok {
        return Err(AppError::Unauthorized(
            "Invalid or expired refresh token".to_string(),
        ));
    }
    mint_session(db, jwt, global_domain).await
}

async fn mint_session(
    db: &PgPool,
    jwt: &JwtService,
    global_domain: &str,
) -> Result<Session, AppError> {
    let session_token = jwt
        .issue(
            "operator",
            None,
            global_domain,
            TokenType::ResolverAdminSession,
            true,
            global_domain,
            SESSION_TTL_SECS,
        )
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;
    let refresh_token = random_token();
    let expires_at = Utc::now() + Duration::days(REFRESH_TTL_DAYS);
    repository::set_admin_refresh(db, &sha256_hex(&refresh_token), expires_at).await?;
    Ok(Session {
        session_token,
        refresh_token,
        expires_in_secs: SESSION_TTL_SECS,
    })
}
