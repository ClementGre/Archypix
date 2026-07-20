//! The single generic outbound federation call (feature 28 §5.2). Every authenticated verb travels
//! as a `FederationEnvelope` to `POST /api/federation/message`; `send` is the one place that fetches
//! the token, resolves the peer backend, posts the envelope, classifies errors
//! (transient/permanent/version — §6.2/§5.4), and busts a stale backend-URL cache once on failure
//! (§4.5).

use super::FederationClient;
use crate::clients::federation::models::{
    FederationEnvelope, FederationMessageType, FederationResponse,
};
use crate::infra::redis::RedisKey;
use archypix_common::error::AppError;
use tracing::warn;

impl FederationClient {
    /// Deliver `msg` to `peer` and decode its typed response. `token_subject_username` is the local
    /// user on whose behalf the pairwise token is held; `peer_username`/`peer_global_domain` identify
    /// the recipient backend.
    #[tracing::instrument(
        skip(self, msg),
        fields(otel.kind = "client", msg_type = M::TYPE_NAME, %peer_global_domain)
    )]
    pub async fn send<M: FederationMessageType>(
        &self,
        token_subject_username: &str,
        peer_username: &str,
        peer_global_domain: &str,
        msg: M,
    ) -> Result<M::Response, AppError> {
        let token = self
            .get_or_wait_federation_token(token_subject_username, peer_username, peer_global_domain)
            .await?;
        let envelope = FederationEnvelope {
            msg_version: M::VERSION,
            message: msg.into_message(),
        };

        // First attempt against the (possibly cached) backend URL.
        let backend_base_url = self
            .resolve_backend_url(peer_username, peer_global_domain)
            .await?;
        match self
            .post_envelope::<M>(&backend_base_url, &token, peer_global_domain, &envelope)
            .await
        {
            Ok(resp) => Ok(resp),
            // §4.5: a transient failure may mean the peer migrated backends — bust the cache and
            // re-resolve once before surfacing the error.
            Err(e) if is_transient(&e) => {
                let _ = self
                    .cache
                    .del(RedisKey::FederationBackend(
                        peer_username,
                        peer_global_domain,
                    ))
                    .await;
                let backend_base_url = self
                    .resolve_backend_url(peer_username, peer_global_domain)
                    .await?;
                self.post_envelope::<M>(&backend_base_url, &token, peer_global_domain, &envelope)
                    .await
            }
            Err(e) => Err(e),
        }
    }

    async fn post_envelope<M: FederationMessageType>(
        &self,
        backend_base_url: &str,
        token: &str,
        peer_global_domain: &str,
        envelope: &FederationEnvelope,
    ) -> Result<M::Response, AppError> {
        let url = format!("{}/api/federation/message", backend_base_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(token)
            .headers(self.trace_headers_for(peer_global_domain))
            .json(envelope)
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, msg_type = M::TYPE_NAME, "federation: message delivery failed");
                // Connect/timeout/transport → the peer is unreachable right now.
                AppError::ServiceUnavailable(
                    "The recipient's instance is unreachable right now — try again later."
                        .to_string(),
                )
            })?;

        let status = resp.status();
        if status.is_success() {
            let wire: FederationResponse = resp.json().await.map_err(|e| {
                AppError::InternalServerError(format!("federation: bad response body: {e}"))
            })?;
            // Re-encode the tagged response and decode into the concrete per-message shape (the
            // extra `type` tag is ignored). Cheap and keeps `send` fully generic.
            let value = serde_json::to_value(&wire)
                .map_err(|e| AppError::InternalServerError(e.to_string()))?;
            return serde_json::from_value(value).map_err(|e| {
                AppError::InternalServerError(format!("federation: unexpected response shape: {e}"))
            });
        }

        // 426: protocol-version mismatch — produce a directional message (§5.4).
        if status.as_u16() == 426 {
            let body: VersionMismatchBody = resp.json().await.unwrap_or_default();
            let ours = M::VERSION;
            let message = match body.receiver_version {
                Some(theirs) if theirs < ours => {
                    "The recipient's instance is running an older, incompatible version of Archypix."
                }
                Some(_) => {
                    "Your instance is out of date — update to share with this recipient."
                }
                None => "Incompatible Archypix versions between the two instances.",
            };
            return Err(AppError::Custom(
                426,
                serde_json::json!({ "error": message, "version_mismatch": true }),
            ));
        }

        // Everything else: map by status, preferring the peer's `{error}` body message.
        let code = status.as_u16();
        let peer_msg = resp
            .json::<PeerError>()
            .await
            .ok()
            .and_then(|p| p.error)
            .unwrap_or_else(|| format!("federation peer returned {code}"));
        Err(classify_status(code, peer_msg))
    }
}

#[derive(serde::Deserialize, Default)]
struct VersionMismatchBody {
    receiver_version: Option<u16>,
}

#[derive(serde::Deserialize)]
struct PeerError {
    error: Option<String>,
}

/// A transient error means "retry later might help" — connect/timeout/peer-5xx.
fn is_transient(e: &AppError) -> bool {
    matches!(e, AppError::ServiceUnavailable(_))
}

/// Map a peer 4xx/5xx status onto a caller-facing error (§6.2). Transient (5xx) → `503`; the rest
/// keep their specific meaning so the caller surfaces "why" rather than "try later".
fn classify_status(code: u16, msg: String) -> AppError {
    match code {
        404 => AppError::NotFound,
        403 => AppError::Forbidden(msg),
        409 => AppError::Conflict(msg),
        429 => AppError::TooManyRequests(msg),
        400..=499 => AppError::BadRequest(msg),
        _ => AppError::ServiceUnavailable(
            "The recipient's instance is unreachable right now — try again later.".to_string(),
        ),
    }
}
