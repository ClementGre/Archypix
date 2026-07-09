//! Backend settings — the **single source of truth** for every knob, core and runtime alike
//! (feature 23 §4).
//!
//! Read a field by its typed key: `settings.get(keys::JWT_SECRET)` (the value type is fixed by
//! the key). Composed/derived values (URLs, masked strings) are the free functions at the bottom.
//! Core fields (`.core()`/`.secret()`) are env-only; the rest are DB-editable from the dashboard and
//! read live from the snapshot each request/tick.

use crate::repository::app_settings::AppSettingsRepository;
use archypix_common::registration::RegistrationMode;
use archypix_common::settings::{SettingSpec, Settings};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// UI section labels for the settings dashboard.
pub mod group {
    pub const SERVER: &str = "Server";
    pub const DATABASE: &str = "Database";
    pub const REDIS: &str = "Redis";
    pub const IDENTITY: &str = "Identity & Topology";
    pub const RESOLVER: &str = "Resolver";
    pub const AUTH: &str = "Authentication";
    pub const FEDERATION: &str = "Federation";
    pub const WORKERS: &str = "Workers & Jobs";
    pub const PIPELINE: &str = "Pipeline & Routines";
    pub const STORAGE: &str = "Storage & Quotas";
    pub const S3: &str = "Object Storage (S3)";
    pub const WEBDAV: &str = "WebDAV";
    pub const OBSERVABILITY: &str = "Observability";
    pub const RATE_LIMITS: &str = "Rate Limits & Caps";
    pub const REGISTRATION: &str = "Registration";
}

/// Typed field handles. The `name` is the snapshot key; the env var is its UPPER_SNAKE.
pub mod keys {
    use super::RegistrationMode;
    use archypix_common::settings::SettingKey;

    // ── Server ──
    pub const LISTEN_ADDR: SettingKey<String> = SettingKey::new("listen_addr");
    pub const CORS_ORIGINS: SettingKey<Vec<String>> = SettingKey::new("cors_origins");

    // ── Database ──
    pub const DB_HOST: SettingKey<String> = SettingKey::new("db_host");
    pub const DB_PORT: SettingKey<u16> = SettingKey::new("db_port");
    pub const DB_USER: SettingKey<String> = SettingKey::new("db_user");
    pub const DB_PASSWORD: SettingKey<Option<String>> = SettingKey::new("db_password");
    pub const DB_NAME: SettingKey<String> = SettingKey::new("db_name");

    // ── Redis ──
    pub const REDIS_HOST: SettingKey<String> = SettingKey::new("redis_host");
    pub const REDIS_PORT: SettingKey<u16> = SettingKey::new("redis_port");
    pub const REDIS_USER: SettingKey<Option<String>> = SettingKey::new("redis_user");
    pub const REDIS_PASSWORD: SettingKey<Option<String>> = SettingKey::new("redis_password");
    pub const REDIS_DB: SettingKey<u16> = SettingKey::new("redis_db");

    // ── Identity & topology ──
    pub const BACK_DOMAIN: SettingKey<String> = SettingKey::new("back_domain");
    pub const BACK_USE_HTTPS: SettingKey<bool> = SettingKey::new("back_use_https");
    pub const GLOBAL_DOMAIN: SettingKey<String> = SettingKey::new("global_domain");
    pub const USE_RESOLVER: SettingKey<bool> = SettingKey::new("use_resolver");

    // ── Resolver ──
    pub const RESOLVER_INTERNAL_URL: SettingKey<String> = SettingKey::new("resolver_internal_url");
    pub const RESOLVER_JWT_SECRET: SettingKey<Option<String>> =
        SettingKey::new("resolver_jwt_secret");
    pub const BACK_INTERNAL_URL: SettingKey<String> = SettingKey::new("back_internal_url");
    pub const RESOLVER_HEARTBEAT_INTERVAL_SECS: SettingKey<u64> =
        SettingKey::new("resolver_heartbeat_interval_secs");
    pub const RESOLVER_DELEGATION_TTL_SECS: SettingKey<i64> =
        SettingKey::new("resolver_delegation_ttl_secs");

