//! Router assembly, mirroring the backend's `api.rs`.

pub mod admin;
pub mod backends;
pub mod bootstrap;
pub mod middleware;
pub mod public;

use crate::state::AppState;
use axum::response::IntoResponse;
use axum::routing::{any, delete, get, patch, post};
use axum::{Json, Router};
use tower_http::cors::{Any, CorsLayer};

pub fn routes(dynamic_cors: CorsLayer) -> Router<AppState> {
    // The resolver's entire surface is nested under one top-level prefix (feature 25) so a self-hoster
    // has a single forwarding rule and no `.well-known` collision. Handler paths are unchanged inside.
    Router::new()
        .route("/health", get(health))
        .nest("/archypix-resolver", prefixed_routes(dynamic_cors))
}

fn prefixed_routes(dynamic_cors: CorsLayer) -> Router<AppState> {
    // Bootstrap + federation resolution are hit cross-origin by arbitrary frontends resolving
    // `@user:domain`, so they answer any origin regardless of the CORS_ORIGINS setting (feature 25).
    let open = Router::new()
        .route("/info", get(bootstrap::info))
        .route("/resolve", get(bootstrap::resolve))
        .layer(open_cors());

    // The heavier register/admin/backend surface stays behind the operator's dynamic CORS.
    let gated = Router::new()
        .route("/api/public/register", post(public::register))
        .route("/api/public/invites/{code}", get(public::preview_invite))
        .route(
            "/api/public/registration-info",
            get(public::registration_info),
        )
        .route("/api/update", post(backends::update_mapping))
        .route(
            "/api/backends",
            post(backends::self_register).get(backends::list_backends),
        )
        .route("/api/backends/heartbeat", post(backends::heartbeat))
        // Backend-driven invites (resolver mode): a user mints on their backend, pushed up here.
        .route(
            "/api/backends/invites",
            post(backends::create_invite).get(backends::list_invites),
        )
        .route(
            "/api/backends/invites/{code}",
            delete(backends::delete_invite),
        )
        .nest("/api/resolver-admin", admin_routes())
        .route("/health", get(health))
        .layer(dynamic_cors);

    open.merge(gated)
}

/// Open CORS for the bootstrap discovery + resolution routes (feature 25): any origin, independent of
/// the `cors_origins` setting. Read-only, unauthenticated, non-credentialed — safe to open to `*`.
fn open_cors() -> CorsLayer {
    CorsLayer::new()
        .allow_methods(Any)
        .allow_origin(Any)
        .allow_headers(Any)
}

fn admin_routes() -> Router<AppState> {
    Router::new()
        .route("/login", post(admin::login))
        .route("/refresh", post(admin::refresh))
        .route("/overview", get(admin::overview))
        .route("/backends", get(admin::backends))
        .route("/next-backend", get(admin::next_backend))
        .route(
            "/backends/{back_domain}/capacity",
            patch(admin::set_capacity),
        )
        .route(
            "/settings",
            get(admin::get_settings).patch(admin::patch_setting),
        )
        .route("/settings/{key}", delete(admin::reset_setting))
        .route("/routines", get(admin::get_routines))
        .route("/routines/{name}/trigger", post(admin::trigger_routine))
        .route(
            "/invites",
            get(admin::list_invites).post(admin::mint_invite),
        )
        .route("/invites/{code}", delete(admin::revoke_invite))
        .route(
            "/config-matrix",
            get(admin::config_matrix).patch(admin::config_matrix_patch),
        )
        // Per-instance thin proxy to each backend's /api/admin/* (delegation replay).
        .route(
            "/instances/{back_domain}/api/admin/{*path}",
            any(admin::proxy),
        )
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "healthy", "service": "archypix-resolver" }))
}
