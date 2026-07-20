use super::FederationClient;
use crate::infra::redis::RedisKey;
use crate::infra::settings;
use crate::infra::settings::keys;
use archypix_common::error::AppError;
use serde::Deserialize;
use tracing::{debug, trace, warn};

/// TTL of the long-lived stale backend-URL fallback (§4.5) — long enough to ride out a resolver
/// outage, short enough that a genuinely migrated peer eventually stops being served the old URL.
const STALE_BACKEND_TTL_SECS: u64 = 30 * 24 * 3600;

impl FederationClient {
    /// Resolve a user's owning backend base URL, with cache-aside caching, via the resolver's
    /// single-shot `/archypix-resolver/resolve` (feature 25).
    ///
    /// Hits `{scheme}://{global_domain}/archypix-resolver/resolve?user=&domain=` directly — one HTTP
    /// call. Both a resolver and a standalone backend answer
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

        let scheme = settings::federation_scheme(&self.settings);
        debug!("federation: resolving backend URL via /archypix-resolver/resolve");
        let resolve_url = format!("{}://{}/archypix-resolver/resolve", scheme, global_domain);
        let response = match self
            .http
            .get(&resolve_url)
            .query(&[("user", username), ("domain", global_domain)])
            .send()
            .await
        {
            Ok(r) => r,
            // §4.5: the resolver is unreachable (connection error, *not* a 404). Serve a stale
            // fallback URL for an already-known peer so a resolver blip is non-fatal.
            Err(e) => {
                warn!(error = %e, "federation: resolve request failed");
                if let Some(stale) = self
                    .cache
                    .get_str(RedisKey::FederationBackendStale(username, global_domain))
                    .await
                    .ok()
                    .flatten()
                {
                    debug!("federation: serving stale backend URL (resolver unreachable)");
                    return Ok(stale);
                }
                return Err(AppError::ServiceUnavailable(
                    "The resolver is unreachable and no cached backend is known.".to_string(),
                ));
            }
        };

        // A resolver-answered 404 ("domain is its own backend") must NOT be cached — a transient
        // resolver 404 must not pin a wrong backend (feature-27-era decision, §4.5).
        let (backend_url, cacheable) = if response.status() == reqwest::StatusCode::NOT_FOUND {
            // Standalone backend (no resolver): its own domain is the answer.
            (
                normalize_base_url(&format!("{}://{}", scheme, global_domain)),
                false,
            )
        } else {
            let body: ResolveResponse = response
                .error_for_status()
                .map_err(|e| AppError::InternalServerError(e.to_string()))?
                .json()
                .await
                .map_err(|e| AppError::InternalServerError(e.to_string()))?;
            (normalize_base_url(&body.backend_url), true)
        };

        debug!(backend_url, "federation: backend URL resolved");

        if cacheable {
            let _ = self
                .cache
                .set_str_ex(
                    RedisKey::FederationBackend(username, global_domain),
                    &backend_url,
                    self.settings.get(keys::FEDERATION_BACKEND_CACHE_TTL_SECS),
                )
                .await;
            // Long-lived fallback for the resolver-blip case (§4.5).
            let _ = self
                .cache
                .set_str_ex(
                    RedisKey::FederationBackendStale(username, global_domain),
                    &backend_url,
                    STALE_BACKEND_TTL_SECS,
                )
                .await;
        }

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