    // ── Auth ──
    pub const JWT_SECRET: SettingKey<String> = SettingKey::new("jwt_secret");
    pub const ACCESS_TOKEN_TTL_SECS: SettingKey<i64> = SettingKey::new("access_token_ttl_secs");
    pub const REFRESH_TOKEN_TTL_SECS: SettingKey<i64> = SettingKey::new("refresh_token_ttl_secs");

    // ── Federation ──
    pub const WEBFINGER_USE_HTTPS: SettingKey<bool> = SettingKey::new("webfinger_use_https");
    pub const FEDERATION_JWT_TTL_SECS: SettingKey<i64> = SettingKey::new("federation_jwt_ttl_secs");
    pub const FEDERATION_BACKEND_CACHE_TTL_SECS: SettingKey<u64> =
        SettingKey::new("federation_backend_cache_ttl_secs");
    pub const FEDERATION_REQUEST_TIMEOUT_MS: SettingKey<u64> =
        SettingKey::new("federation_request_timeout_ms");
    pub const TRACE_PROPAGATION_PEERS: SettingKey<Vec<String>> =
        SettingKey::new("trace_propagation_peers");

    // ── Workers & jobs ──
    pub const WORKER_JWT_SECRET: SettingKey<String> = SettingKey::new("worker_jwt_secret");
    pub const JOB_PROCESSING_TIMEOUT_SECS: SettingKey<i64> =
        SettingKey::new("job_processing_timeout_secs");
    pub const JOB_WATCHDOG_INTERVAL_SECS: SettingKey<u64> =
        SettingKey::new("job_watchdog_interval_secs");
    pub const JOB_RETENTION_SECS: SettingKey<i64> = SettingKey::new("job_retention_secs");
    pub const JOB_CLEANUP_INTERVAL_SECS: SettingKey<u64> =
        SettingKey::new("job_cleanup_interval_secs");

    // ── Pipeline & routines ──
    pub const PIPELINE_POLL_INTERVAL_SECS: SettingKey<u64> =
        SettingKey::new("pipeline_poll_interval_secs");
    pub const PIPELINE_BATCH_SLEEP_MS: SettingKey<u64> = SettingKey::new("pipeline_batch_sleep_ms");
    pub const PIPELINE_CONCURRENCY: SettingKey<usize> = SettingKey::new("pipeline_concurrency");
    pub const PIPELINE_RETRY_BACKOFF_SECS: SettingKey<i64> =
        SettingKey::new("pipeline_retry_backoff_secs");
    pub const PIPELINE_DEBOUNCE_MS: SettingKey<u64> = SettingKey::new("pipeline_debounce_ms");
    pub const EXIF_DRAIN_INTERVAL_SECS: SettingKey<u64> =
        SettingKey::new("exif_drain_interval_secs");
    pub const EXIF_DRAIN_BATCH: SettingKey<i64> = SettingKey::new("exif_drain_batch");
    pub const PURGE_SWEEP_INTERVAL_SECS: SettingKey<u64> =
        SettingKey::new("purge_sweep_interval_secs");
    pub const PURGE_SWEEP_BATCH: SettingKey<i64> = SettingKey::new("purge_sweep_batch");
    pub const STORAGE_RECONCILE_INTERVAL_SECS: SettingKey<u64> =
        SettingKey::new("storage_reconcile_interval_secs");
    pub const TASK_QUEUE_CONCURRENCY: SettingKey<usize> = SettingKey::new("task_queue_concurrency");

    // ── Storage & quotas ──
    pub const DEFAULT_STORAGE_QUOTA_BYTES: SettingKey<i64> =
        SettingKey::new("default_storage_quota_bytes");
    pub const STORAGE_RESERVATION_TTL_SECS: SettingKey<u64> =
        SettingKey::new("storage_reservation_ttl_secs");
    pub const STORAGE_WARN_RATIO: SettingKey<f64> = SettingKey::new("storage_warn_ratio");
    pub const STORAGE_CRITICAL_RATIO: SettingKey<f64> = SettingKey::new("storage_critical_ratio");

