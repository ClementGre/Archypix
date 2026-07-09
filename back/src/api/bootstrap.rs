use crate::infra::settings;
use crate::infra::settings::keys;
use crate::repository::user::UserRepository;
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct ResolverInfo {
    is_resolver: bool,
    api_url: String,
}

/// `GET /archypix-resolver/info` — bootstrap discovery (feature 25). Answered directly at whatever
/// domain is queried. A backend always reports `is_resolver: false` with its own public base URL;
/// when a resolver fronts the global domain, the resolver answers this route instead (reporting
/// `is_resolver: true`). The frontend calls this once against the target domain before login/register
/// to learn where the heavier `/api/public/*` surface lives and whether a fleet dashboard exists.
pub async fn info(State(state): State<AppState>) -> Json<ResolverInfo> {
    Json(ResolverInfo {
        is_resolver: false,
        api_url: settings::public_base_url(&state.settings),
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

/// `GET /archypix-resolver/resolve?user=&domain=` — the backend answers this too (feature 25), so a
/// single-domain deployment whose backend domain differs from the global domain and runs no resolver
/// can forward `/archypix-resolver/` from the global domain to this backend and still resolve
/// identities in one hop. Confirms the user exists here (else `404`) and always returns **this
/// backend's own public URL** — same shape as the resolver's `resolve`. `404` on a domain mismatch.
pub async fn resolve(
    State(state): State<AppState>,
    Query(query): Query<ResolveQuery>,
) -> Result<Json<ResolveResponse>, AppError> {
    if query.domain != state.settings.get(keys::GLOBAL_DOMAIN) {
        return Err(AppError::NotFound);
    }
    UserRepository::find_by_username(&state.db, &query.user)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(ResolveResponse {
        backend_url: settings::public_base_url(&state.settings),
    }))
}
