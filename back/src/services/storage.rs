//! Storage-quota enforcement & reporting (feature 22).
//!
//! Authoritative usage lives in Postgres (`user_storage`, trigger-maintained). Redis holds the fast
//! path: a cached mirror of the committed billed total plus the in-flight reservation counter.
//! Enforcement math on any byte-adding write is `committed + reserved + incoming ≤ quota`.

use crate::infra::config::Config;
use crate::infra::error::AppError;
use crate::infra::redis::{Cache, RedisKey, storage_reservation_prefix};
use crate::repository::user_storage::{UserStorage, UserStorageRepository};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// TTL of the cached `committed` mirror. Self-heals a mirror left stale by a background purge; write
/// points invalidate it explicitly, and the reconcile routine refreshes it daily.
const COMMITTED_TTL_SECS: u64 = 3600;

/// Cached billed total (`committed`), recomputing from Postgres on a cache miss.
async fn committed(cache: &dyn Cache, db: &PgPool, user_id: Uuid) -> Result<i64, AppError> {
    if let Some(v) = cache
        .get_str(RedisKey::StorageCommitted(user_id))
        .await
        .ok()
        .flatten()
        .and_then(|s| s.parse::<i64>().ok())
    {
        return Ok(v);
    }
    let total = UserStorageRepository::get(db, user_id)
        .await?
        .billed_total();
    let _ = cache
        .set_str_ex(
            RedisKey::StorageCommitted(user_id),
            &total.to_string(),
            COMMITTED_TTL_SECS,
        )
        .await;
    Ok(total)
}

/// Sum of the user's in-flight presigned-upload reservations.
async fn reserved(cache: &dyn Cache, user_id: Uuid) -> i64 {
    cache
        .sum_int_by_prefix(&storage_reservation_prefix(user_id))
        .await
        .unwrap_or(0)
}

/// Effective usage for a quota check: committed (cached mirror) + reserved (in-flight uploads).
pub async fn effective_usage(
    cache: &dyn Cache,
    db: &PgPool,
    user_id: Uuid,
) -> Result<i64, AppError> {
    Ok(committed(cache, db, user_id).await? + reserved(cache, user_id).await)
}

/// Whether `incoming` more bytes fit under the user's quota (unlimited ⇒ always fits). `incoming`
/// may be a net delta (WebDAV overwrite) or an absolute add.
pub async fn fits(
    cache: &dyn Cache,
    db: &PgPool,
    user_id: Uuid,
    incoming: i64,
) -> Result<bool, AppError> {
    match UserStorageRepository::get_quota(db, user_id).await? {
        None => Ok(true), // unlimited
        Some(quota) if quota <= 0 => Ok(true),
        Some(quota) => {
            let used = effective_usage(cache, db, user_id).await?;
            Ok(used.saturating_add(incoming.max(0)) <= quota)
        }
    }
}

/// Whether the user is already at or over quota, ignoring `incoming` (the coarse gate used when a
/// presign slot declares no size — the `complete_upload` hard check is the backstop).
pub async fn at_or_over_quota(
    cache: &dyn Cache,
    db: &PgPool,
    user_id: Uuid,
) -> Result<bool, AppError> {
    match UserStorageRepository::get_quota(db, user_id).await? {
        None => Ok(false),
        Some(quota) if quota <= 0 => Ok(false),
        Some(quota) => Ok(effective_usage(cache, db, user_id).await? >= quota),
    }
}

/// Add a reservation sub-key for an in-flight presigned upload (§5.2). Auto-releases on TTL.
pub async fn reserve(
    cache: &dyn Cache,
    config: &Config,
    user_id: Uuid,
    picture_id: Uuid,
    bytes: i64,
) -> Result<(), AppError> {
    cache
        .set_str_ex(
            RedisKey::StorageReservation(user_id, picture_id),
            &bytes.max(0).to_string(),
            config.storage_reservation_ttl_secs,
        )
        .await
}

/// Release an upload reservation (on `complete_upload`, or on abort).
pub async fn release(cache: &dyn Cache, user_id: Uuid, picture_id: Uuid) {
    let _ = cache
        .del(RedisKey::StorageReservation(user_id, picture_id))
        .await;
}

/// Invalidate the cached committed mirror after a byte-adding commit; the next check recomputes.
pub async fn invalidate_committed(cache: &dyn Cache, user_id: Uuid) {
    let _ = cache.del(RedisKey::StorageCommitted(user_id)).await;
}

// ── Reporting (GET /me/storage, admin payloads) ─────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WarnLevel {
    Ok,
    Warn,
    Critical,
    Full,
}

/// Classify a usage ratio against the configured thresholds. Unlimited (`ratio = None`) is `Ok`.
pub fn warn_level(ratio: Option<f64>, config: &Config) -> WarnLevel {
    match ratio {
        None => WarnLevel::Ok,
        Some(r) if r >= 1.0 => WarnLevel::Full,
        Some(r) if r >= config.storage_critical_ratio => WarnLevel::Critical,
        Some(r) if r >= config.storage_warn_ratio => WarnLevel::Warn,
        Some(_) => WarnLevel::Ok,
    }
}

/// The `GET /api/me/storage` payload (feature 22 §8.1).
#[derive(Debug, Serialize)]
pub struct StorageInfo {
    pub quota_bytes: Option<i64>,
    pub used_bytes: i64,
    pub available_bytes: Option<i64>,
    pub breakdown: UserStorage,
    pub reclaimable_trash_bytes: i64,
    pub usage_ratio: Option<f64>,
    pub warn_level: WarnLevel,
}

/// Build the storage report for a user. `used` is the authoritative billed total (read straight
/// from Postgres, not the cached mirror).
#[tracing::instrument(skip(db, config), fields(user_id = %user_id))]
pub async fn storage_info(
    db: &PgPool,
    config: &Config,
    user_id: Uuid,
) -> Result<StorageInfo, AppError> {
    let breakdown = UserStorageRepository::get(db, user_id).await?;
    let quota_bytes = UserStorageRepository::get_quota(db, user_id).await?;
    let used_bytes = breakdown.billed_total();
    let reclaimable_trash_bytes = breakdown.reclaimable_trash_bytes();

    // A `0` quota means unlimited (matches `NULL`); only a positive quota constrains.
    let effective_quota = quota_bytes.filter(|q| *q > 0);
    let available_bytes = effective_quota.map(|q| (q - used_bytes).max(0));
    let usage_ratio = effective_quota.map(|q| used_bytes as f64 / q as f64);

    Ok(StorageInfo {
        quota_bytes: effective_quota,
        used_bytes,
        available_bytes,
        breakdown,
        reclaimable_trash_bytes,
        usage_ratio,
        warn_level: warn_level(usage_ratio, config),
    })
}
