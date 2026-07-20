//! Fixed-window rate limiting backed by Redis.
//!
//! A single primitive — [`check`] — increments a per-bucket counter and rejects with
//! `429 Too Many Requests` once it exceeds `max` within `window_secs`. The window is a fixed
//! window (the counter's TTL is set only on the first hit), so the limit naturally resets after
//! the window elapses rather than sliding forever under sustained load.

use crate::infra::redis::{Cache, RedisKey};
use archypix_common::error::AppError;
use chrono::Utc;

/// Rate-limit categories tracked for the admin observability surface (feature 28 §9.2).
pub mod category {
    pub const LOGIN: &str = "login";
    pub const REGISTER: &str = "register";
    pub const PUBLIC_UPLOAD: &str = "public_upload";
    pub const FEDERATION: &str = "federation";
    pub const PRESIGN: &str = "presign";

    /// Every category, for the admin timeline.
    pub const ALL: [&str; 5] = [LOGIN, REGISTER, PUBLIC_UPLOAD, FEDERATION, PRESIGN];
}

/// Like [`check`], but on rejection also records a per-minute rejection event for `category` so the
/// admin "Rate limiting" tab can surface recent activity (feature 28 §9.2). `retention_secs` is the
/// event bucket TTL. Recording is best-effort (never blocks the request).
pub async fn check_categorized(
    cache: &dyn Cache,
    category: &str,
    bucket: &str,
    max: u64,
    window_secs: u64,
    retention_secs: u64,
) -> Result<(), AppError> {
    match check(cache, bucket, max, window_secs).await {
        Ok(()) => Ok(()),
        Err(e @ AppError::TooManyRequests(_)) => {
            record_rejection(cache, category, retention_secs).await;
            Err(e)
        }
        Err(e) => Err(e),
    }
}

/// Record one rate-limit rejection in the current-minute bucket for `category` (aggregated so a
/// flood stays bounded). Best-effort.
pub async fn record_rejection(cache: &dyn Cache, category: &str, retention_secs: u64) {
    let minute = Utc::now().timestamp() / 60;
    let _ = cache
        .incr_ex(RedisKey::RateLimitEvent(category, minute), retention_secs)
        .await;
}

/// Increment the counter for `bucket` and fail with `TooManyRequests` if it now exceeds `max`.
///
/// `bucket` is an opaque `category:id` string (e.g. `login:alice`, `register:1.2.3.4`). A cache
/// error never blocks the request — the limiter fails open so a Redis outage cannot lock everyone
/// out (availability over a best-effort throttle).
async fn check(
    cache: &dyn Cache,
    bucket: &str,
    max: u64,
    window_secs: u64,
) -> Result<(), AppError> {
    let count = match cache
        .incr_ex(RedisKey::RateLimit(bucket), window_secs)
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(bucket, error = ?e, "rate limiter unavailable — failing open");
            return Ok(());
        }
    };
    if count > max {
        tracing::warn!(bucket, count, max, "rate limit exceeded");
        return Err(AppError::TooManyRequests(
            "Too many requests. Please slow down and try again later".to_string(),
        ));
    }
    Ok(())
}
