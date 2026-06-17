use crate::infra::config::Config;
use crate::infra::error::AppError;
use async_trait::async_trait;
use bb8_redis::{
    RedisConnectionManager, bb8,
    redis::{AsyncCommands, cmd},
};
use serde::{Serialize, de::DeserializeOwned};
use std::fmt;
use tracing::info;
use uuid::Uuid;

/// Canonical Redis key definitions. Every key used anywhere in the codebase is listed here.
///
/// All variants hold only `Copy` types (`Uuid`, `&str`) so the enum itself is `Copy`.
#[derive(Copy, Clone)]
pub enum RedisKey<'a> {
    /// Transient upload session during the presigned-PUT window.
    UploadSession(Uuid),
    /// Cached presigned GET URL for a picture — covers owned, same-backend, and cross-instance.
    PictureUrl(Uuid, &'a str),
    /// Cached federation JWT for communicating with `global_domain`.
    FederationToken(&'a str),
    /// Cached backend domain for `username@global_domain`.
    FederationBackend(&'a str, &'a str),
    /// Cached local user UUID for a given username.
    UserByUsername(&'a str),
    /// Cached instance-wide admin analytics (short TTL).
    AdminStats,
    /// Cached per-user admin analytics (short TTL).
    AdminUserStats(Uuid),
    /// Cached WebDAV auth resolution, keyed by the SHA-256 of the presented token
    /// (so the plaintext token is never a Redis key). See 06_webdav.md §3.3.
    WebdavToken(&'a str),
}

impl<'a> RedisKey<'a> {
    pub fn build(&self) -> String {
        match self {
            Self::UploadSession(id) => format!("upload:{id}"),
            Self::PictureUrl(id, variant) => format!("presign:{id}:{variant}"),
            Self::FederationToken(domain) => format!("federation:token:{domain}"),
            Self::FederationBackend(u, d) => format!("federation:backend:{u}@{d}"),
            Self::UserByUsername(username) => format!("user:username:{username}"),
            Self::AdminStats => "admin:stats:instance".to_string(),
            Self::AdminUserStats(id) => format!("admin:stats:user:{id}"),
            Self::WebdavToken(hash) => format!("webdav:token:{hash}"),
        }
    }
}

impl<'a> fmt::Display for RedisKey<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.build())
    }
}

// ── Cache trait ───────────────────────────────────────────────────────────────

/// Abstraction over the cache layer. Implemented by `RedisClient` in production
/// and `InMemoryCache` in tests.
///
/// Note: generic helpers (`cache_get_json`, `cache_set_json_ex`) are free functions
/// in this module rather than trait methods, because generic methods prevent the
/// trait from being used as `dyn Cache`.
#[async_trait]
pub trait Cache: Send + Sync {
    async fn get_str(&self, key: RedisKey<'_>) -> Result<Option<String>, AppError>;
    async fn set_str_ex(
        &self,
        key: RedisKey<'_>,
        value: &str,
        ttl_secs: u64,
    ) -> Result<(), AppError>;
    async fn del(&self, key: RedisKey<'_>) -> Result<(), AppError>;
    /// Return all keys matching a glob-style pattern. Admin/diagnostic use only.
    async fn scan_keys(&self, pattern: &str) -> Result<Vec<String>, AppError>;
}

// ── JSON helpers (free functions to preserve dyn-compatibility) ───────────────

pub async fn cache_get_json<T: DeserializeOwned>(
    cache: &dyn Cache,
    key: RedisKey<'_>,
) -> Result<Option<T>, AppError> {
    cache
        .get_str(key)
        .await?
        .map(|s| {
            serde_json::from_str::<T>(&s).map_err(|e| AppError::InternalServerError(e.to_string()))
        })
        .transpose()
}

pub async fn cache_set_json_ex<T: Serialize>(
    cache: &dyn Cache,
    key: RedisKey<'_>,
    value: &T,
    ttl_secs: u64,
) -> Result<(), AppError> {
    let json =
        serde_json::to_string(value).map_err(|e| AppError::InternalServerError(e.to_string()))?;
    cache.set_str_ex(key, &json, ttl_secs).await
}

// ── RedisClient ───────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct RedisClient {
    pool: bb8::Pool<RedisConnectionManager>,
}

#[async_trait]
impl Cache for RedisClient {
    async fn get_str(&self, key: RedisKey<'_>) -> Result<Option<String>, AppError> {
        let k = key.build();
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        conn.get(&k)
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))
    }

    async fn set_str_ex(
        &self,
        key: RedisKey<'_>,
        value: &str,
        ttl_secs: u64,
    ) -> Result<(), AppError> {
        let k = key.build();
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        let _: () = conn
            .set_ex(&k, value, ttl_secs)
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(())
    }

    async fn del(&self, key: RedisKey<'_>) -> Result<(), AppError> {
        let k = key.build();
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        let _: () = conn
            .del(&k)
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(())
    }

    async fn scan_keys(&self, pattern: &str) -> Result<Vec<String>, AppError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        conn.keys::<_, Vec<String>>(pattern)
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))
    }
}

pub async fn connect(config: &Config) -> anyhow::Result<RedisClient> {
    info!("Connecting to Redis: {}", config.redis_url_masked());
    let manager = RedisConnectionManager::new(config.redis_url())?;
    let pool = bb8::Pool::builder()
        .connection_timeout(std::time::Duration::from_secs(5))
        .build(manager)
        .await?;
    {
        let mut conn = pool.get().await?;
        let reply: String = cmd("PING").query_async(&mut *conn).await?;
        assert_eq!("PONG", reply);
    }
    info!("Connected to Redis");
    Ok(RedisClient { pool })
}
