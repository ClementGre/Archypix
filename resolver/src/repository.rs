//! Resolver SQL layer. Compile-time checked queries (`query!`/`query_as!`/`query_scalar!`, offline
//! cache in `.sqlx`) over the `backends`, `user_mappings`, `invites`, `resolver_admin`, and
//! `resolver_settings` tables.

use archypix_common::error::AppError;
use archypix_common::registration::Invite;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
// ── Runtime settings overrides (resolver_settings) ──────────────────────────────

pub async fn load_settings_overrides(db: &PgPool) -> Result<HashMap<String, Value>, AppError> {
    let rows = sqlx::query!("SELECT key, value FROM resolver_settings")
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(|r| (r.key, r.value)).collect())
}

pub async fn upsert_setting(db: &PgPool, key: &str, value: &Value) -> Result<(), AppError> {
    sqlx::query!(
        "INSERT INTO resolver_settings (key, value, updated_at) VALUES ($1, $2, now())
         ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = now()",
        key,
        value,
    )
        .execute(db)
        .await?;
    Ok(())
}

pub async fn delete_setting(db: &PgPool, key: &str) -> Result<(), AppError> {
    sqlx::query!("DELETE FROM resolver_settings WHERE key = $1", key)
        .execute(db)
        .await?;
    Ok(())
}

