//! Worker configuration. Parsed through the shared [`Settings`](archypix_common::settings) engine
//! (typed keys, same as the backend/resolver) — the worker is env-only (no DB / dashboard), so the
//! merged snapshot is read once into this `Config`. Multi-value fields (paired backend lists, job
//! types) are derived from the scalar settings.

use archypix_common::job::JobType;
use archypix_common::settings::{SettingKey, SettingSpec, Settings};
use std::collections::HashMap;

pub mod group {
    pub const IDENTITY: &str = "Identity";
    pub const BACKENDS: &str = "Backends";
    pub const JOBS: &str = "Jobs";
    pub const SERVER: &str = "Server";
}

pub mod setting_keys {
    use super::*;
    pub const GLOBAL_DOMAIN: SettingKey<String> = SettingKey::new("global_domain");
    pub const WORKER_JWT_SECRET: SettingKey<String> = SettingKey::new("worker_jwt_secret");
    pub const WORKER_ID: SettingKey<Option<String>> = SettingKey::new("worker_id");
    pub const BACK_URL: SettingKey<Vec<String>> = SettingKey::new("back_url");
    pub const BACK_DOMAIN: SettingKey<Vec<String>> = SettingKey::new("back_domain");
    pub const POLL_INTERVAL_MS: SettingKey<u64> = SettingKey::new("poll_interval_ms");
    pub const MAX_CONCURRENT_JOBS: SettingKey<usize> = SettingKey::new("max_concurrent_jobs");
    pub const JOB_TYPES: SettingKey<Vec<String>> = SettingKey::new("job_types");
    pub const LISTEN_ADDR: SettingKey<String> = SettingKey::new("listen_addr");
}

fn registry() -> Vec<SettingSpec> {
    use setting_keys::*;
    vec![
        SettingSpec::new(GLOBAL_DOMAIN, group::IDENTITY)
            .core()
            .doc("Global identity domain.", "example.com"),
        SettingSpec::new(WORKER_JWT_SECRET, group::IDENTITY)
            .secret()
            .doc("Shared secret for worker JWTs.", ""),
        SettingSpec::new(WORKER_ID, group::IDENTITY)
            .core()
            .nullable()
            .doc("Stable worker id (auto-generated if unset).", "worker-1"),
        SettingSpec::new(BACK_URL, group::BACKENDS).core().doc(
            "Comma-separated backend base URLs (paired with BACK_DOMAIN).",
            "http://backend1:8000",
        ),
        SettingSpec::new(BACK_DOMAIN, group::BACKENDS).core().doc(
            "Comma-separated backend domains (paired with BACK_URL).",
            "backend1.example.com",
        ),
        SettingSpec::new(POLL_INTERVAL_MS, group::JOBS)
            .default("1000")
            .doc("Job poll interval (ms).", "1000"),
        SettingSpec::new(MAX_CONCURRENT_JOBS, group::JOBS)
            .default("6")
            .doc("Max concurrent jobs across all backends.", "6"),
        SettingSpec::new(JOB_TYPES, group::JOBS).default("").doc(
            "Job types this worker handles (empty = all).",
            "gen_thumbnail,edit_picture",
        ),
        SettingSpec::new(LISTEN_ADDR, group::SERVER)
            .core()
            .default("0.0.0.0:80")
            .doc("Health-check server bind address.", "0.0.0.0:8080"),
    ]
}

/// Per-backend connectivity settings.
#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub back_url: String,
    pub back_domain: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub backends: Vec<BackendConfig>,
    pub global_domain: String,
    pub worker_jwt_secret: String,
    pub worker_id: String,
    pub poll_interval_ms: u64,
    pub max_concurrent_jobs: usize,
    /// Job types this worker handles. Empty = accept all types.
    pub job_types: Vec<JobType>,
    pub listen_addr: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();
        let s = Settings::load(&registry(), &HashMap::new())
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;

        let back_urls = s.get(setting_keys::BACK_URL);
        let back_domains = s.get(setting_keys::BACK_DOMAIN);
        anyhow::ensure!(
            !back_urls.is_empty() && back_urls.len() == back_domains.len(),
            "BACK_URL and BACK_DOMAIN must be non-empty and have the same number of comma-separated entries \
             (got {} URLs and {} domains)",
            back_urls.len(),
            back_domains.len()
        );
        let backends = back_urls
            .into_iter()
            .zip(back_domains)
            .map(|(back_url, back_domain)| BackendConfig {
                back_url,
                back_domain,
            })
            .collect();

        let worker_id = s.get(setting_keys::WORKER_ID).unwrap_or_else(|| {
            let short = uuid::Uuid::new_v4().to_string();
            format!("worker-{}", short.split('-').next().unwrap_or("0"))
        });

        let job_types = s
            .get(setting_keys::JOB_TYPES)
            .into_iter()
            .filter_map(|t| match t.parse::<JobType>() {
                Ok(t) => Some(t),
                Err(e) => {
                    tracing::warn!("ignoring unknown job type in JOB_TYPES: {e}");
                    None
                }
            })
            .collect();

        Ok(Config {
            backends,
            global_domain: s.get(setting_keys::GLOBAL_DOMAIN),
            worker_jwt_secret: s.get(setting_keys::WORKER_JWT_SECRET),
            worker_id,
            poll_interval_ms: s.get(setting_keys::POLL_INTERVAL_MS),
            max_concurrent_jobs: s.get(setting_keys::MAX_CONCURRENT_JOBS),
            job_types,
            listen_addr: s.get(setting_keys::LISTEN_ADDR),
        })
    }
}