    // ── S3 ──
    pub const S3_ENDPOINT: SettingKey<String> = SettingKey::new("s3_endpoint");
    pub const S3_PUBLIC_ENDPOINT: SettingKey<String> = SettingKey::new("s3_public_endpoint");
    pub const S3_WORKERS_ENDPOINT: SettingKey<String> = SettingKey::new("s3_workers_endpoint");
    pub const S3_ACCESS_KEY: SettingKey<String> = SettingKey::new("s3_access_key");
    pub const S3_SECRET_KEY: SettingKey<String> = SettingKey::new("s3_secret_key");
    pub const S3_REGION: SettingKey<String> = SettingKey::new("s3_region");
    pub const S3_BUCKET_STAGING: SettingKey<String> = SettingKey::new("s3_bucket_staging");
    pub const S3_BUCKET_PICTURES: SettingKey<String> = SettingKey::new("s3_bucket_pictures");
    pub const S3_BUCKET_VERSIONS: SettingKey<String> = SettingKey::new("s3_bucket_versions");
    pub const S3_BUCKET_SMALL: SettingKey<String> = SettingKey::new("s3_bucket_small");
    pub const S3_BUCKET_MEDIUM: SettingKey<String> = SettingKey::new("s3_bucket_medium");
    pub const S3_BUCKET_LARGE: SettingKey<String> = SettingKey::new("s3_bucket_large");
    pub const S3_PRESIGN_TTL_SECS: SettingKey<u64> = SettingKey::new("s3_presign_ttl_secs");
    pub const S3_PRESIGN_CACHE_MARGIN_SECS: SettingKey<u64> =
        SettingKey::new("s3_presign_cache_margin_secs");

    // ── WebDAV ──
    pub const WEBDAV_MAX_UPLOAD_BYTES: SettingKey<u64> = SettingKey::new("webdav_max_upload_bytes");

    // ── Rate limits & caps ──
    pub const RATE_LIMIT_LOGIN_MAX: SettingKey<u64> = SettingKey::new("rate_limit_login_max");
    pub const RATE_LIMIT_LOGIN_WINDOW_SECS: SettingKey<u64> =
        SettingKey::new("rate_limit_login_window_secs");
    pub const RATE_LIMIT_REGISTER_MAX: SettingKey<u64> = SettingKey::new("rate_limit_register_max");
    pub const RATE_LIMIT_REGISTER_WINDOW_SECS: SettingKey<u64> =
        SettingKey::new("rate_limit_register_window_secs");
    pub const MAX_PENDING_OUTGOING_SHARES: SettingKey<usize> =
        SettingKey::new("max_pending_outgoing_shares");
    pub const MAX_PENDING_INCOMING_SHARES: SettingKey<usize> =
        SettingKey::new("max_pending_incoming_shares");

    // ── Registration ──
    pub const REGISTRATION_MODE: SettingKey<RegistrationMode> =
        SettingKey::new("registration_mode");
}

