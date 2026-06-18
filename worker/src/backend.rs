use crate::auth::generate_token;
use crate::config::{BackendConfig, Config};
use crate::error::{Result, WorkerError};
use archypix_common::job::JobType;
use archypix_common::transfer::{ClaimJobResponse, ClaimQuery, CompleteJobRequest, FailJobRequest};
use futures_util::StreamExt;
use reqwest::Client;
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt;
use tracing::debug;
use uuid::Uuid;

struct CachedToken {
    token: String,
    /// Unix timestamp after which the cached token must be refreshed.
    valid_until: i64,
}

/// HTTP client for communicating with one Archypix backend.
#[derive(Clone)]
pub struct BackendClient {
    /// Short-lived client for backend API calls (claim, complete, fail).
    /// 10 s total timeout is plenty for lightweight JSON endpoints.
    api_http: Client,
    /// Separate client for presigned S3 downloads and uploads.
    /// No total-request timeout — large files take as long as they take.
    /// A connect timeout prevents hanging on unreachable endpoints.
    presign_http: Client,
    back_url: String,
    back_domain: String,
    global_domain: String,
    worker_jwt_secret: String,
    worker_id: String,
    job_types: Vec<JobType>,
    /// Cached worker JWT — isolated per BackendClient (each has a distinct `aud`).
    token_cache: Arc<Mutex<Option<CachedToken>>>,
}

impl BackendClient {
    pub fn new(config: &Config, backend: &BackendConfig) -> Self {
        let api_http = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("failed to build API HTTP client");

        let presign_http = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            // No total timeout — transfers of large raw files can take minutes.
            .build()
            .expect("failed to build presign HTTP client");

        Self {
            api_http,
            presign_http,
            back_url: backend.back_url.clone(),
            back_domain: backend.back_domain.clone(),
            global_domain: config.global_domain.clone(),
            worker_jwt_secret: config.worker_jwt_secret.clone(),
            worker_id: config.worker_id.clone(),
            job_types: config.job_types.clone(),
            token_cache: Arc::new(Mutex::new(None)),
        }
    }

    pub fn back_domain(&self) -> &str {
        &self.back_domain
    }

    /// Return the cached worker JWT if still valid, otherwise generate and cache a fresh one.
    ///
    /// Tokens are valid for 300 s; refresh 30 s early to avoid using a token that
    /// expires in flight.
    fn get_or_refresh_token(&self) -> Result<String> {
        let mut guard = self.token_cache.lock().expect("token cache poisoned");
        let now = chrono::Utc::now().timestamp();
        if let Some(ref cached) = *guard {
            if cached.valid_until > now {
                return Ok(cached.token.clone());
            }
        }
        let token = generate_token(
            &self.worker_id,
            &self.global_domain,
            &self.back_domain,
            &self.worker_jwt_secret,
        )?;
        *guard = Some(CachedToken {
            token: token.clone(),
            valid_until: now + 300 - 30,
        });
        Ok(token)
    }

    /// Attempt to claim the next pending job. Returns `None` if no job is available.
    pub async fn claim_next_job(&self) -> Result<Option<ClaimJobResponse>> {
        let token = self.get_or_refresh_token()?;
        let url = format!(
            "{}/api/worker/jobs/next",
            self.back_url.trim_end_matches('/')
        );
        let query = ClaimQuery {
            types: self.job_types.clone(),
        };
        let resp = self
            .api_http
            .get(&url)
            .bearer_auth(&token)
            .query(&query)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(WorkerError::BackendError { status, body });
        }

        let body = resp.bytes().await?;
        if body.as_ref() == b"null" || body.is_empty() {
            return Ok(None);
        }
        let job = serde_json::from_slice::<ClaimJobResponse>(&body)?;
        debug!(job_id = %job.job_id, job_type = %job.job_type, backend = %self.back_domain, "claimed job");
        Ok(Some(job))
    }

    /// Report a job as completed.
    pub async fn complete_job(&self, job_id: Uuid, body: CompleteJobRequest) -> Result<()> {
        let token = self.get_or_refresh_token()?;
        let url = format!(
            "{}/api/worker/jobs/{job_id}/complete",
            self.back_url.trim_end_matches('/')
        );
        let resp = self
            .api_http
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(WorkerError::BackendError { status, body });
        }
        Ok(())
    }

    /// Report a job as failed.
    ///
    /// `claim_token` must match what was issued at claim time.
    /// When `permanent` is `true` the backend marks the job permanently failed
    /// regardless of remaining retries.
    pub async fn fail_job(
        &self,
        job_id: Uuid,
        claim_token: Uuid,
        error: &str,
        permanent: bool,
    ) -> Result<()> {
        let token = self.get_or_refresh_token()?;
        let url = format!(
            "{}/api/worker/jobs/{job_id}/fail",
            self.back_url.trim_end_matches('/')
        );
        let body = FailJobRequest {
            claim_token,
            error: error.to_string(),
            permanent,
        };
        let resp = self
            .api_http
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(WorkerError::BackendError { status, body });
        }
        Ok(())
    }

    /// Download a file from a presigned URL, streaming directly to disk.
    pub async fn download_presigned(&self, url: &str, dest: &std::path::Path) -> Result<()> {
        let resp = self.presign_http.get(url).send().await?;
        if !resp.status().is_success() {
            return Err(WorkerError::BackendError {
                status: resp.status().as_u16(),
                body: "failed to download from presigned URL".to_string(),
            });
        }
        let mut file = tokio::fs::File::create(dest).await?;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            file.write_all(&chunk?).await?;
        }
        file.flush().await?;
        Ok(())
    }

    /// Upload a file to a presigned PUT URL.
    pub async fn upload_presigned(&self, url: &str, src: &std::path::Path) -> Result<()> {
        let data = tokio::fs::read(src).await?;
        let resp = self.presign_http.put(url).body(data).send().await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(WorkerError::BackendError { status, body });
        }
        Ok(())
    }
}
