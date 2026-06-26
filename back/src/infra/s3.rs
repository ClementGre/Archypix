use crate::infra::config::Config;
use crate::infra::error::AppError;
use async_trait::async_trait;
use aws_config::meta::region::RegionProviderChain;
use aws_config::{BehaviorVersion, Region};
use aws_sdk_s3::Client;
use aws_sdk_s3::config::Credentials;
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{
    BucketLifecycleConfiguration, ExpirationStatus, LifecycleExpiration, LifecycleRule,
    LifecycleRuleFilter,
};
use base64::Engine as _;
use std::time::Duration;
use tracing::{info, warn};
use uuid::Uuid;

// ── Storage trait ─────────────────────────────────────────────────────────────

/// Abstraction over the object storage layer. Implemented by `StorageClient` in production
/// and `MockStorage` in tests.
#[async_trait]
pub trait Storage: Send + Sync {
    async fn presign_get(&self, bucket: &str, key: &str) -> Result<String, AppError>;
    async fn presign_put(&self, bucket: &str, key: &str) -> Result<String, AppError>;
    async fn presign_get_worker(&self, bucket: &str, key: &str) -> Result<String, AppError>;
    async fn presign_put_worker(&self, bucket: &str, key: &str) -> Result<String, AppError>;
    async fn copy_object(
        &self,
        src_bucket: &str,
        src_key: &str,
        dst_bucket: &str,
        dst_key: &str,
    ) -> Result<(), AppError>;
    async fn delete_object(&self, bucket: &str, key: &str) -> Result<(), AppError>;
    /// Fetch an object's bytes (server-side, internal endpoint). Used by the WebDAV
    async fn get_object(&self, bucket: &str, key: &str) -> Result<Vec<u8>, AppError>;
    /// Return an object's size in bytes (a `HEAD`). Used to read the authoritative `file_size`
    async fn object_size(&self, bucket: &str, key: &str) -> Result<i64, AppError>;
    /// Upload bytes to a key (server-side, internal endpoint).
    async fn put_object(
        &self,
        bucket: &str,
        key: &str,
        body: Vec<u8>,
        content_type: Option<&str>,
    ) -> Result<(), AppError>;
    /// Stream an on-disk file to a key without buffering it in memory.
    async fn put_object_file(
        &self,
        bucket: &str,
        key: &str,
        path: &std::path::Path,
        content_type: Option<&str>,
    ) -> Result<(), AppError>;
}

pub fn picture_key(user_id: Uuid, picture_id: Uuid) -> String {
    format!("{}/{}", user_id, picture_id)
}

pub fn version_key(user_id: Uuid, picture_id: Uuid, version_id: Uuid) -> String {
    format!("{}/{}/{}", user_id, picture_id, version_id)
}

/// Thin wrapper around the S3 client that adds presigned URL helpers.
#[derive(Clone)]
pub struct StorageClient {
    /// Used for internal operations: uploads, copies, deletes, bucket management.
    client: Client,
    /// Configured with `s3_public_endpoint` — presigned URLs reachable by browsers.
    presign_client: Client,
    /// Configured with `s3_workers_endpoint` — presigned URLs reachable by worker processes.
    worker_presign_client: Client,
    presign_ttl: Duration,
}

impl StorageClient {
    pub fn new(
        client: Client,
        presign_client: Client,
        worker_presign_client: Client,
        presign_ttl: Duration,
    ) -> Self {
        Self {
            client,
            presign_client,
            worker_presign_client,
            presign_ttl,
        }
    }

    // ── Browser-facing presigns ───────────────────────────────────────────────

    #[tracing::instrument(skip(self))]
    pub async fn presign_get(&self, bucket: &str, key: &str) -> Result<String, AppError> {
        presign_get_with(&self.presign_client, bucket, key, self.presign_ttl).await
    }

    #[tracing::instrument(skip(self))]
    pub async fn presign_put(&self, bucket: &str, key: &str) -> Result<String, AppError> {
        presign_put_with(&self.presign_client, bucket, key, self.presign_ttl).await
    }

