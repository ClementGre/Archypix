use super::FederationClient;
use crate::infra::redis::RedisKey;
use crate::infra::settings;
use crate::infra::settings::keys;
use archypix_common::error::AppError;
use serde::Deserialize;
use tracing::{debug, trace, warn};

impl FederationClient {
    /// Resolve a user's owning backend base URL via WebFinger, with cache-aside caching.
    ///
    /// Queries `{webfinger_scheme}://{global_domain}/.well-known/webfinger` and returns the
    /// full `backend_url` link href (e.g. `https://backend1.example.com`), which already
    /// includes the correct scheme as advertised by the resolver.
    ///
    /// Result is cached under `FederationBackend(username, global_domain)`.
    #[tracing::instrument(skip(self), fields(otel.kind = "client", username = %username, global_domain = %global_domain))]
    pub async fn resolve_backend_url(
        &self,
        username: &str,
        global_domain: &str,
    ) -> Result<String, AppError> {
        if let Some(cached) = self
            .cache
            .get_str(RedisKey::FederationBackend(username, global_domain))
            .await
            .ok()
            .flatten()
        {
            trace!("federation: backend URL resolved from cache");
            return Ok(cached);
        }

        debug!("federation: resolving backend URL via WebFinger");
        let webfinger_url = format!(
            "{}://{}/.well-known/webfinger",
            settings::webfinger_scheme(&self.settings),
            global_domain
        );
        let response = self
            .http
            .get(&webfinger_url)
            .query(&[(
                "resource",
                format!("archypix:@{}:{}", username, global_domain),
            )])
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "federation: WebFinger request failed");
                AppError::InternalServerError(e.to_string())
            })?
            .error_for_status()
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;

        let body: WebFingerResponse = response
            .json()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;

        let backend_url = body
            .links
            .iter()
            .find(|l| l.rel == "backend_url")
            .map(|l| normalize_base_url(&l.href))
            .ok_or_else(|| AppError::BadRequest("Missing backend_url in WebFinger".to_string()))?;

        debug!(
            backend_url,
            "federation: backend URL resolved via WebFinger"
        );

        let _ = self
            .cache
            .set_str_ex(
                RedisKey::FederationBackend(username, global_domain),
                &backend_url,
                self.settings.get(keys::FEDERATION_BACKEND_CACHE_TTL_SECS),
            )
            .await;

        Ok(backend_url)
    }
}

// ── WebFinger response types ──────────────────────────────────────────────────

#[derive(Deserialize)]
struct WebFingerResponse {
    links: Vec<WebFingerLink>,
}

#[derive(Deserialize)]
struct WebFingerLink {
    rel: String,
    href: String,
}

/// Trim trailing slashes from a base URL, preserving the scheme and host.
/// e.g. `https://backend1.example.com/` → `https://backend1.example.com`
fn normalize_base_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}
