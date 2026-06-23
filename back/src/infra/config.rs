#[derive(Debug, Clone)]
pub struct Config {
    // ── Server ────────────────────────────────────────────────────────────────
    pub listen_addr: String,

    // ── Database (split) ──────────────────────────────────────────────────────
    pub db_host: String,
    pub db_port: u16,
    pub db_user: String,
    pub db_password: Option<String>,
    pub db_name: String,

    // ── Redis (split) ─────────────────────────────────────────────────────────
    pub redis_host: String,
    pub redis_port: u16,
    pub redis_user: Option<String>,
    pub redis_password: Option<String>,
    pub redis_db: u8,

    // ── CORS ──────────────────────────────────────────────────────────────────
    /// Comma-separated list of allowed origins. Use `*` to allow any origin (dev only).
    pub cors_origins: Vec<String>,

    // ── Identity / Domains ────────────────────────────────────────────────────
    /// This backend's public-facing domain (host:port). Used as JWT audience and WebFinger href.
    pub back_domain: String,
    /// Whether this backend is served over HTTPS. Determines the scheme in public URLs.
    pub back_use_https: bool,
    /// Global domain that appears in user identities (@user:global_domain).
    pub global_domain: String,

    // ── Resolver ──────────────────────────────────────────────────────────────
    pub use_resolver: bool,
    /// Internal URL of the resolver (e.g. http://resolver:8080). Only used when use_resolver=true.
    pub resolver_internal_url: String,
    /// Shared JWT secret between this backend and the resolver.
    pub resolver_jwt_secret: String,
    /// Internal URL that the resolver uses to reach this backend for API calls
    /// (e.g. http://backend1:8000 in Docker). Defaults to `public_base_url()` if not set.
    pub back_internal_url: Option<String>,

    // ── JWT / Auth ────────────────────────────────────────────────────────────
    pub jwt_secret: String,
    pub access_token_ttl_secs: i64,
    pub refresh_token_ttl_secs: i64,

    // ── Federation ────────────────────────────────────────────────────────────
    /// Whether to use HTTPS when resolving remote backends via WebFinger.
    /// Controls only the `.well-known/webfinger` call; all subsequent API calls
    /// use the scheme embedded in the `backend_url` returned by the resolver.
    pub webfinger_use_https: bool,
    pub federation_jwt_ttl_secs: i64,
    pub federation_backend_cache_ttl_secs: u64,
    pub federation_request_timeout_ms: u64,

    // ── Workers ───────────────────────────────────────────────────────────────
    /// Shared JWT secret between this backend and all worker instances.
    pub worker_jwt_secret: String,
    /// Maximum number of in-process background tasks running concurrently
    /// (tag-rename, etc.). Does not affect external workers.
    pub task_queue_concurrency: usize,
    /// How long (seconds) a job may stay in `processing` state before the
    /// watchdog considers the worker dead and resets it to `pending`.
    pub job_processing_timeout_secs: i64,
    /// How often (seconds) the watchdog runs its stale-job scan.
    pub job_watchdog_interval_secs: u64,
    /// Age (seconds) after a terminal job's `completed_at` at which it is deleted by the cleanup
    /// task. Default: 2592000 (30 days).
    pub job_retention_secs: i64,
    /// How often (seconds) the terminal-job cleanup task runs. Default: 86400 (24 h).
    pub job_cleanup_interval_secs: u64,
    /// How often (seconds) the trash purge sweep runs (physically deletes owned pictures past their
    /// retention window). Default: 3600 (1 h).
    pub purge_sweep_interval_secs: u64,
    /// Maximum pictures physically purged per sweep tick. Default: 200.
    pub purge_sweep_batch: i64,
    /// How often (seconds) the tagging pipeline loop runs a recovery sweep,
    /// catching pictures missed due to restarts. Event-driven wakes happen immediately.
    /// Default: 3600 (1 hour).
    pub pipeline_poll_interval_secs: u64,
    /// Optional milliseconds to sleep between picture batches in the pipeline (default 0).
    /// Provides manual backpressure for large deployments.
    pub pipeline_batch_sleep_ms: u64,
    /// Maximum number of users whose pipeline runs concurrently. Runs are serialized per user
    /// and parallel across users. Default: 4.
    pub pipeline_concurrency: usize,
    /// Backoff (seconds) before a share whose announce/unannounce delivery failed is retried.
    /// Default: 60.
    pub pipeline_retry_backoff_secs: i64,
    /// Debounce window (milliseconds) before an idle user's pipeline run starts. Wakes that arrive
    /// during the window are coalesced into a single run (the window is *not* reset, so latency is
    /// bounded to this value regardless of wake rate). This collapses a burst of per-picture
    /// worker-completion wakes into one pass. `0` disables debouncing (run immediately) — the value
    /// used by tests for determinism. Default: 5000.
    pub pipeline_debounce_ms: u64,
    /// How often (seconds) the deferred-EXIF-job drain runs its fallback sweep (feature 14 §5). A
    /// batch EXIF edit also wakes the drain immediately; the interval is the crash/lost-wake
    /// recovery backstop. Default: 5.
    pub exif_drain_interval_secs: u64,
    /// Maximum pictures the deferred-EXIF-job drain turns into reconcile jobs per pass. Default: 200.
    pub exif_drain_batch: i64,