    // ── Worker-facing presigns ────────────────────────────────────────────────

    /// Presign a GET URL for worker download — uses `S3_WORKERS_ENDPOINT`.
    #[tracing::instrument(skip(self))]
    pub async fn presign_get_worker(&self, bucket: &str, key: &str) -> Result<String, AppError> {
        presign_get_with(&self.worker_presign_client, bucket, key, self.presign_ttl).await
    }

    /// Presign a PUT URL for worker upload — uses `S3_WORKERS_ENDPOINT`.
    #[tracing::instrument(skip(self))]
    pub async fn presign_put_worker(&self, bucket: &str, key: &str) -> Result<String, AppError> {
        presign_put_with(&self.worker_presign_client, bucket, key, self.presign_ttl).await
    }

    #[tracing::instrument(skip(self), fields(otel.kind = "client", peer.service = "s3"))]
    pub async fn copy_object(
        &self,
        src_bucket: &str,
        src_key: &str,
        dst_bucket: &str,
        dst_key: &str,
    ) -> Result<(), AppError> {
        self.client
            .copy_object()
            .copy_source(format!("{}/{}", src_bucket, src_key))
            .bucket(dst_bucket)
            .key(dst_key)
            .send()
            .await
            .map(|_| ())
            .map_err(|e| AppError::InternalServerError(e.to_string()))
    }

    #[tracing::instrument(skip(self), fields(otel.kind = "client", peer.service = "s3"))]
    pub async fn delete_object(&self, bucket: &str, key: &str) -> Result<(), AppError> {
        self.client
            .delete_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map(|_| ())
            .map_err(|e| AppError::InternalServerError(e.to_string()))
    }

    #[tracing::instrument(skip(self), fields(otel.kind = "client", peer.service = "s3"))]
    pub async fn get_object(&self, bucket: &str, key: &str) -> Result<Vec<u8>, AppError> {
        let out = self
            .client
            .get_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        let data = out
            .body
            .collect()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(data.into_bytes().to_vec())
    }

    /// `HEAD` an object and return its size in bytes. Errors if the object is missing or the
    /// store does not report a content length.

    #[tracing::instrument(skip(self), fields(otel.kind = "client", peer.service = "s3"))]
    pub async fn object_size(&self, bucket: &str, key: &str) -> Result<i64, AppError> {
        let out = self
            .client
            .head_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        out.content_length().ok_or_else(|| {
            AppError::InternalServerError(format!(
                "S3 HEAD {bucket}/{key} returned no content length"
            ))
        })
    }

    #[tracing::instrument(skip(self), fields(otel.kind = "client", peer.service = "s3"))]
    pub async fn put_object(
        &self,
        bucket: &str,
        key: &str,
        body: Vec<u8>,
        content_type: Option<&str>,
    ) -> Result<(), AppError> {
        let mut req = self
            .client
            .put_object()
            .bucket(bucket)
            .key(key)
            .body(ByteStream::from(body));
        if let Some(ct) = content_type {
            req = req.content_type(ct);
        }
        req.send()
            .await
            .map(|_| ())
            .map_err(|e| AppError::InternalServerError(e.to_string()))
    }

    #[tracing::instrument(skip(self), fields(otel.kind = "client", peer.service = "s3"))]
    pub async fn put_object_file(
        &self,
        bucket: &str,
        key: &str,
        path: &std::path::Path,
        content_type: Option<&str>,
    ) -> Result<(), AppError> {
        // `ByteStream::from_path` reads the file in chunks rather than loading it into memory.
        let body = ByteStream::from_path(path)
            .await
            .map_err(|e| AppError::InternalServerError(format!("open upload temp file: {e}")))?;
        let mut req = self.client.put_object().bucket(bucket).key(key).body(body);
        if let Some(ct) = content_type {
            req = req.content_type(ct);
        }
        req.send()
            .await
            .map(|_| ())
            .map_err(|e| AppError::InternalServerError(e.to_string()))
    }
}

