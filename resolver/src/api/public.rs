//! Public registration (feature 23 §6–7). The resolver enforces the mode + invite, picks a backend by
//! the configured strategy (honouring an invite's `instance_pin`), forwards the signup to that backend
//! (replaying its delegation token), and records the `username → back_domain` mapping.

use crate::repository;
use crate::services::{registration, selection};
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::Json;
use axum::extract::{Path, State};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::debug;

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub display_name: String,
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub invite_code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub username: String,
    pub backend_url: String,
    pub message: String,
}

pub async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, AppError> {
    if payload.username.is_empty() || payload.email.is_empty() {
        return Err(AppError::BadRequest(
            "username and email are required".to_string(),
        ));
    }
    if repository::username_exists(&state.db, &payload.username).await? {
        return Err(AppError::BadRequest(format!(
            "Username '{}' is already taken",
            payload.username
        )));
    }

    // Mode + invite gate (atomic redemption). Yields an instance_pin + invited_by, if any.
    let authorized =
        registration::authorize(&state.db, &state.config, payload.invite_code.as_deref()).await?;

    // Placement: honour the pin per pin_importance; capacity/reachability are hard gates.
    let backend =
        selection::pick_backend(&state.db, &state.config, authorized.instance_pin()).await?;

    // Provision on the chosen backend (delegation replay). The backend accepts every forwarded signup.
    let body = serde_json::json!({
        "username": payload.username,
        "display_name": payload.display_name,
        "email": payload.email,
        "password": payload.password,
        "invited_by": authorized.invited_by(),
    });
    state
        .backends
        .register_user(&backend.back_domain, &body)
        .await?;

    repository::upsert_mapping(&state.db, &payload.username, &backend.back_domain).await?;

    debug!(user = %payload.username, back_domain = %backend.back_domain, "registered");
    Ok(Json(RegisterResponse {
        username: payload.username,
        backend_url: backend.public_url(),
        message: "User registered successfully".to_string(),
    }))
}

#[derive(Debug, Serialize)]
pub struct RegistrationInfo {
    pub mode: archypix_common::registration::RegistrationMode,
}

/// `GET /api/public/registration-info` — the effective mode the resolver enforces (feature 23 §6).
pub async fn registration_info(State(state): State<AppState>) -> Json<RegistrationInfo> {
    Json(RegistrationInfo {
        mode: state.config.get(crate::config::setting_keys::REGISTRATION_MODE),
    })
}

#[derive(Debug, Serialize)]
pub struct InvitePreview {
    pub valid: bool,
    pub invited_by: Option<String>,
}

/// `GET /api/public/invites/{code}` — unauthenticated preview so the register page can show
/// "X invited you to join …" (feature 23 §6.3). Mirrors the standalone backend's path.
pub async fn preview_invite(
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> Result<Json<InvitePreview>, AppError> {
    let invite = repository::get_invite(&state.db, &code).await?;
    // A tracking referral is inactive in gated modes — reflect that in `valid` so the register page
    // treats it as invalid (feature 24 fix).
    let mode = state.config.get(crate::config::setting_keys::REGISTRATION_MODE);
    Ok(Json(match invite {
        Some(i) => InvitePreview {
            valid: i.is_active(mode, Utc::now()),
            invited_by: Some(i.created_by),
        },
        None => InvitePreview {
            valid: false,
            invited_by: None,
        },
    }))
}
