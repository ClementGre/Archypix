mod auth;
mod hierarchies;
mod jobs;
mod pictures;
mod settings;
mod shares;
mod tagging_services;
mod tags;
mod users;

use crate::state::AppState;
use axum::Router;
use axum::routing::{delete, get, patch, post};

pub fn auth_routes() -> Router<AppState> {
    Router::new()
        .route("/login", post(auth::login))
        .route("/refresh", post(auth::refresh))
        .route("/logout", post(auth::logout))
        .route("/me", get(auth::me))
}

pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/register", post(users::register))
        .route("/users/{username}", get(users::get_public))
}

pub fn authenticated_routes() -> Router<AppState> {
    Router::new()
        .route("/users/me", patch(users::update_me))
        .route("/pictures/uploads", post(pictures::create_upload))
        .route(
            "/pictures/uploads/batch",
            post(pictures::batch_create_upload),
        )
        .route(
            "/pictures/uploads/{id}/complete",
            post(pictures::complete_upload),
        )
        .route("/pictures/pipeline/wake", post(pictures::wake_pipeline))
        .route("/pictures", get(pictures::list))
        .route("/pictures/{id}", get(pictures::details))
        .route("/pictures/{id}/url", get(pictures::picture_url))
        .route("/pictures/{id}/trash", post(pictures::trash))
        .route("/pictures/{id}/restore", post(pictures::restore))
        .route("/pictures/{id}/exif", post(pictures::edit_received_exif))
        .route("/settings", get(settings::get_settings))
        .route("/settings", patch(settings::update_settings))
        .route("/tags", get(tags::list).patch(tags::edit))
        .route(
            "/shares/outgoing",
            post(shares::create_outgoing).get(shares::list_outgoing),
        )
        .route(
            "/shares/outgoing/{id}/revoke",
            post(shares::revoke_outgoing),
        )
        .route("/shares/incoming", get(shares::list_incoming))
        .route(
            "/shares/incoming/{id}/accept",
            post(shares::accept_incoming),
        )
        .route(
            "/shares/incoming/{id}/reject",
            post(shares::reject_incoming),
        )
        .route("/jobs/{id}", get(jobs::get_job))
        .route("/pictures/{id}/jobs", get(jobs::list_picture_jobs))
        .route("/pictures/{id}/edit", post(jobs::enqueue_edit))
        .route("/pictures/exif", patch(jobs::batch_edit_exif))
        .route("/pictures/{id}/exif/resync", post(jobs::resync_exif))
        .route(
            "/tagging-services",
            get(tagging_services::list_services).post(tagging_services::create_service),
        )
        .route(
            "/tagging-services/reorder",
            post(tagging_services::reorder_services),
        )
        .route(
            "/tagging-services/{id}",
            get(tagging_services::get_service)
                .patch(tagging_services::update_service)
                .delete(tagging_services::delete_service),
        )
        .route(
            "/tagging-services/{id}/mappings",
            post(tagging_services::add_mapping),
        )
        .route(
            "/tagging-services/{id}/mappings/{rule_id}",
            delete(tagging_services::delete_mapping),
        )
        .route(
            "/tagging-services/{id}/rules",
            post(tagging_services::add_rule),
        )
        .route(
            "/tagging-services/{id}/rules/reorder",
            post(tagging_services::reorder_rules),
        )
        .route(
            "/tagging-services/{id}/rules/{rule_id}",
            patch(tagging_services::update_rule).delete(tagging_services::delete_rule),
        )
        .route(
            "/tagging-services/{id}/segments",
            post(tagging_services::add_segment),
        )
        .route(
            "/tagging-services/{id}/segments/{segment_id}",
            delete(tagging_services::delete_segment),
        )
        .route(
            "/hierarchies",
            get(hierarchies::list).post(hierarchies::create),
        )
        .route(
            "/hierarchies/{id}",
            get(hierarchies::get)
                .patch(hierarchies::update)
                .delete(hierarchies::delete),
        )
        .route("/hierarchies/{id}/tree", get(hierarchies::tree))
        .route("/hierarchies/{id}/browse", get(hierarchies::browse))
        .route(
            "/hierarchies/{id}/webdav",
            get(hierarchies::webdav_get).patch(hierarchies::webdav_patch),
        )
        .route(
            "/hierarchies/{id}/webdav/regenerate",
            post(hierarchies::webdav_regenerate),
        )
}