/// The full backend registry — the one place documenting every field.
pub fn registry() -> Vec<SettingSpec> {
    use keys::*;
    vec![
        // ── Server ──
        SettingSpec::new(LISTEN_ADDR, group::SERVER).core().default("0.0.0.0:80").restart_required()
            .doc("Address the HTTP server binds to.", "0.0.0.0:8000"),
        SettingSpec::new(CORS_ORIGINS, group::SERVER).default("")
            .doc("Allowed CORS origins ('*' = any, dev only). Hot-swapped by the dynamic CORS layer.", "https://app.example.com"),

        // ── Database ──
        SettingSpec::new(DB_HOST, group::DATABASE).core().doc("Postgres host.", "postgres"),
        SettingSpec::new(DB_PORT, group::DATABASE).core().default("5432").doc("Postgres port.", "5432"),
        SettingSpec::new(DB_USER, group::DATABASE).core().default("postgres").doc("Postgres user.", "archypix"),
        SettingSpec::new(DB_PASSWORD, group::DATABASE).secret().nullable().doc("Postgres password.", ""),
        SettingSpec::new(DB_NAME, group::DATABASE).core().default("archypix").doc("Postgres database name.", "archypix_back"),

        // ── Redis ──
        SettingSpec::new(REDIS_HOST, group::REDIS).core().doc("Redis host.", "redis"),
        SettingSpec::new(REDIS_PORT, group::REDIS).core().default("6379").doc("Redis port.", "6379"),
        SettingSpec::new(REDIS_USER, group::REDIS).core().nullable().doc("Redis user (optional).", ""),
        SettingSpec::new(REDIS_PASSWORD, group::REDIS).secret().nullable().doc("Redis password (optional).", ""),
        SettingSpec::new(REDIS_DB, group::REDIS).core().default("0")
            .doc("Redis logical DB index (0–15). Backends sharing one Redis should use different indices to isolate keys.", "0"),

        // ── Identity & topology ──
        SettingSpec::new(BACK_DOMAIN, group::IDENTITY).core().restart_required()
            .doc("This backend's public domain (host[:port]); JWT audience + WebFinger href.", "backend1.example.com"),
        SettingSpec::new(BACK_USE_HTTPS, group::IDENTITY).core().default("true").doc("Serve public URLs over HTTPS.", "true"),
        SettingSpec::new(GLOBAL_DOMAIN, group::IDENTITY).core().restart_required()
            .doc("Shared identity domain — the part after ':' in @user:global_domain. All backends sharing a user namespace must agree. May differ from BACK_DOMAIN (front WebFinger on it via a reverse proxy).", "example.com"),
        SettingSpec::new(USE_RESOLVER, group::IDENTITY).core().restart_required()
            .doc("Whether multiple backends share GLOBAL_DOMAIN behind a resolver. false = standalone (serves its own WebFinger + enforces registration locally); true = a resolver owns WebFinger and routes lookups + new-user registration across the pool.", "true"),

        // ── Resolver ──
        SettingSpec::new(RESOLVER_INTERNAL_URL, group::RESOLVER).core().default_computed(|s| {
            let scheme = if s.get(BACK_USE_HTTPS) { "https" } else { "http" };
            Value::String(format!("{}://{}", scheme, s.get(GLOBAL_DOMAIN)))
        }, "{scheme}://{GLOBAL_DOMAIN}")
            .doc("Internal URL of the resolver reachable from this backend. May differ from the public resolver URL on a shared private network (Docker/VPC). Defaults to {scheme}://{GLOBAL_DOMAIN}.", "http://resolver:8080"),
        SettingSpec::new(RESOLVER_JWT_SECRET, group::RESOLVER).secret().nullable()
            .doc("Shared secret authenticating this backend's PUSHES to the resolver (self-register / mapping update / heartbeat); must match the resolver's. Required when USE_RESOLVER. The resolver→backend direction uses backend-signed delegation tokens instead (feature 23 §3).", ""),
        SettingSpec::new(BACK_INTERNAL_URL, group::RESOLVER).core().default_computed(|s| {
            let scheme = if s.get(BACK_USE_HTTPS) { "https" } else { "http" };
            Value::String(format!("{}://{}", scheme, s.get(BACK_DOMAIN)))
        }, "{scheme}://{BACK_DOMAIN}")
            .doc("Internal URL the resolver uses to reach THIS backend for API calls (e.g. user creation). Defaults to the public URL; override on a shared private network to use the container hostname.", "http://backend1:8000"),
        SettingSpec::new(RESOLVER_HEARTBEAT_INTERVAL_SECS, group::RESOLVER).default("300").routine("resolver_heartbeat")
            .doc("How often this backend pushes a heartbeat (delegation token + metrics) to the resolver.", "300"),
        SettingSpec::new(RESOLVER_DELEGATION_TTL_SECS, group::RESOLVER).default("360")
            .doc("TTL of a minted ResolverDelegation token (must exceed the heartbeat interval).", "360"),

        // ── Auth ──
        SettingSpec::new(JWT_SECRET, group::AUTH).secret().doc("HS256 secret for user/session JWTs.", ""),
        SettingSpec::new(ACCESS_TOKEN_TTL_SECS, group::AUTH).default("900").doc("Access-token lifetime (seconds).", "900"),
        SettingSpec::new(REFRESH_TOKEN_TTL_SECS, group::AUTH).default("15552000").doc("Refresh-token lifetime (seconds).", "15552000"),

        // ── Federation ──
        SettingSpec::new(WEBFINGER_USE_HTTPS, group::FEDERATION).core().default("true")
            .doc("Use HTTPS for the initial /archypix-resolver/resolve query when resolving remote backends (subsequent calls use the scheme in the returned backend_url). Set false in local/Docker (HTTP) environments.", "true"),
        SettingSpec::new(FEDERATION_JWT_TTL_SECS, group::FEDERATION).default("86400").doc("TTL of pairwise federation JWTs.", "86400"),
        SettingSpec::new(FEDERATION_BACKEND_CACHE_TTL_SECS, group::FEDERATION).default("3600").doc("TTL of the WebFinger backend-URL cache.", "3600"),
        SettingSpec::new(FEDERATION_REQUEST_TIMEOUT_MS, group::FEDERATION).default("1000").doc("Per-request timeout (ms) for outbound federation calls.", "1000"),
        SettingSpec::new(TRACE_PROPAGATION_PEERS, group::OBSERVABILITY).default("")
            .doc("Global domains of federation peers sharing this operator's Jaeger; trace context flows only to these.", "peer.example.com"),

        // ── Workers & jobs ──
        SettingSpec::new(WORKER_JWT_SECRET, group::WORKERS).secret().doc("Shared secret for worker JWTs.", ""),
        SettingSpec::new(JOB_PROCESSING_TIMEOUT_SECS, group::WORKERS).default("600").routine("job_watchdog")
            .doc("How long a job may stay 'processing' before the watchdog resets it.", "600"),
        SettingSpec::new(JOB_WATCHDOG_INTERVAL_SECS, group::WORKERS).default("60").routine("job_watchdog")
            .doc("How often the stale-job watchdog scan runs.", "60"),
        SettingSpec::new(JOB_RETENTION_SECS, group::WORKERS).default("2592000").routine("job_cleanup")
            .doc("Age after a terminal job's completion at which cleanup deletes it.", "2592000"),
        SettingSpec::new(JOB_CLEANUP_INTERVAL_SECS, group::WORKERS).default("86400").routine("job_cleanup")
            .doc("How often terminal-job cleanup runs.", "86400"),

        // ── Pipeline & routines ──
        SettingSpec::new(PIPELINE_POLL_INTERVAL_SECS, group::PIPELINE).default("3600").routine("pipeline")
            .doc("How often the pipeline runs a recovery sweep (event-driven wakes are immediate).", "3600"),
        SettingSpec::new(PIPELINE_DEBOUNCE_MS, group::PIPELINE).default("5000").routine("pipeline")
            .doc("Debounce window (ms) coalescing a burst of pipeline wakes. 0 disables.", "5000"),
        SettingSpec::new(PIPELINE_CONCURRENCY, group::PIPELINE).default("4").routine("pipeline").restart_required()
            .doc("Max users whose pipeline runs concurrently (serial per user).", "4"),
        SettingSpec::new(PIPELINE_RETRY_BACKOFF_SECS, group::PIPELINE).default("60").routine("pipeline")
            .doc("Backoff (seconds) before retrying a failed share announce/unannounce.", "60"),
        SettingSpec::new(PIPELINE_BATCH_SLEEP_MS, group::PIPELINE).default("0").routine("pipeline")
            .doc("Optional sleep (ms) between picture batches for backpressure.", "0"),
        SettingSpec::new(EXIF_DRAIN_INTERVAL_SECS, group::PIPELINE).default("5").routine("exif_drain")
            .doc("Fallback sweep interval for the deferred-EXIF-job drain.", "5"),
        SettingSpec::new(EXIF_DRAIN_BATCH, group::PIPELINE).default("200").routine("exif_drain")
            .doc("Max pictures the EXIF drain turns into reconcile jobs per pass.", "200"),
        SettingSpec::new(PURGE_SWEEP_INTERVAL_SECS, group::PIPELINE).default("3600").routine("purge_sweep")
            .doc("How often the trash purge sweep runs.", "3600"),
        SettingSpec::new(PURGE_SWEEP_BATCH, group::PIPELINE).default("200").routine("purge_sweep")
            .doc("Max pictures physically purged per sweep tick.", "200"),
        SettingSpec::new(STORAGE_RECONCILE_INTERVAL_SECS, group::PIPELINE).default("86400").routine("storage_reconcile")
            .doc("Period of the storage-usage reconcile (drift safety net).", "86400"),
        SettingSpec::new(TASK_QUEUE_CONCURRENCY, group::PIPELINE).default("4").restart_required()
            .doc("Max concurrency for general-purpose routines (tag-rename, unannounce pictures, etc.).", "4"),

        // ── Storage & quotas ──
        SettingSpec::new(DEFAULT_STORAGE_QUOTA_BYTES, group::STORAGE).default("0").doc("Initial quota (bytes) for a new user. 0 = unlimited.", "10737418240"),
        SettingSpec::new(STORAGE_RESERVATION_TTL_SECS, group::STORAGE).default_computed(|s| {
            Value::from(s.get(S3_PRESIGN_TTL_SECS) + 60)
        }, "{S3_PRESIGN_TTL_SECS} + 60").doc("TTL (seconds) of an in-flight upload reservation.", "3660"),
        SettingSpec::new(STORAGE_WARN_RATIO, group::STORAGE).default("0.8").doc("Usage ratio at which GET /me/storage reports 'warn'.", "0.8"),
        SettingSpec::new(STORAGE_CRITICAL_RATIO, group::STORAGE).default("0.9").doc("Usage ratio at which GET /me/storage reports 'critical'.", "0.9"),

        // ── S3 ──
        SettingSpec::new(S3_ENDPOINT, group::S3).core().doc("Server-side S3 endpoint.", "http://minio:9000"),
        SettingSpec::new(S3_PUBLIC_ENDPOINT, group::S3).core().default_computed(|s| Value::String(s.get(S3_ENDPOINT)), "{S3_ENDPOINT}")
            .doc("Public-facing S3 endpoint embedded in presigned URLs handed to browsers. Defaults to S3_ENDPOINT; override when the internal address (e.g. http://minio:9000) differs from what browsers reach (e.g. http://localhost:9000).", "http://localhost:9000"),
        SettingSpec::new(S3_WORKERS_ENDPOINT, group::S3).core().default_computed(|s| Value::String(s.get(S3_ENDPOINT)), "{S3_ENDPOINT}")
            .doc("S3 endpoint embedded in presigned URLs handed to worker processes. Defaults to S3_ENDPOINT; override when workers reach the store via a different address than browsers.", "http://minio:9000"),
        SettingSpec::new(S3_ACCESS_KEY, group::S3).secret().doc("S3 access key.", ""),
        SettingSpec::new(S3_SECRET_KEY, group::S3).secret().doc("S3 secret key.", ""),
        SettingSpec::new(S3_REGION, group::S3).core().default("us-east-1").doc("S3 region.", "us-east-1"),
        SettingSpec::new(S3_BUCKET_STAGING, group::S3).core().default("archypix-staging").doc("Temporary-upload bucket — MUST be dedicated (a 1-day auto-expiration rule is applied at startup). All other buckets may share one bucket (keys are namespaced).", "archypix-staging"),
        SettingSpec::new(S3_BUCKET_PICTURES, group::S3).core().default("archypix-pictures").doc("Current-picture bucket.", "archypix-pictures"),
        SettingSpec::new(S3_BUCKET_VERSIONS, group::S3).core().default("archypix-versions").doc("Version-snapshot bucket.", "archypix-versions"),
        SettingSpec::new(S3_BUCKET_SMALL, group::S3).core().default("archypix-small").doc("Small-thumbnail bucket.", "archypix-small"),
        SettingSpec::new(S3_BUCKET_MEDIUM, group::S3).core().default("archypix-medium").doc("Medium-thumbnail bucket.", "archypix-medium"),
        SettingSpec::new(S3_BUCKET_LARGE, group::S3).core().default("archypix-large").doc("Large-thumbnail bucket.", "archypix-large"),
        SettingSpec::new(S3_PRESIGN_TTL_SECS, group::S3).default("3600").doc("Presigned-URL lifetime (seconds).", "3600"),
        SettingSpec::new(S3_PRESIGN_CACHE_MARGIN_SECS, group::S3).default("600").doc("Safety margin (seconds) before a cached presign is considered stale.", "600"),

        // ── WebDAV ──
        SettingSpec::new(WEBDAV_MAX_UPLOAD_BYTES, group::WEBDAV).default("5368709120").doc("Upper bound on a single WebDAV PUT body (bytes).", "5368709120"),

        // ── Rate limits & caps ──
        SettingSpec::new(RATE_LIMIT_LOGIN_MAX, group::RATE_LIMITS).default("10").doc("Max failed/attempted logins per username per window.", "10"),
        SettingSpec::new(RATE_LIMIT_LOGIN_WINDOW_SECS, group::RATE_LIMITS).default("300").doc("Login rate-limit window (seconds).", "300"),
        SettingSpec::new(RATE_LIMIT_REGISTER_MAX, group::RATE_LIMITS).default("5").doc("Max registrations per client IP per window.", "5"),
        SettingSpec::new(RATE_LIMIT_REGISTER_WINDOW_SECS, group::RATE_LIMITS).default("3600").doc("Registration rate-limit window (seconds).", "3600"),
        SettingSpec::new(MAX_PENDING_OUTGOING_SHARES, group::RATE_LIMITS).default("100").doc("Max pending outgoing shares per user.", "100"),
        SettingSpec::new(MAX_PENDING_INCOMING_SHARES, group::RATE_LIMITS).default("200").doc("Max pending incoming shares per recipient.", "200"),

        // ── Registration ──
        SettingSpec::new(REGISTRATION_MODE, group::REGISTRATION).default("open")
            .doc("Who may register — **standalone mode only** (behind a resolver the resolver's own registration_mode is authoritative): open (anyone), invite (any user mints invites), admin_invite (only admins mint).", "open"),
    ]
}

