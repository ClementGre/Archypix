mod handlers;
mod models;

use crate::state::AppState;
use axum::Router;
use axum::routing::{get, post};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/jobs/next", get(handlers::claim_next_job))
        .route("/jobs/{id}/respond", post(handlers::respond_job))
}
