mod auth;
mod hierarchies;
pub mod invites;
mod jobs;
mod pictures;
mod public_shares;
mod public_view;
mod settings;
mod shares;
mod tagging_services;
mod tags;
mod users;

use crate::state::AppState;
use axum::Router;
use axum::routing::{get, patch, post, put};

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
        .route("/invites/{code}", get(invites::preview_invite))
        .route("/registration-info", get(invites::registration_info))
        // Public shares (feature 27): token-gated view + anonymous contribution.
        .route("/shares/{token}", get(public_view::meta))
        .route("/shares/{token}/unlock", post(public_view::unlock))
        .route("/shares/{token}/pictures", get(public_view::pictures))
        .route(
            "/shares/{token}/pictures/{pid}",
            get(public_view::picture_detail),
        )
        .route(
            "/shares/{token}/pictures/{pid}/url",
            get(public_view::picture_url),
        )
        .route("/shares/{token}/aggregate", post(public_view::aggregate))
        .route("/shares/{token}/uploads", post(public_view::uploads))
        .route(
            "/shares/{token}/uploads/{pid}/complete",
            post(public_view::complete_upload),
        )
}

pub fn authenticated_routes() -> Router<AppState> {
    Router::new()
        .route("/users/me", patch(users::update_me))
        .route("/me/storage", get(users::get_storage))
        // Invites + invitation graph (feature 23 §6)
        .route(
            "/invites",
            post(invites::mint_invite).get(invites::list_invites),
        )
        .route(
            "/invites/{code}",
            axum::routing::delete(invites::revoke_invite),
        )
        .route("/me/invitations", get(invites::my_invitations))
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
        .route("/pictures/aggregate", post(pictures::aggregate))
        .route("/pictures/trash", post(pictures::batch_trash))
        .route("/pictures/restore", post(pictures::batch_restore))
        .route("/pictures/{id}", get(pictures::details))
        .route("/pictures/{id}/url", get(pictures::picture_url))
        .route("/pictures/{id}/trash", post(pictures::trash))
        .route("/pictures/{id}/restore", post(pictures::restore))
        .route("/pictures/{id}/copy", post(pictures::copy))
        .route("/pictures/{id}/copies", get(pictures::copies))
        .route("/pictures/{id}/copies/keep", post(pictures::keep_copy))
        .route("/pictures/{id}/exif", post(pictures::edit_received_exif))
        .route("/pictures/{id}/creator", post(pictures::set_creator))
        .route("/pictures/creator", patch(pictures::batch_set_creator))
        .route("/settings", get(settings::get_settings))
        .route("/settings", patch(settings::update_settings))
        .route("/tags", get(tags::list).patch(tags::edit))
        .route("/tags/rename", post(tags::rename))
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
        // Public shares (feature 27): owner management + logged-in visitor Convert.
        .route(
            "/shares/public",
            post(public_shares::create).get(public_shares::list),
        )
        .route("/shares/public/save-copy", post(public_shares::save_copy))
        .route("/shares/public/subscribe", post(public_shares::subscribe))
        .route("/shares/public/{id}", patch(public_shares::update))
        .route("/shares/public/{id}/revoke", post(public_shares::revoke))
        .route(
            "/shares/public/{id}/contributions/trash",
            post(public_shares::trash_contributions),
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
            "/tagging-services/{id}/config",
            put(tagging_services::replace_config),
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