// ── Composed / derived values (multi-field; free functions over the settings) ────

pub fn back_scheme(s: &Settings) -> &'static str {
    if s.get(keys::BACK_USE_HTTPS) {
        "https"
    } else {
        "http"
    }
}
pub fn webfinger_scheme(s: &Settings) -> &'static str {
    if s.get(keys::WEBFINGER_USE_HTTPS) {
        "https"
    } else {
        "http"
    }
}
pub fn public_base_url(s: &Settings) -> String {
    format!("{}://{}", back_scheme(s), s.get(keys::BACK_DOMAIN))
}
pub fn database_url(s: &Settings) -> String {
    build_pg_url(
        &s.get(keys::DB_HOST),
        s.get(keys::DB_PORT),
        &s.get(keys::DB_USER),
        s.get(keys::DB_PASSWORD).as_deref(),
        &s.get(keys::DB_NAME),
    )
}
pub fn database_url_masked(s: &Settings) -> String {
    let pw = if s.get(keys::DB_PASSWORD).is_some() {
        Some("***")
    } else {
        None
    };
    build_pg_url(
        &s.get(keys::DB_HOST),
        s.get(keys::DB_PORT),
        &s.get(keys::DB_USER),
        pw,
        &s.get(keys::DB_NAME),
    )
}
pub fn redis_url(s: &Settings) -> String {
    build_redis_url(
        &s.get(keys::REDIS_HOST),
        s.get(keys::REDIS_PORT),
        s.get(keys::REDIS_USER).as_deref(),
        s.get(keys::REDIS_PASSWORD).as_deref(),
        s.get(keys::REDIS_DB) as u8,
    )
}
pub fn redis_url_masked(s: &Settings) -> String {
    let pw = if s.get(keys::REDIS_PASSWORD).is_some() {
        Some("***")
    } else {
        None
    };
    build_redis_url(
        &s.get(keys::REDIS_HOST),
        s.get(keys::REDIS_PORT),
        s.get(keys::REDIS_USER).as_deref(),
        pw,
        s.get(keys::REDIS_DB) as u8,
    )
}

