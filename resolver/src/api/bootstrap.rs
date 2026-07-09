//! Bootstrap + resolution endpoints (feature 25), both answered directly at the resolver's domain.
//!
//! - `GET /archypix-resolver/info` — discovery: reports `is_resolver: true` + the public `api_url`.
//! - `GET /archypix-resolver/resolve?user=&domain=` — the federation/login hot path: `@user:domain`
//!   → owning backend URL, in one HTTP call (replaces the old `.well-known/webfinger` query).

use crate::config;
use crate::repository;
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

#[derive(Serialize)]
pub struct ResolverInfo {
    is_resolver: bool,
    api_url: String,
}

/// `GET /archypix-resolver/info` — the resolver always reports itself as such and advertises the
/// public base URL the frontend uses for `/api/public/*` and `/api/resolver-admin/*`.
pub async fn info(State(state): State<AppState>) -> Json<ResolverInfo> {
    Json(ResolverInfo {
        is_resolver: true,
        api_url: config::public_url(&state.config),
    })
}

#[derive(Debug, Deserialize)]
pub struct ResolveQuery {
    user: String,
    domain: String,
}

#[derive(Serialize)]
pub struct ResolveResponse {
    backend_url: String,
}

/// `GET /archypix-resolver/resolve?user=&domain=` — resolve `@user:domain` to the owning backend's
/// public base URL (moka-cached). `404` for an unknown user or a mismatched domain.
pub async fn resolve(
    Query(query): Query<ResolveQuery>,
    State(state): State<AppState>,
) -> Result<Json<ResolveResponse>, AppError> {
    let global_domain = state.global_domain();
    if query.domain != global_domain {
        warn!(domain = %query.domain, "resolve: domain does not match this resolver");
        return Err(AppError::NotFound);
    }

    let backend_url = match state.cache.get(&query.user).await {
        Some(url) => url,
        None => {
            let url = repository::get_backend_url(&state.db, &query.user)
                .await?
                .ok_or_else(|| {
                    warn!(user = %query.user, "resolve: username not found");
                    AppError::NotFound
                })?;
            state.cache.insert(query.user.clone(), url.clone()).await;
            url
        }
    };
    debug!(user = %query.user, "resolve");
    Ok(Json(ResolveResponse { backend_url }))
}
