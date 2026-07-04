mod handlers;
mod models;

use crate::state::AppState;
use axum::Router;
use axum::routing::{get, patch, post};

pub fn routes() -> Router<AppState> {
    Router::new()
        // ── Instance ──────────────────────────────────────────────────────────
        .route("/instance", get(handlers::get_instance))
        // ── Instance-wide analytics (cached, 60 s TTL) ────────────────────────
        .route("/stats", get(handlers::get_instance_stats))
        // ── Consistency check ─────────────────────────────────────────────────
        .route("/consistency", get(handlers::get_consistency))
        // ── User management ───────────────────────────────────────────────────
        .route(
            "/users",
            get(handlers::list_users).post(handlers::create_user),
        )
        .route(
            "/users/{id}",
            patch(handlers::update_user).delete(handlers::delete_user),
        )
        // ── Per-user analytics (cached, 120 s TTL) ────────────────────────────
        .route("/users/{id}/stats", get(handlers::get_user_stats))
        // ── Per-user S3 storage audit (cached) ────────────────────────────────
        .route("/users/{id}/storage-audit", get(handlers::storage_audit))
        // ── Per-user shares ───────────────────────────────────────────────────
        .route("/users/{id}/shares", get(handlers::get_user_shares))
        // ── Per-user pipeline wake ────────────────────────────────────────────
        .route(
            "/users/{id}/pipeline/wake",
            post(handlers::wake_user_pipeline),
        )
        // ── Job management ────────────────────────────────────────────────────
        .route("/jobs", get(handlers::list_jobs))
        .route("/jobs/stale", get(handlers::list_stale_jobs))
        .route("/jobs/{id}/reset", post(handlers::reset_job))
        .route("/jobs/{id}/cancel", post(handlers::cancel_job))
        // ── Bulk thumbnail / content-hash regeneration ────────────────────────
        .route(
            "/pictures/regenerate-thumbnails",
            post(handlers::regenerate_thumbnails),
        )
        // ── Share management ──────────────────────────────────────────────────
        .route("/shares/errored", get(handlers::list_errored_shares))
        .route(
            "/shares/outgoing/{id}/force-reconcile",
            post(handlers::force_reconcile_share),
        )
        // ── Federation ────────────────────────────────────────────────────────
        .route(
            "/federation/instances",
            get(handlers::list_federation_instances),
        )
        .route(
            "/federation/connections",
            get(handlers::list_active_federation_connections),
        )
}