fn build_pg_url(host: &str, port: u16, user: &str, password: Option<&str>, db: &str) -> String {
    match password {
        Some(pw) => format!("postgres://{user}:{pw}@{host}:{port}/{db}"),
        None => format!("postgres://{user}@{host}:{port}/{db}"),
    }
}
fn build_redis_url(
    host: &str,
    port: u16,
    user: Option<&str>,
    password: Option<&str>,
    db: u8,
) -> String {
    match (user, password) {
        (Some(u), Some(pw)) => format!("redis://{u}:{pw}@{host}:{port}/{db}"),
        (None, Some(pw)) => format!("redis://:{pw}@{host}:{port}/{db}"),
        (Some(u), None) => format!("redis://{u}@{host}:{port}/{db}"),
        (None, None) => format!("redis://{host}:{port}/{db}"),
    }
}

// ── Loading ──────────────────────────────────────────────────────────────────────

/// Load from `default → env` only (no DB yet) — the first boot step, before the DB is reachable
/// (connection settings themselves live here). Reload with DB overrides once connected.
pub fn load_env_only() -> anyhow::Result<Arc<Settings>> {
    dotenvy::dotenv().ok();
    let settings = Settings::load(&registry(), &HashMap::new())?;
    validate(&settings)?;
    Ok(Arc::new(settings))
}

