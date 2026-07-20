use super::FederationClient;
use crate::clients::federation::models::{PresignRequest, PresignRequestItem, PresignResponse};
use archypix_common::error::AppError;
use std::collections::HashMap;
use uuid::Uuid;

/// One presigned URL for a remote picture, plus the owner's advertised expiry (epoch seconds), so
/// the recipient caches it under a truthful lifetime (feature 28 §10).
pub struct RemotePresign {
    pub url: String,
    pub expires_at: Option<i64>,
}

impl FederationClient {
    /// Request presigned URLs for a batch of pictures stored on a remote instance, authorised
    /// by per-picture tokens. A single HTTP call is made per owner backend. The owner identity
    /// is only used to resolve the backend URL — the request body carries just the tokens, which
    /// are self-resolving on the owner's side. Returns a map of `picture_token → (url, expiry)`.
    ///
    /// Stays a separate method from the generic [`FederationClient::send`] — it is unauthenticated
    /// (token-gated) and hits a different endpoint.
    #[tracing::instrument(
        skip(self, pictures),
        fields(otel.kind = "client", owner_username = %owner_username, owner_global_domain = %owner_global_domain, picture_count = pictures.len())
    )]
    pub async fn presign_remote_pictures(
        &self,
        owner_username: &str,
        owner_global_domain: &str,
        pictures: &[(Uuid, &str)],
    ) -> Result<HashMap<Uuid, RemotePresign>, AppError> {
        let backend_base_url = self
            .resolve_backend_url(owner_username, owner_global_domain)
            .await?;
        let url = format!("{}/api/federation/pictures/presign", backend_base_url);

        let items: Vec<PresignRequestItem> = pictures
            .iter()
            .map(|(token, variant)| PresignRequestItem {
                picture_token: *token,
                variant: Some(variant.to_string()),
            })
            .collect();

        let resp = self
            .http
            .post(&url)
            .headers(self.trace_headers_for(owner_global_domain))
            .json(&PresignRequest { pictures: items })
            .send()
            .await
            // A down/slow owner is transient: surface `503` so the read path can degrade instead of
            // 500ing (feature 28 §3).
            .map_err(|_| {
                AppError::ServiceUnavailable(
                    "The owner's instance is unreachable — try again later.".to_string(),
                )
            })?;

        let status = resp.status();
        if !status.is_success() {
            return Err(if status.is_server_error() {
                AppError::ServiceUnavailable(
                    "The owner's instance is unreachable — try again later.".to_string(),
                )
            } else {
                AppError::BadRequest(format!("remote presign failed: {status}"))
            });
        }

        let body: PresignResponse = resp
            .json()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;

        Ok(body
            .urls
            .into_iter()
            .map(|r| {
                (
                    r.picture_token,
                    RemotePresign {
                        url: r.url,
                        expires_at: r.expires_at,
                    },
                )
            })
            .collect())
    }
}
