mod handlers;

use crate::state::AppState;
use axum::Router;
use axum::routing::post;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/request", post(handlers::auth_request))
        .route("/auth/grant", post(handlers::auth_grant))
        .route("/shares/announce", post(handlers::announce_share))
        .route("/shares/accept", post(handlers::accept_share))
        .route("/shares/reject", post(handlers::reject_share))
        .route("/shares/revoke", post(handlers::revoke_share))
        .route("/pictures/announce", post(handlers::announce_pictures))
        .route("/pictures/unannounce", post(handlers::unannounce_pictures))
        .route(
            "/pictures/edit_request",
            post(handlers::edit_picture_request),
        )
        .route("/pictures/presign", post(handlers::presign_pictures))
}