/// Merge in the DB runtime overrides and publish (after the DB is connected).
pub async fn reload_from_db(settings: &Settings, db: &sqlx::PgPool) -> anyhow::Result<()> {
    let overrides = AppSettingsRepository::load_all(db).await?;
    settings.reload(&overrides)?;
    validate(settings)?;
    Ok(())
}

/// Cross-field invariants not expressible per-field.
fn validate(s: &Settings) -> anyhow::Result<()> {
    use keys::*;
    let staging = s.get(S3_BUCKET_STAGING);
    for b in [
        s.get(S3_BUCKET_PICTURES),
        s.get(S3_BUCKET_VERSIONS),
        s.get(S3_BUCKET_SMALL),
        s.get(S3_BUCKET_MEDIUM),
        s.get(S3_BUCKET_LARGE),
    ] {
        if b == staging {
            anyhow::bail!(
                "S3_BUCKET_STAGING must differ from all other bucket names (it has an expiration rule)."
            );
        }
    }
    if s.get(USE_RESOLVER) && s.get(RESOLVER_JWT_SECRET).is_none() {
        anyhow::bail!("RESOLVER_JWT_SECRET must be set when USE_RESOLVER=true.");
    }
    Ok(())
}

// ── Test helpers ─────────────────────────────────────────────────────────────────