    // ── S3 / Object storage ───────────────────────────────────────────────────
    pub s3_endpoint: String,
    /// Public-facing S3 endpoint used in presigned URLs returned to browsers.
    /// Defaults to `s3_endpoint` when not set — override when the internal and
    /// external addresses differ (e.g. `http://minio:9000` vs `http://localhost:9000`).
    pub s3_public_endpoint: String,
    /// S3 endpoint used in presigned URLs returned to worker processes.
    /// Defaults to `s3_endpoint`.  Override when workers reach MinIO via a
    /// different address than the public internet (e.g. a private Docker network).
    pub s3_workers_endpoint: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub s3_region: String,
    pub s3_bucket_staging: String,
    pub s3_bucket_pictures: String,
    pub s3_bucket_versions: String,
    pub s3_bucket_small: String,
    pub s3_bucket_medium: String,
    pub s3_bucket_large: String,
    pub s3_presign_ttl_secs: u64,
    pub s3_presign_cache_margin_secs: u64,

    // ── WebDAV ────────────────────────────────────────────────────────────────
    /// Upper bound on a single WebDAV `PUT` body (bytes). The body is streamed to a temp file
    /// (never buffered in memory) and rejected with `413` past this limit. Default: 5 GiB.
    pub webdav_max_upload_bytes: u64,

    // ── Observability ─────────────────────────────────────────────────────────
    /// Global domains of federation peers that share this operator's Jaeger instance.
    /// Trace context is propagated to/from these peers only; all others start a fresh root.
    pub trace_propagation_peers: Vec<String>,

    // ── Rate limiting / abuse caps ————————————————————————————————–───────────
    /// Max failed-or-attempted logins per username per window before `429`. Default 10.
    pub rate_limit_login_max: u64,
    pub rate_limit_login_window_secs: u64,
    /// Max registrations per client IP per window before `429`. Default 5.
    pub rate_limit_register_max: u64,
    pub rate_limit_register_window_secs: u64,
    /// Max `pending` outgoing shares a single user may hold at once. Default 100.
    pub max_pending_outgoing_shares: usize,
    /// Max `pending` incoming shares a single recipient may hold at once. Default 200.
    pub max_pending_incoming_shares: usize,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();

        let use_resolver = require_bool_env("USE_RESOLVER")?;

        // Extract early so they can be used to derive defaults for other fields.
        let back_use_https = env_bool("BACK_USE_HTTPS", true)?;
        let global_domain = require_env("GLOBAL_DOMAIN")?;
        let back_scheme = if back_use_https { "https" } else { "http" };
        let s3_endpoint = require_env("S3_ENDPOINT")?;

        let config = Config {
            listen_addr: env("LISTEN_ADDR", "0.0.0.0:80".to_string()),

            db_host: require_env("DB_HOST")?,
            db_port: env_u16("DB_PORT", 5432)?,
            db_user: env("DB_USER", "postgres".to_string()),
            db_password: optional_env("DB_PASSWORD"),
            db_name: env("DB_NAME", "archypix".to_string()),

            redis_host: require_env("REDIS_HOST")?,
            redis_port: env_u16("REDIS_PORT", 6379)?,
            redis_user: optional_env("REDIS_USER"),
            redis_password: optional_env("REDIS_PASSWORD"),
            redis_db: env_u8("REDIS_DB", 0)?,

            cors_origins: require_env("CORS_ORIGINS")?
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),

            back_domain: require_env("BACK_DOMAIN")?,
            back_use_https,
            global_domain: global_domain.clone(),