#[async_trait]
impl Storage for StorageClient {
    async fn presign_get(&self, bucket: &str, key: &str) -> Result<String, AppError> {
        self.presign_get(bucket, key).await
    }
    async fn presign_put(&self, bucket: &str, key: &str) -> Result<String, AppError> {
        self.presign_put(bucket, key).await
    }
    async fn presign_get_worker(&self, bucket: &str, key: &str) -> Result<String, AppError> {
        self.presign_get_worker(bucket, key).await
    }
    async fn presign_put_worker(&self, bucket: &str, key: &str) -> Result<String, AppError> {
        self.presign_put_worker(bucket, key).await
    }
    async fn copy_object(
        &self,
        src_bucket: &str,
        src_key: &str,
        dst_bucket: &str,
        dst_key: &str,
    ) -> Result<(), AppError> {
        self.copy_object(src_bucket, src_key, dst_bucket, dst_key)
            .await
    }
    async fn delete_object(&self, bucket: &str, key: &str) -> Result<(), AppError> {
        self.delete_object(bucket, key).await
    }
    async fn get_object(&self, bucket: &str, key: &str) -> Result<Vec<u8>, AppError> {
        self.get_object(bucket, key).await
    }
    async fn object_size(&self, bucket: &str, key: &str) -> Result<i64, AppError> {
        self.object_size(bucket, key).await
    }
    async fn put_object(
        &self,
        bucket: &str,
        key: &str,
        body: Vec<u8>,
        content_type: Option<&str>,
    ) -> Result<(), AppError> {
        self.put_object(bucket, key, body, content_type).await
    }
    async fn put_object_file(
        &self,
        bucket: &str,
        key: &str,
        path: &std::path::Path,
        content_type: Option<&str>,
    ) -> Result<(), AppError> {
        self.put_object_file(bucket, key, path, content_type).await
    }
}

pub async fn connect(config: &Config) -> anyhow::Result<StorageClient> {
    let region = Region::new(config.s3_region.clone());
    let region_provider = RegionProviderChain::first_try(region);
    let credentials = Credentials::new(
        config.s3_access_key.clone(),
        config.s3_secret_key.clone(),
        None,
        None,
        "static",
    );
    // Build shared config without an endpoint — each client sets its own below.
    let shared_config = aws_config::defaults(BehaviorVersion::v2026_01_12())
        .region(region_provider)
        .credentials_provider(credentials)
        .load()
        .await;

    let mk_client = |endpoint: &str| {
        Client::from_conf(
            aws_sdk_s3::config::Builder::from(&shared_config)
                .endpoint_url(endpoint.to_string())
                .force_path_style(true)
                .build(),
        )
    };

    let client = mk_client(&config.s3_endpoint);
    let presign_client = mk_client(&config.s3_public_endpoint);
    let worker_presign_client = mk_client(&config.s3_workers_endpoint);

    info!(
        "Connecting to MinIO/S3: {} (public: {}, workers: {})",
        config.s3_endpoint, config.s3_public_endpoint, config.s3_workers_endpoint
    );
    client
        .list_buckets()
        .send()
        .await
        .map_err(|e| match e.as_service_error() {
            Some(svc) => anyhow::anyhow!(
                "Failed to connect to MinIO/S3 at {}: {} (code: {}, message: {})",
                config.s3_endpoint,
                svc,
                svc.meta().code().unwrap_or("unknown"),
                svc.meta().message().unwrap_or("no message"),
            ),
            None => anyhow::anyhow!(
                "Failed to connect to MinIO/S3 at {}: {}",
                config.s3_endpoint,
                e
            ),
        })?;
    info!("Connected to MinIO/S3");

    let buckets = [
        config.s3_bucket_staging.as_str(),
        config.s3_bucket_pictures.as_str(),
        config.s3_bucket_versions.as_str(),
        config.s3_bucket_small.as_str(),
        config.s3_bucket_medium.as_str(),
        config.s3_bucket_large.as_str(),
    ];
    ensure_buckets(&client, &buckets).await?;
    if let Err(e) = ensure_staging_lifecycle(&client, &config.s3_bucket_staging).await {
        warn!("{}", e);
        warn!(
            "Staging bucket '{}' will not auto-expire — orphaned objects must be cleaned manually.",
            config.s3_bucket_staging
        );
    }

    Ok(StorageClient::new(
        client,
        presign_client,
        worker_presign_client,
        Duration::from_secs(config.s3_presign_ttl_secs),
    ))
}