/// Build settings for tests (no process-env dependency). `overrides` are `(ENV_NAME, value)` pairs,
/// so even core fields can be customised.
pub fn test_settings_with(overrides: &[(&str, &str)]) -> Arc<Settings> {
    let mut env: HashMap<String, String> = [
        ("DB_HOST", "localhost"),
        ("DB_NAME", "test"),
        ("REDIS_HOST", "localhost"),
        ("BACK_DOMAIN", "backend.test.com"),
        ("BACK_USE_HTTPS", "false"),
        ("GLOBAL_DOMAIN", "test.com"),
        ("USE_RESOLVER", "false"),
        ("WEBFINGER_USE_HTTPS", "false"),
        (
            "JWT_SECRET",
            "test_jwt_secret_must_be_long_enough_for_hmac_sha256",
        ),
        (
            "WORKER_JWT_SECRET",
            "test_worker_secret_must_be_long_enough_also",
        ),
        ("S3_ENDPOINT", "http://localhost:9000"),
        ("S3_ACCESS_KEY", "minioadmin"),
        ("S3_SECRET_KEY", "minioadmin"),
        ("PIPELINE_DEBOUNCE_MS", "0"),
        ("TASK_QUEUE_CONCURRENCY", "1"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    for (k, v) in overrides {
        env.insert(k.to_string(), v.to_string());
    }
    let lookup = move |k: &str| env.get(k).cloned();
    Arc::new(
        Settings::load_with_env(&registry(), &HashMap::new(), &lookup)
            .expect("test settings must load"),
    )
}