            use_resolver,
            // Default: assume the resolver is reachable at the global domain using the
            // same scheme as this backend. Override when using an internal network address.
            resolver_internal_url: env(
                "RESOLVER_INTERNAL_URL",
                format!("{}://{}", back_scheme, global_domain),
            ),
            resolver_jwt_secret: env("RESOLVER_JWT_SECRET", String::new()),
            back_internal_url: optional_env("BACK_INTERNAL_URL"),

            jwt_secret: require_env("JWT_SECRET")?,
            access_token_ttl_secs: env_i64("ACCESS_TOKEN_TTL_SECS", 900)?,
            refresh_token_ttl_secs: env_i64("REFRESH_TOKEN_TTL_SECS", 15_552_000)?,

            webfinger_use_https: env_bool("WEBFINGER_USE_HTTPS", true)?,
            federation_jwt_ttl_secs: env_i64("FEDERATION_JWT_TTL_SECS", 86_400)?,
            federation_backend_cache_ttl_secs: env_u64("FEDERATION_BACKEND_CACHE_TTL_SECS", 3600)?,
            federation_request_timeout_ms: env_u64("FEDERATION_REQUEST_TIMEOUT_MS", 1000)?,

            worker_jwt_secret: require_env("WORKER_JWT_SECRET")?,
            task_queue_concurrency: env_usize("TASK_QUEUE_CONCURRENCY", 4)?,
            job_processing_timeout_secs: env_i64("JOB_PROCESSING_TIMEOUT_SECS", 600)?,
            job_watchdog_interval_secs: env_u64("JOB_WATCHDOG_INTERVAL_SECS", 60)?,
            job_retention_secs: env_i64("JOB_RETENTION_SECS", 2_592_000)?,
            job_cleanup_interval_secs: env_u64("JOB_CLEANUP_INTERVAL_SECS", 86_400)?,
            purge_sweep_interval_secs: env_u64("PURGE_SWEEP_INTERVAL_SECS", 3600)?,
            purge_sweep_batch: env_i64("PURGE_SWEEP_BATCH", 200)?,
            pipeline_poll_interval_secs: env_u64("PIPELINE_POLL_INTERVAL_SECS", 3600)?,
            pipeline_batch_sleep_ms: env_u64("PIPELINE_BATCH_SLEEP_MS", 0)?,
            pipeline_concurrency: env_usize("PIPELINE_CONCURRENCY", 4)?,
            pipeline_retry_backoff_secs: env_i64("PIPELINE_RETRY_BACKOFF_SECS", 60)?,
            pipeline_debounce_ms: env_u64("PIPELINE_DEBOUNCE_MS", 5000)?,
            exif_drain_interval_secs: env_u64("EXIF_DRAIN_INTERVAL_SECS", 5)?,
            exif_drain_batch: env_i64("EXIF_DRAIN_BATCH", 200)?,

            s3_public_endpoint: env("S3_PUBLIC_ENDPOINT", s3_endpoint.clone()),
            s3_workers_endpoint: env("S3_WORKERS_ENDPOINT", s3_endpoint.clone()),
            s3_endpoint,
            s3_access_key: require_env("S3_ACCESS_KEY")?,
            s3_secret_key: require_env("S3_SECRET_KEY")?,
            s3_region: env("S3_REGION", "us-east-1".to_string()),
            s3_bucket_staging: env("S3_BUCKET_STAGING", "archypix-staging".to_string()),
            s3_bucket_pictures: env("S3_BUCKET_PICTURES", "archypix-pictures".to_string()),
            s3_bucket_versions: env("S3_BUCKET_VERSIONS", "archypix-versions".to_string()),
            s3_bucket_small: env("S3_BUCKET_SMALL", "archypix-small".to_string()),
            s3_bucket_medium: env("S3_BUCKET_MEDIUM", "archypix-medium".to_string()),
            s3_bucket_large: env("S3_BUCKET_LARGE", "archypix-large".to_string()),
            s3_presign_ttl_secs: env_u64("S3_PRESIGN_TTL_SECS", 3600)?,
            s3_presign_cache_margin_secs: env_u64("S3_PRESIGN_CACHE_MARGIN_SECS", 600)?,

            webdav_max_upload_bytes: env_u64("WEBDAV_MAX_UPLOAD_BYTES", 5 * 1024 * 1024 * 1024)?,