/// A backend row incl. heartbeat state + capacity policy (feature 23 §10.2).
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Backend {
    pub back_domain: String,
    pub use_https: bool,
    #[serde(skip)]
    pub internal_url: String,
    /// Never serialised to the dashboard.
    #[serde(skip)]
    pub delegation_token: Option<String>,
    pub delegation_expires_at: Option<DateTime<Utc>>,
    pub user_count: i64,
    pub picture_count: i64,
    pub storage_bytes: i64,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub healthy: bool,
    pub reachable: bool,
    pub accepting_registrations: bool,
    pub max_users: Option<i64>,
    pub version: Option<String>,
    pub last_selected_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl Backend {
    pub fn public_url(&self) -> String {
        let scheme = if self.use_https { "https" } else { "http" };
        format!("{scheme}://{}", self.back_domain)
    }
    /// Eligible for new-user placement (feature 23 §7.3): reachable, accepting, and under capacity.
    pub fn is_eligible(&self) -> bool {
        self.reachable
            && self.accepting_registrations
            && self.max_users.map(|m| self.user_count < m).unwrap_or(true)
    }
}

// ── Backends ────────────────────────────────────────────────────────────────

pub async fn upsert_backend(
    db: &PgPool,
    back_domain: &str,
    use_https: bool,
    internal_url: &str,
) -> Result<(), AppError> {
    sqlx::query!(
        "INSERT INTO backends (back_domain, use_https, internal_url) VALUES ($1, $2, $3)
         ON CONFLICT (back_domain) DO UPDATE SET use_https = $2, internal_url = $3",
        back_domain,
        use_https,
        internal_url,
    )
        .execute(db)
        .await?;
    Ok(())
}

/// Store a heartbeat: delegation token + metrics, and mark the backend reachable (feature 23 §3.2).
#[allow(clippy::too_many_arguments)]
pub async fn record_heartbeat(
    db: &PgPool,
    back_domain: &str,
    delegation_token: &str,
    delegation_expires_at: DateTime<Utc>,
    user_count: i64,
    picture_count: i64,
    storage_bytes: i64,
    healthy: bool,
    version: &str,
) -> Result<bool, AppError> {
    let res = sqlx::query!(
        "UPDATE backends SET delegation_token = $2, delegation_expires_at = $3, user_count = $4,
            picture_count = $5, storage_bytes = $6, healthy = $7, version = $8,
            last_heartbeat_at = now(), reachable = true
         WHERE back_domain = $1",
        back_domain,
        delegation_token,
        delegation_expires_at,
        user_count,
        picture_count,
        storage_bytes,
        healthy,
        version,
    )
        .execute(db)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn list_backends(db: &PgPool) -> Result<Vec<Backend>, AppError> {
    Ok(
        sqlx::query_as!(Backend, "SELECT * FROM backends ORDER BY created_at ASC")
            .fetch_all(db)
            .await?,
    )
}

pub async fn get_backend(db: &PgPool, back_domain: &str) -> Result<Option<Backend>, AppError> {
    Ok(sqlx::query_as!(
        Backend,
        "SELECT * FROM backends WHERE back_domain = $1",
        back_domain
    )
        .fetch_optional(db)
        .await?)
}

pub async fn set_capacity(
    db: &PgPool,
    back_domain: &str,
    accepting_registrations: bool,
    max_users: Option<i64>,
) -> Result<(), AppError> {
    sqlx::query!(
        "UPDATE backends SET accepting_registrations = $2, max_users = $3 WHERE back_domain = $1",
        back_domain,
        accepting_registrations,
        max_users,
    )
        .execute(db)
        .await?;
    Ok(())
}

/// Round-robin cursor: mark a backend as just-selected.
pub async fn touch_selected(db: &PgPool, back_domain: &str) -> Result<(), AppError> {
    sqlx::query!(
        "UPDATE backends SET last_selected_at = now() WHERE back_domain = $1",
        back_domain
    )
        .execute(db)
        .await?;
    Ok(())
}

/// Mark reachable backends unreachable once their delegation token is past expiry (feature 23 §8.3).
pub async fn prune_stale(db: &PgPool) -> Result<u64, AppError> {
    let res = sqlx::query!(
        "UPDATE backends SET reachable = false
         WHERE reachable = true AND (delegation_expires_at IS NULL OR delegation_expires_at < now())",
    )
        .execute(db)
        .await?;
    Ok(res.rows_affected())
}

pub async fn fleet_totals(db: &PgPool) -> Result<(i64, i64, i64), AppError> {
    let row = sqlx::query!(
        r#"SELECT SUM(user_count)::BIGINT AS user_count, SUM(picture_count)::BIGINT AS picture_count,
           SUM(storage_bytes)::BIGINT AS storage_bytes FROM backends"#,
    )
        .fetch_one(db)
        .await?;
    Ok((
        row.user_count.unwrap_or(0),
        row.picture_count.unwrap_or(0),
        row.storage_bytes.unwrap_or(0),
    ))
}

// ── User mappings ─────────────────────────────────────────────────────────────

pub async fn get_backend_url(db: &PgPool, username: &str) -> Result<Option<String>, AppError> {
    Ok(sqlx::query_scalar!(
        "SELECT CASE WHEN b.use_https THEN 'https://' ELSE 'http://' END || b.back_domain
         FROM user_mappings u JOIN backends b ON u.back_domain = b.back_domain
         WHERE u.username = $1",
        username,
    )
        .fetch_optional(db)
        .await?
        .flatten())
}

pub async fn upsert_mapping(
    db: &PgPool,
    username: &str,
    back_domain: &str,
) -> Result<(), AppError> {
    sqlx::query!(
        "INSERT INTO user_mappings (username, back_domain, updated_at) VALUES ($1, $2, now())
         ON CONFLICT (username) DO UPDATE SET back_domain = $2, updated_at = now()",
        username,
        back_domain,
    )
        .execute(db)
        .await?;
    Ok(())
}

/// All `(username, back_domain)` mappings — the reconcile routine's current-state snapshot.
pub async fn list_mappings(db: &PgPool) -> Result<Vec<(String, String)>, AppError> {
    let rows = sqlx::query!("SELECT username, back_domain FROM user_mappings")
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(|r| (r.username, r.back_domain)).collect())
}

/// Delete the given usernames' mappings (reconcile pruning of deleted users). Returns rows affected.
pub async fn delete_mappings(db: &PgPool, usernames: &[String]) -> Result<u64, AppError> {
    if usernames.is_empty() {
        return Ok(0);
    }
    let res = sqlx::query!(
        "DELETE FROM user_mappings WHERE username = ANY($1)",
        usernames
    )
        .execute(db)
        .await?;
    Ok(res.rows_affected())
}

pub async fn username_exists(db: &PgPool, username: &str) -> Result<bool, AppError> {
    let n = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM user_mappings WHERE username = $1",
        username
    )
        .fetch_one(db)
        .await?;
    Ok(n.unwrap_or(0) > 0)
}

// ── Invites ─────────────────────────────────────────────────────────────────

struct InviteRow {
    code: String,
    max_uses: Option<i64>,
    uses: i64,
    expires_at: Option<DateTime<Utc>>,
    created_by: String,
    instance_pin: Option<String>,
}

impl From<InviteRow> for Invite {
    fn from(r: InviteRow) -> Self {
        Invite {
            code: r.code,
            max_uses: r.max_uses,
            uses: r.uses,
            expires_at: r.expires_at,
            created_by: r.created_by,
            instance_pin: r.instance_pin,
        }
    }
}

