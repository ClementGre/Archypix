use anyhow::Context;
use archypix_common::job::JobType;

/// Per-backend connectivity settings.
#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub back_url: String,
    pub back_domain: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    // Backends (one or more)
    pub backends: Vec<BackendConfig>,

    // Shared worker identity and credentials
    pub global_domain: String,
    pub worker_jwt_secret: String,
    pub worker_id: String,

    // Job polling
    pub poll_interval_ms: u64,
    pub max_concurrent_jobs: usize,
    /// Job types this worker handles. Empty = accept all types.
    pub job_types: Vec<JobType>,

    // HTTP server (health check)
    pub listen_addr: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();

        let back_url_raw = require_env("BACK_URL")?;
        let back_domain_raw = require_env("BACK_DOMAIN")?;

        let back_urls: Vec<&str> = back_url_raw.split(',').map(str::trim).collect();
        let back_domains: Vec<&str> = back_domain_raw.split(',').map(str::trim).collect();

        anyhow::ensure!(
            back_urls.len() == back_domains.len(),
            "BACK_URL and BACK_DOMAIN must have the same number of comma-separated entries \
             (got {} URLs and {} domains)",
            back_urls.len(),
            back_domains.len()
        );

        let backends: Vec<BackendConfig> = back_urls
            .into_iter()
            .zip(back_domains.into_iter())
            .map(|(url, domain)| {
                anyhow::ensure!(!url.is_empty(), "BACK_URL contains an empty entry");
                anyhow::ensure!(!domain.is_empty(), "BACK_DOMAIN contains an empty entry");
                Ok(BackendConfig {
                    back_url: url.to_string(),
                    back_domain: domain.to_string(),
                })
            })
            .collect::<anyhow::Result<_>>()?;

        let global_domain = require_env("GLOBAL_DOMAIN")?;
        let worker_jwt_secret = require_env("WORKER_JWT_SECRET")?;

        let worker_id = std::env::var("WORKER_ID").unwrap_or_else(|_| {
            format!(
                "worker-{}",
                uuid::Uuid::new_v4()
                    .to_string()
                    .split('-')
                    .next()
                    .unwrap_or("0")
            )
        });

        let poll_interval_ms = env_u64("POLL_INTERVAL_MS", 1000)?;
        let max_concurrent_jobs = env_usize("MAX_CONCURRENT_JOBS", 6)?;

        let job_types = std::env::var("JOB_TYPES")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .filter_map(|s| match s.parse::<JobType>() {
                Ok(t) => Some(t),
                Err(e) => {
                    tracing::warn!("ignoring unknown job type in JOB_TYPES: {e}");
                    None
                }
            })
            .collect();

        let listen_addr = std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:80".to_string());

        Ok(Config {
            backends,
            global_domain,
            worker_jwt_secret,
            worker_id,
            poll_interval_ms,
            max_concurrent_jobs,
            job_types,
            listen_addr,
        })
    }
}

fn require_env(name: &str) -> anyhow::Result<String> {
    let val = std::env::var(name).with_context(|| format!("{name} must be specified"))?;
    if val.trim().is_empty() {
        anyhow::bail!("{name} cannot be empty");
    }
    Ok(val)
}

fn env_u64(name: &str, default: u64) -> anyhow::Result<u64> {
    let val = std::env::var(name).unwrap_or_else(|_| default.to_string());
    val.trim()
        .parse()
        .with_context(|| format!("{name} must be a non-negative integer"))
}

fn env_usize(name: &str, default: usize) -> anyhow::Result<usize> {
    let val = std::env::var(name).unwrap_or_else(|_| default.to_string());
    val.trim()
        .parse()
        .with_context(|| format!("{name} must be a positive integer"))
}