            rate_limit_login_max: env_u64("RATE_LIMIT_LOGIN_MAX", 10)?,
            rate_limit_login_window_secs: env_u64("RATE_LIMIT_LOGIN_WINDOW_SECS", 300)?,
            rate_limit_register_max: env_u64("RATE_LIMIT_REGISTER_MAX", 5)?,
            rate_limit_register_window_secs: env_u64("RATE_LIMIT_REGISTER_WINDOW_SECS", 3600)?,
            max_pending_outgoing_shares: env_usize("MAX_PENDING_OUTGOING_SHARES", 100)?,
            max_pending_incoming_shares: env_usize("MAX_PENDING_INCOMING_SHARES", 200)?,

            trace_propagation_peers: std::env::var("TRACE_PROPAGATION_PEERS")
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        };

        let storage_buckets = [
            &config.s3_bucket_pictures,
            &config.s3_bucket_versions,
            &config.s3_bucket_small,
            &config.s3_bucket_medium,
            &config.s3_bucket_large,
        ];
        if storage_buckets.contains(&&config.s3_bucket_staging) {
            return Err(anyhow::anyhow!(
                "S3_BUCKET_STAGING must differ from all other bucket names \
                 (pictures/versions/small/medium/large) — it has an expiration rule applied at startup."
            ));
        }

        if config.use_resolver && config.resolver_jwt_secret.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "RESOLVER_JWT_SECRET must be specified when USE_RESOLVER=true."
            ));
        }

        Ok(config)
    }

    // ── Derived URL builders ──────────────────────────────────────────────────

    pub fn back_scheme(&self) -> &'static str {
        if self.back_use_https { "https" } else { "http" }
    }

    /// Full public base URL of this backend, e.g. `https://backend1.example.com`.
    pub fn public_base_url(&self) -> String {
        format!("{}://{}", self.back_scheme(), self.back_domain)
    }

    pub fn webfinger_scheme(&self) -> &'static str {
        if self.webfinger_use_https {
            "https"
        } else {
            "http"
        }
    }

    pub fn database_url(&self) -> String {
        build_postgres_url(
            &self.db_host,
            self.db_port,
            &self.db_user,
            self.db_password.as_deref(),
            &self.db_name,
        )
    }

    pub fn database_url_masked(&self) -> String {
        build_postgres_url(
            &self.db_host,
            self.db_port,
            &self.db_user,
            self.db_password.as_ref().map(|_| "***"),
            &self.db_name,
        )
    }

    pub fn redis_url(&self) -> String {
        build_redis_url(
            &self.redis_host,
            self.redis_port,
            self.redis_user.as_deref(),
            self.redis_password.as_deref(),
            self.redis_db,
        )
    }

    pub fn redis_url_masked(&self) -> String {
        build_redis_url(
            &self.redis_host,
            self.redis_port,
            self.redis_user.as_deref(),
            self.redis_password.as_ref().map(|_| "***"),
            self.redis_db,
        )
    }

    pub fn test_defaults() -> Self {
        Self {
            listen_addr: "127.0.0.1:0".to_string(),
            global_domain: "test.com".to_string(),
            back_domain: "backend.test.com".to_string(),
            back_use_https: false,
            db_host: "localhost".to_string(),
            db_port: 5432,
            db_user: "postgres".to_string(),
            db_password: None,
            db_name: "test".to_string(),
            redis_host: "localhost".to_string(),
            redis_port: 6379,
            redis_user: None,
            redis_password: None,
            redis_db: 0,
            cors_origins: vec![],
            use_resolver: false,
            resolver_internal_url: "http://localhost:8081".to_string(),
            resolver_jwt_secret: String::new(),
            back_internal_url: None,
            jwt_secret: "test_jwt_secret_must_be_long_enough_for_hmac_sha256".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 86400,
            webfinger_use_https: false,
            federation_jwt_ttl_secs: 3600,
            federation_backend_cache_ttl_secs: 300,
            federation_request_timeout_ms: 1000,
            worker_jwt_secret: "test_worker_secret_must_be_long_enough_also".to_string(),
            task_queue_concurrency: 1,
            job_processing_timeout_secs: 600,
            job_watchdog_interval_secs: 60,
            job_retention_secs: 2_592_000,
            job_cleanup_interval_secs: 86_400,
            purge_sweep_interval_secs: 3600,
            purge_sweep_batch: 200,
            pipeline_poll_interval_secs: 3600,
            pipeline_batch_sleep_ms: 0,
            pipeline_concurrency: 4,
            pipeline_retry_backoff_secs: 60,
            pipeline_debounce_ms: 0,
            exif_drain_interval_secs: 5,
            exif_drain_batch: 200,
            s3_endpoint: "http://localhost:9000".to_string(),
            s3_public_endpoint: "http://localhost:9000".to_string(),
            s3_workers_endpoint: "http://localhost:9000".to_string(),
            s3_access_key: "minioadmin".to_string(),
            s3_secret_key: "minioadmin".to_string(),
            s3_region: "us-east-1".to_string(),
            s3_bucket_staging: "archypix-staging".to_string(),
            s3_bucket_pictures: "archypix-pictures".to_string(),
            s3_bucket_versions: "archypix-versions".to_string(),
            s3_bucket_small: "archypix-small".to_string(),
            s3_bucket_medium: "archypix-medium".to_string(),
            s3_bucket_large: "archypix-large".to_string(),
            s3_presign_ttl_secs: 3600,
            s3_presign_cache_margin_secs: 600,
            webdav_max_upload_bytes: 5 * 1024 * 1024 * 1024,
            rate_limit_login_max: 10,
            rate_limit_login_window_secs: 300,
            rate_limit_register_max: 5,
            rate_limit_register_window_secs: 3600,
            max_pending_outgoing_shares: 100,
            max_pending_incoming_shares: 200,
            trace_propagation_peers: vec![],
        }
    }
}