pub async fn create_invite(
    db: &PgPool,
    code: &str,
    max_uses: Option<i64>,
    expires_at: Option<DateTime<Utc>>,
    created_by: &str,
    instance_pin: Option<&str>,
) -> Result<Invite, AppError> {
    let row = sqlx::query_as!(
        InviteRow,
        "INSERT INTO invites (code, max_uses, expires_at, created_by, instance_pin)
         VALUES ($1, $2, $3, $4, $5) RETURNING code, max_uses, uses, expires_at, created_by, instance_pin",
        code,
        max_uses,
        expires_at,
        created_by,
        instance_pin,
    )
        .fetch_one(db)
        .await?;
    Ok(row.into())
}

pub async fn list_invites(db: &PgPool) -> Result<Vec<Invite>, AppError> {
    let rows = sqlx::query_as!(
        InviteRow,
        "SELECT code, max_uses, uses, expires_at, created_by, instance_pin FROM invites ORDER BY created_at DESC"
    )
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn get_invite(db: &PgPool, code: &str) -> Result<Option<Invite>, AppError> {
    let row = sqlx::query_as!(
        InviteRow,
        "SELECT code, max_uses, uses, expires_at, created_by, instance_pin FROM invites WHERE code = $1",
        code,
    )
        .fetch_optional(db)
        .await?;
    Ok(row.map(Into::into))
}

pub async fn delete_invite(db: &PgPool, code: &str) -> Result<(), AppError> {
    sqlx::query!("DELETE FROM invites WHERE code = $1", code)
        .execute(db)
        .await?;
    Ok(())
}

/// Atomically redeem an invite; returns the redeemed row (its `instance_pin` + `created_by`) or None.
pub async fn redeem_invite(db: &PgPool, code: &str) -> Result<Option<Invite>, AppError> {
    let row = sqlx::query_as!(
        InviteRow,
        "UPDATE invites SET uses = uses + 1
         WHERE code = $1 AND (max_uses IS NULL OR max_uses = 0 OR uses < max_uses)
           AND (expires_at IS NULL OR expires_at > now())
         RETURNING code, max_uses, uses, expires_at, created_by, instance_pin",
        code,
    )
        .fetch_optional(db)
        .await?;
    Ok(row.map(Into::into))
}

pub async fn list_invites_by(db: &PgPool, created_by: &str) -> Result<Vec<Invite>, AppError> {
    let rows = sqlx::query_as!(
        InviteRow,
        "SELECT code, max_uses, uses, expires_at, created_by, instance_pin FROM invites WHERE created_by = $1 ORDER BY created_at DESC",
        created_by,
    )
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn cleanup_invites(db: &PgPool) -> Result<u64, AppError> {
    // `max_uses = 0` is an unlimited tracking invite — never cleaned up on exhaustion grounds.
    let res = sqlx::query!(
        "DELETE FROM invites
         WHERE (expires_at IS NOT NULL AND expires_at <= now())
            OR (max_uses IS NOT NULL AND max_uses > 0 AND uses >= max_uses)",
    )
        .execute(db)
        .await?;
    Ok(res.rows_affected())
}

// ── Operator credential ───────────────────────────────────────────────────────

pub struct AdminCred {
    pub token_hash: String,
    pub refresh_token_hash: Option<String>,
    pub refresh_expires_at: Option<DateTime<Utc>>,
}

pub async fn get_admin(db: &PgPool) -> Result<Option<AdminCred>, AppError> {
    Ok(sqlx::query_as!(
        AdminCred,
        "SELECT token_hash, refresh_token_hash, refresh_expires_at FROM resolver_admin WHERE id = 1",
    )
        .fetch_optional(db)
        .await?)
}

pub async fn upsert_admin_token(db: &PgPool, token_hash: &str) -> Result<(), AppError> {
    sqlx::query!(
        "INSERT INTO resolver_admin (id, token_hash, rotated_at) VALUES (1, $1, now())
         ON CONFLICT (id) DO UPDATE SET token_hash = $1, rotated_at = now()",
        token_hash,
    )
        .execute(db)
        .await?;
    Ok(())
}

pub async fn set_admin_refresh(
    db: &PgPool,
    refresh_token_hash: &str,
    refresh_expires_at: DateTime<Utc>,
) -> Result<(), AppError> {
    sqlx::query!(
        "UPDATE resolver_admin SET refresh_token_hash = $1, refresh_expires_at = $2 WHERE id = 1",
        refresh_token_hash,
        refresh_expires_at,
    )
        .execute(db)
        .await?;
    Ok(())
}
