use super::FederationClient;
use crate::clients::federation::models::{FederationAuthGrant, FederationAuthRequest};
use crate::domain::auth::TokenType;
use crate::infra::redis::{RedisKey, cache_get_json, cache_set_json_ex};
use crate::infra::settings::keys;
use archypix_common::error::AppError;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::{sleep, timeout};
use tracing::{debug, trace, warn};
use uuid::Uuid;

/// The federation-token cache value: the token plus its **local** expiry (epoch seconds), so a
/// proactive refresh can fire before expiry without a cold handshake on the hot path (§4.2).
#[derive(Debug, Serialize, Deserialize)]
struct CachedFederationToken {
    token: String,
    expires_at: i64,
}

impl FederationClient {
    /// Return a valid federation token for `recipient_global_domain` if one is cached, spawning a
    /// proactive background refresh when it is within the refresh margin; otherwise start a
    /// single-flight handshake and return `None` (the grant arrives asynchronously — the caller
    /// polls via [`get_or_wait_federation_token`]).
    #[tracing::instrument(skip(self), fields(otel.kind = "client", sender = %sender_username, recipient_global_domain = %recipient_global_domain))]
    pub async fn ensure_federation_token(
        &self,
        sender_username: &str,
        recipient_username: &str,
        recipient_global_domain: &str,
    ) -> Result<Option<String>, AppError> {
        if let Some(cached) = cache_get_json::<CachedFederationToken>(
            self.cache.as_ref(),
            RedisKey::FederationToken(recipient_global_domain),
        )
            .await
            .ok()
            .flatten()
        {
            let now = Utc::now().timestamp();
            let margin = self.settings.get(keys::FEDERATION_TOKEN_REFRESH_MARGIN_SECS);
            if now < cached.expires_at - margin {
                trace!("federation: token resolved from cache");
                return Ok(Some(cached.token));
            }
            if now < cached.expires_at {
                // Stale-but-valid: refresh in the background, never block the hot path (§4.2).
                trace!("federation: token near expiry — spawning background refresh");
                let this = self.clone();
                let (s, ru, rd) = (
                    sender_username.to_string(),
                    recipient_username.to_string(),
                    recipient_global_domain.to_string(),
                );
                tokio::spawn(async move {
                    let _ = this.perform_handshake(&s, &ru, &rd).await;
                });
                return Ok(Some(cached.token));
            }
            // Expired → fall through to a synchronous handshake.
        }

        self.perform_handshake(sender_username, recipient_username, recipient_global_domain)
            .await?;
        Ok(None)
    }

    /// Single-flighted handshake (§4.3): the first caller for a peer acquires a Redis lock and sends
    /// `auth/request`; concurrent callers no-op and wait for the shared grant to land in the cache.
    #[tracing::instrument(skip(self), fields(otel.kind = "client", recipient_global_domain = %recipient_global_domain))]
    async fn perform_handshake(
        &self,
        sender_username: &str,
        recipient_username: &str,
        recipient_global_domain: &str,
    ) -> Result<(), AppError> {
        let lock_ttl = self.settings.get(keys::FEDERATION_GRANT_WAIT_MS).div_ceil(1000);
        let acquired = self
            .cache
            .set_str_nx_ex(
                RedisKey::FederationRefreshLock(recipient_global_domain),
                "1",
                lock_ttl.max(1),
            )
            .await
            .unwrap_or(true);
        if !acquired {
            // Another handshake to this peer is in flight — let it land the shared grant.
            trace!("federation: handshake already in flight for peer");
            return Ok(());
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
                requester_instance: self.settings.get(keys::GLOBAL_DOMAIN).clone(),
                username: sender_username.to_string(),
                scope: "federation".to_string(),
                nonce,
            })
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "federation: auth request failed");
                AppError::ServiceUnavailable(
                    "The recipient's instance is unreachable right now — try again later."
                        .to_string(),
                )
            })?
            .error_for_status()
            .map_err(|e| AppError::BadRequest(format!("Federation auth request failed: {e}")))?;
        Ok(())
    }

    /// Get a valid federation token for `recipient_global_domain`, polling the cache until the
    /// grant callback arrives if the token is not already cached (bounded by `FEDERATION_GRANT_WAIT_MS`).
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
        let deadline = Duration::from_millis(self.settings.get(keys::FEDERATION_GRANT_WAIT_MS));
        let domain = recipient_global_domain;
        let cache = self.cache.clone();

        timeout(deadline, async move {
            loop {
                if let Some(cached) = cache_get_json::<CachedFederationToken>(
                    cache.as_ref(),
                    RedisKey::FederationToken(domain),
                )
                    .await
                    .ok()
                    .flatten()
                {
                    return Ok(cached.token);
                }
                sleep(Duration::from_millis(200)).await;
            }
        })
        .await
        .map_err(|_| {
            warn!("federation: auth token grant timed out");
            AppError::ServiceUnavailable(
                "The recipient's instance is unreachable right now — try again later.".to_string(),
            )
        })?
    }

    /// Store a federation token received via the `/api/federation/auth/grant` callback.
    /// Verifies the grant's `nonce` against the one persisted, then caches `{ token, expires_at }`
    /// with `expires_at = now + ttl_secs` against this instance's own clock (§4.4 — skew-independent).
    #[tracing::instrument(skip(self, token, nonce), fields(otel.kind = "client", issuer_global_domain = %issuer_global_domain, ttl_secs = ttl_secs))]
    pub async fn store_federation_token(
        &self,
        issuer_global_domain: &str,
        token: &str,
        ttl_secs: i64,
        nonce: &str,
    ) -> Result<(), AppError> {
        if ttl_secs <= 0 {
            return Err(AppError::BadRequest("Token already expired".to_string()));
        }
        let ttl: u64 = ttl_secs
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
        cache_set_json_ex(
            self.cache.as_ref(),
            RedisKey::FederationToken(issuer_global_domain),
            &CachedFederationToken {
                token: token.to_string(),
                expires_at: Utc::now().timestamp() + ttl_secs,
            },
            ttl,
        )
            .await
    }

    /// Issue a federation JWT for a requesting instance (used in the auth handshake).
    pub fn issue_federation_token(
        &self,
        requester_global_domain: &str,
    ) -> Result<String, AppError> {
        self.jwt
            .issue(
                requester_global_domain,
                None,
                &self.settings.get(keys::GLOBAL_DOMAIN),
                TokenType::Federation,
                false,
                &self.settings.get(keys::BACK_DOMAIN),
                self.settings.get(keys::FEDERATION_JWT_TTL_SECS),
            )
            .map_err(Into::into)
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
                AppError::ServiceUnavailable(
                    "The requester's instance is unreachable right now — try again later."
                        .to_string(),
                )
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
