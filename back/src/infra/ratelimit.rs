//! Fixed-window rate limiting backed by Redis.
//!
//! A single primitive — [`check`] — increments a per-bucket counter and rejects with
//! `429 Too Many Requests` once it exceeds `max` within `window_secs`. The window is a fixed
//! window (the counter's TTL is set only on the first hit), so the limit naturally resets after
//! the window elapses rather than sliding forever under sustained load.

use crate::infra::error::AppError;
use crate::infra::redis::{Cache, RedisKey};

/// Increment the counter for `bucket` and fail with `TooManyRequests` if it now exceeds `max`.
///
/// `bucket` is an opaque `category:id` string (e.g. `login:alice`, `register:1.2.3.4`). A cache
/// error never blocks the request — the limiter fails open so a Redis outage cannot lock everyone
/// out (availability over a best-effort throttle).
pub async fn check(
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