// ── Shared presign helpers ────────────────────────────────────────────────────

async fn presign_get_with(
    client: &Client,
    bucket: &str,
    key: &str,
    ttl: Duration,
) -> Result<String, AppError> {
    let config = PresigningConfig::expires_in(ttl)
        .map_err(|e| AppError::InternalServerError(format!("presign config: {e}")))?;
    client
        .get_object()
        .bucket(bucket)
        .key(key)
        .presigned(config)
        .await
        .map(|p| p.uri().to_string())
        .map_err(|e| AppError::InternalServerError(e.to_string()))
}

async fn presign_put_with(
    client: &Client,
    bucket: &str,
    key: &str,
    ttl: Duration,
) -> Result<String, AppError> {
    let config = PresigningConfig::expires_in(ttl)
        .map_err(|e| AppError::InternalServerError(format!("presign config: {e}")))?;
    client
        .put_object()
        .bucket(bucket)
        .key(key)
        .presigned(config)
        .await
        .map(|p| p.uri().to_string())
        .map_err(|e| AppError::InternalServerError(e.to_string()))
}

async fn ensure_staging_lifecycle(client: &Client, bucket: &str) -> anyhow::Result<()> {
    let expiration = LifecycleExpiration::builder().days(1).build();
    let rule = LifecycleRule::builder()
        .id("expire-staging")
        .status(ExpirationStatus::Enabled)
        .filter(LifecycleRuleFilter::builder().prefix("").build())
        .expiration(expiration)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build lifecycle rule: {}", e))?;
    let lifecycle_config = BucketLifecycleConfiguration::builder()
        .rules(rule)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build lifecycle config: {}", e))?;
    if let Err(e) = client
        .put_bucket_lifecycle_configuration()
        .bucket(bucket)
        .lifecycle_configuration(lifecycle_config)
        .customize()
        .mutate_request(|req| {
            // MinIO requires a Content-MD5 header; the AWS SDK does not add it automatically.
            if let Some(body) = req.body().bytes() {
                let digest = md5::compute(body);
                let encoded = base64::engine::general_purpose::STANDARD.encode(digest.as_ref());
                req.headers_mut().insert(
                    http::header::HeaderName::from_static("content-md5"),
                    http::header::HeaderValue::from_str(&encoded).unwrap(),
                );
            }
        })
        .send()
        .await
    {
        let svc = e.into_service_error();
        return Err(anyhow::anyhow!(
            "Failed to set lifecycle rule on '{}': {} (code: {}, message: {})",
            bucket,
            svc,
            svc.meta().code().unwrap_or("unknown"),
            svc.meta().message().unwrap_or("no message"),
        ));
    }
    info!(
        "Lifecycle rule set on staging bucket: {} (1-day expiry)",
        bucket
    );
    Ok(())
}

async fn ensure_buckets(client: &Client, buckets: &[&str]) -> anyhow::Result<()> {
    for &bucket in buckets {
        match client.create_bucket().bucket(bucket).send().await {
            Ok(_) => info!("Created S3 bucket: {}", bucket),
            Err(e) => {
                let svc = e.into_service_error();
                if svc.is_bucket_already_owned_by_you() || svc.is_bucket_already_exists() {
                    // Bucket already exists — nothing to do.
                } else {
                    return Err(anyhow::anyhow!(
                        "Failed to create bucket '{}': {} (code: {}, message: {})",
                        bucket,
                        svc,
                        svc.meta().code().unwrap_or("unknown"),
                        svc.meta().message().unwrap_or("no message"),
                    ));
                }
            }
        }
    }
    Ok(())
}
