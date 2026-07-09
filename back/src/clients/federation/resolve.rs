use super::FederationClient;
use crate::infra::redis::RedisKey;
use crate::infra::settings;
use crate::infra::settings::keys;
use archypix_common::error::AppError;
use serde::Deserialize;
use tracing::{debug, trace, warn};

impl FederationClient {
    /// Resolve a user's owning backend base URL, with cache-aside caching (feature 25 replaces the
    /// old `.well-known/webfinger` single-shot with the resolver's `/archypix-resolver/resolve`).
    ///
    /// Hits `{scheme}://{global_domain}/archypix-resolver/resolve?user=&domain=` directly — one HTTP
    /// call, exactly like the old WebFinger lookup. Both a resolver and a standalone backend answer
    /// it with `200 { backend_url }`. A `404` means either no such user or a domain serving no
    /// `resolve` at all (no resolver + prefix not forwarded); we fall back to the domain itself as the
    /// backend (`{scheme}://{global_domain}`) so a bare standalone instance still resolves.
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

        let scheme = settings::webfinger_scheme(&self.settings);
        debug!("federation: resolving backend URL via /archypix-resolver/resolve");
        let resolve_url = format!("{}://{}/archypix-resolver/resolve", scheme, global_domain);
        let response = self
            .http
            .get(&resolve_url)
            .query(&[("user", username), ("domain", global_domain)])
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "federation: resolve request failed");
                AppError::InternalServerError(e.to_string())
            })?;

        let backend_url = if response.status() == reqwest::StatusCode::NOT_FOUND {
            // Standalone backend (no resolver): its own domain is the answer.
            normalize_base_url(&format!("{}://{}", scheme, global_domain))
        } else {
            let body: ResolveResponse = response
                .error_for_status()
                .map_err(|e| AppError::InternalServerError(e.to_string()))?
                .json()
                .await
                .map_err(|e| AppError::InternalServerError(e.to_string()))?;
            normalize_base_url(&body.backend_url)
        };

        debug!(backend_url, "federation: backend URL resolved");

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

#[derive(Deserialize)]
struct ResolveResponse {
    backend_url: String,
}

/// Trim trailing slashes from a base URL, preserving the scheme and host.
/// e.g. `https://backend1.example.com/` → `https://backend1.example.com`
fn normalize_base_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}