fn build_postgres_url(
    host: &str,
    port: u16,
    user: &str,
    password: Option<&str>,
    db: &str,
) -> String {
    match password {
        Some(pw) => format!("postgres://{}:{}@{}:{}/{}", user, pw, host, port, db),
        None => format!("postgres://{}@{}:{}/{}", user, host, port, db),
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
        (Some(u), Some(pw)) => format!("redis://{}:{}@{}:{}/{}", u, pw, host, port, db),
        (None, Some(pw)) => format!("redis://:{}@{}:{}/{}", pw, host, port, db),
        (Some(u), None) => format!("redis://{}@{}:{}/{}", u, host, port, db),
        (None, None) => format!("redis://{}:{}/{}", host, port, db),
    }
}

fn env(name: &str, default: String) -> String {
    std::env::var(name).unwrap_or(default)
}

fn optional_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn require_env(name: &str) -> anyhow::Result<String> {
    let val = std::env::var(name)
        .map_err(|_| anyhow::anyhow!("{} environment variable must be specified.", name))?;
    if val.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "{} environment variable cannot be empty.",
            name
        ));
    }
    Ok(val)
}

fn env_bool(name: &str, default: bool) -> anyhow::Result<bool> {
    match std::env::var(name) {
        Err(_) => Ok(default),
        Ok(val) => parse_bool(name, &val),
    }
}

fn require_bool_env(name: &str) -> anyhow::Result<bool> {
    let val = require_env(name)?;
    parse_bool(name, &val)
}

fn parse_bool(name: &str, val: &str) -> anyhow::Result<bool> {
    match val.trim().to_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(anyhow::anyhow!(
            "{} must be a boolean (true/false/1/0/yes/no), got: {}",
            name,
            val
        )),
    }
}

fn env_i64(name: &str, default: i64) -> anyhow::Result<i64> {
    let val = std::env::var(name).unwrap_or_else(|_| default.to_string());
    val.trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("{} must be an integer.", name))
}

fn env_u8(name: &str, default: u8) -> anyhow::Result<u8> {
    let val = std::env::var(name).unwrap_or_else(|_| default.to_string());
    val.trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("{} must be a non-negative integer (0-255).", name))
}

fn env_u16(name: &str, default: u16) -> anyhow::Result<u16> {
    let val = std::env::var(name).unwrap_or_else(|_| default.to_string());
    val.trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("{} must be a valid port number (0-65535).", name))
}

fn env_u64(name: &str, default: u64) -> anyhow::Result<u64> {
    let val = std::env::var(name).unwrap_or_else(|_| default.to_string());
    val.trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("{} must be a non-negative integer.", name))
}

fn env_usize(name: &str, default: usize) -> anyhow::Result<usize> {
    let val = std::env::var(name).unwrap_or_else(|_| default.to_string());
    val.trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("{} must be a positive integer.", name))
}
