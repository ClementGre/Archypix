mod handlers;

use crate::state::AppState;
use axum::Router;
use axum::routing::post;

pub fn routes() -> Router<AppState> {
    // Feature 28 §5: the eight authenticated verb routes collapse to one typed, versioned envelope.
    // `auth/request`, `auth/grant`, and `pictures/presign` stay dedicated (they bootstrap the token
    // or use the token-gated presign auth model).
    Router::new()
        .route("/auth/request", post(handlers::auth_request))
        .route("/auth/grant", post(handlers::auth_grant))
        .route("/message", post(handlers::message))
        .route("/pictures/presign", post(handlers::presign_pictures))
}
