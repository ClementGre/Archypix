mod admin;
mod bootstrap;
mod federation;
mod middleware;
mod resolver;
mod user;
mod webdav;
mod worker;

use crate::infra::settings::keys;
use crate::state::AppState;
use archypix_common::settings::Settings;
use axum::http::HeaderValue;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

pub fn routes(settings: Arc<Settings>) -> Router<AppState> {
    // Dynamic CORS (feature 23 §4.4): the allowed origins are read from the live settings snapshot on
    // every request, so an operator can change them from the dashboard without a restart.
    let cors = CorsLayer::new()
        .allow_methods(Any)
        .allow_origin(dynamic_allow_origin(settings.clone()))
        .allow_headers([AUTHORIZATION, CONTENT_TYPE]);

    let mut router = Router::new()
        .nest("/api", api_routes().layer(cors.clone()))
        // WebDAV lives outside /api — clients authenticate with HTTP Basic, not a User JWT.
        .merge(webdav::routes())
        .route("/health", get(health));

    if !settings.get(keys::USE_RESOLVER) {
        // Bootstrap discovery + resolution (feature 25): fixed, CORS-open paths answered directly at
        // this domain. `info` reports a non-resolver serving its own public API; `resolve` lets a
        // single-domain deployment (global domain forwarding `/archypix-resolver/` here) resolve
        // identities without a resolver, returning this backend's own public URL.
        router
            .route(
                "/archypix-resolver/info",
                get(bootstrap::info).layer(cors.clone()),
            )
            .route(
                "/archypix-resolver/resolve",
                get(bootstrap::resolve).layer(cors),
            )
    } else {
        router
    }
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "archypix-back"
    }))
}

fn api_routes() -> Router<AppState> {
    Router::new()
        .nest("/resolver", resolver::routes())
        .nest("/admin", admin::routes())
        .nest("/auth", user::auth_routes())
        .nest("/public", user::public_routes())
        .nest("/authenticated", user::authenticated_routes())
        .nest("/federation", federation::routes())
        .nest("/worker", worker::routes())
}

/// An `AllowOrigin` predicate that consults the live `cors_origins` setting per request. `*` allows
/// (echoes) any origin; otherwise the origin must be in the list.
fn dynamic_allow_origin(settings: Arc<Settings>) -> AllowOrigin {
    AllowOrigin::predicate(move |origin: &HeaderValue, _parts| {
        let allowed = settings.get(keys::CORS_ORIGINS);
        if allowed.iter().any(|o| o == "*") {
            return true;
        }
        origin
            .to_str()
            .map(|o| allowed.iter().any(|a| a == o))
            .unwrap_or(false)
    })
}
