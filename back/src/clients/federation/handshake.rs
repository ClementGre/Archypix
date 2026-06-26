use super::FederationClient;
use crate::clients::federation::models::{FederationAuthGrant, FederationAuthRequest};
use crate::domain::auth::TokenType;
use crate::infra::error::AppError;
use crate::infra::redis::RedisKey;
use std::time::Duration;
use tokio::time::{sleep, timeout};
use tracing::{debug, trace, warn};
use uuid::Uuid;

impl FederationClient {
    /// Request a federation token from the remote instance, if not already cached.
    /// Returns `Some(token)` on cache hit, `None` when the async grant is still in flight.
    /// `sender_username` is required so the backend B can resolve back the backend domain of A
    #[tracing::instrument(skip(self), fields(otel.kind = "client", sender = %sender_username, recipient_global_domain = %recipient_global_domain))]
    pub async fn ensure_federation_token(
        &self,
        sender_username: &str,
        recipient_username: &str,
        recipient_global_domain: &str,
    ) -> Result<Option<String>, AppError> {
        if let Some(token) = self
            .cache
            .get_str(RedisKey::FederationToken(recipient_global_domain))
            .await
            .ok()
            .flatten()
        {
            trace!("federation: token resolved from cache");
            return Ok(Some(token));
        }

        let backend_base_url = self
            .resolve_backend_url(recipient_username, recipient_global_domain)
            .await?;

        // Mint a nonce and persist it *before* sending the request. The grant callback must echo
        // this nonce, so an unsolicited POST to `/auth/grant` cannot poison the token cache.
        let nonce = Uuid::new_v4().to_string();
        let _ = self
            .cache
            .set_str_ex(
                RedisKey::FederationAuthNonce(recipient_global_domain),
                &nonce,
                60,
            )
            .await;

        debug!(backend_base_url, "federation: requesting auth token");
        let request_url = format!("{}/api/federation/auth/request", backend_base_url);
        self.http
            .post(&request_url)
            .headers(self.trace_headers_for(recipient_global_domain))
            .json(&FederationAuthRequest {
                requester_instance: self.config.global_domain.clone(),
                username: sender_username.to_string(),
                scope: "federation".to_string(),
                nonce,
            })
            .timeout(Duration::from_millis(
                self.config.federation_request_timeout_ms,
            ))
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "federation: auth request failed");
                AppError::InternalServerError(e.to_string())
            })?
            .error_for_status()
            .map_err(|e| AppError::BadRequest(format!("Federation auth request failed: {e}")))?;

        Ok(None)
    }

    /// Get a valid federation token for `recipient_global_domain`, polling the cache until the
    /// grant callback arrives if the token is not already cached.
    #[tracing::instrument(skip(self), fields(otel.kind = "client", sender = %sender_username, recipient_global_domain = %recipient_global_domain))]
    pub async fn get_or_wait_federation_token(
        &self,
        sender_username: &str,
        recipient_username: &str,
        recipient_global_domain: &str,
    ) -> Result<String, AppError> {
        if let Some(token) = self
            .ensure_federation_token(sender_username, recipient_username, recipient_global_domain)
            .await?
        {
            return Ok(token);
        }

        debug!("federation: waiting for auth token grant");
        let deadline = Duration::from_millis(self.config.federation_request_timeout_ms);
        let domain = recipient_global_domain;
        let cache = self.cache.clone();

        timeout(deadline, async move {
            loop {
                if let Some(token) = cache
                    .get_str(RedisKey::FederationToken(domain))
                    .await
                    .ok()
                    .flatten()
                {
                    return Ok(token);
                }
                sleep(Duration::from_millis(200)).await;
            }
        })
        .await
        .map_err(|_| {
            warn!("federation: auth token grant timed out");
            AppError::BadRequest("Federation token request timed out".to_string())
        })?
    }

    /// Store a federation token received via the `/api/federation/auth/grant` callback.
    /// Verifies the grant's `nonce` against the one persisted.
    #[tracing::instrument(skip(self, token, nonce), fields(otel.kind = "client", issuer_global_domain = %issuer_global_domain, ttl_secs = ttl_secs))]
    pub async fn store_federation_token(
        &self,
        issuer_global_domain: &str,
        token: &str,
        ttl_secs: i64,
        nonce: &str,
    ) -> Result<(), AppError> {
        let ttl = ttl_secs
            .try_into()
            .map_err(|_| AppError::BadRequest("Invalid token TTL".to_string()))?;

        let expected = self
            .cache
            .get_str(RedisKey::FederationAuthNonce(issuer_global_domain))
            .await
            .ok()
            .flatten();
        match expected {
            Some(n) if n == nonce && !nonce.is_empty() => {
                // One-time use: consume the nonce so a replayed grant is rejected.
                let _ = self
                    .cache
                    .del(RedisKey::FederationAuthNonce(issuer_global_domain))
                    .await;
            }
            _ => {
                warn!(
                    "federation: rejected auth grant — no matching pending request (possible poisoning attempt)"
                );
                return Err(AppError::Unauthorized(
                    "Unsolicited or stale federation grant".to_string(),
                ));
            }
        }

        trace!("federation: storing auth token");
        self.cache
            .set_str_ex(RedisKey::FederationToken(issuer_global_domain), token, ttl)
            .await
    }

    /// Issue a federation JWT for a requesting instance (used in the auth handshake).
    pub fn issue_federation_token(
        &self,
        requester_global_domain: &str,
    ) -> Result<String, AppError> {
        self.jwt.issue(
            requester_global_domain,
            None,
            &self.config.global_domain,
            TokenType::Federation,
            false,
            &self.config.back_domain,
            self.config.federation_jwt_ttl_secs,
        )
    }

    /// Send the federation token grant to the requester's backend.
    #[tracing::instrument(skip(self, grant), fields(otel.kind = "client", username = %username, requester_global_domain = %requester_global_domain))]
    pub async fn send_auth_grant(
        &self,
        username: &str,
        requester_global_domain: &str,
        grant: &FederationAuthGrant,
    ) -> Result<(), AppError> {
        let backend_base_url = self
            .resolve_backend_url(username, requester_global_domain)
            .await?;
        debug!(backend_base_url, "federation: sending auth grant");
        let callback_url = format!("{}/api/federation/auth/grant", backend_base_url);
        let resp = self
            .http
            .post(callback_url)
            .headers(self.trace_headers_for(requester_global_domain))
            .json(grant)
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "federation: auth grant delivery failed");
                AppError::InternalServerError(e.to_string())
            })?;

        if !resp.status().is_success() {
            warn!(status = %resp.status(), "federation: auth grant rejected by remote");
            return Err(AppError::InternalServerError(format!(
                "Callback rejected grant: {}",
                resp.status()
            )));
        }
        Ok(())
    }
}
