use crate::api::middleware::auth_user::AuthUser;
use crate::infra::ratelimit;
use crate::infra::settings::keys;
use crate::repository::invite::InviteRepository;
use crate::repository::user::UserRepository;
use crate::services;
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::extract::{ConnectInfo, Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: Uuid,
    pub username: String,
    pub email: String,
    pub display_name: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub password: String,
    /// Invite code (required in invite/admin_invite mode; optional pinning in open mode).
    #[serde(default)]
    pub invite_code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMeRequest {
    pub display_name: Option<String>,
    pub email: Option<String>,
}

#[tracing::instrument(skip(state, payload, addr), fields(user = %payload.username))]
pub async fn register(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<RegisterRequest>,
) -> Result<Json<UserResponse>, AppError> {
    if state.settings.get(keys::USE_RESOLVER) {
        return Err(AppError::BadRequest(
            "Registration is handled by the resolver".to_string(),
        ));
    }
    // Throttle account-creation spam per source IP (07_security_audit.md §2.2).
    ratelimit::check_categorized(
        state.cache.as_ref(),
        ratelimit::category::REGISTER,
        &format!("register:{}", addr.ip()),
        state.settings.get(keys::RATE_LIMIT_REGISTER_MAX),
        state.settings.get(keys::RATE_LIMIT_REGISTER_WINDOW_SECS),
        state.settings.get(keys::RATE_LIMIT_EVENT_RETENTION_SECS),
    )
    .await?;

    // Registration mode + invite gate (feature 23 §6). Standalone-only path; the resolver enforces
    // this in resolver deployments.
    let invited_by = enforce_registration(&state, payload.invite_code.as_deref()).await?;

    let user = services::users::create_user(
        &state.db,
        &payload.username,
        &payload.email,
        &payload.display_name,
        &payload.password,
        false,
        Some(state.settings.get(keys::DEFAULT_STORAGE_QUOTA_BYTES)),
        invited_by.as_deref(),
    )
    .await?;
    Ok(Json(UserResponse {
        id: user.id,
        username: user.username,
        email: user.email,
        display_name: user.display_name,
    }))
}

/// Enforce the current registration mode and redeem an invite if required. Returns the inviter
/// username (the future `users.invited_by`), if any.
async fn enforce_registration(
    state: &AppState,
    invite_code: Option<&str>,
) -> Result<Option<String>, AppError> {
    let mode = state.settings.get(keys::REGISTRATION_MODE);
    let code = invite_code.map(str::trim).filter(|c| !c.is_empty());
    if mode.requires_invite() {
        let code = code.ok_or_else(|| {
            AppError::BadRequest("an invite code is required to register".to_string())
        })?;
        match InviteRepository::redeem(&state.db, code).await? {
            // A tracking referral link is not a real invite in a gated mode (feature 23 §6).
            Some(inv) if inv.is_tracking() => Err(AppError::BadRequest(
                "the invite code is invalid, expired, or has no remaining uses".to_string(),
            )),
            Some(inv) => Ok(Some(inv.created_by)),
            None => Err(AppError::BadRequest(
                "the invite code is invalid, expired, or has no remaining uses".to_string(),
            )),
        }
    } else if let Some(code) = code {
        // Open mode: honour a valid invite for provenance, silently ignore an invalid one.
        Ok(InviteRepository::redeem(&state.db, code)
            .await?
            .map(|inv| inv.created_by))
    } else {
        Ok(None)
    }
}

#[tracing::instrument(skip(state), fields(user = %username))]
pub async fn get_public(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<UserResponse>, AppError> {
    let user = UserRepository::find_by_username(&state.db, &username)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(UserResponse {
        id: user.id,
        username: user.username,
        email: user.email,
        display_name: user.display_name,
    }))
}

/// `GET /api/authenticated/me/storage` — the caller's storage quota, usage, and breakdown
/// (feature 22 §8.1). Drives the footer bar, settings breakdown, and upload preflight.
#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn get_storage(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<services::storage::StorageInfo>, AppError> {
    let info = services::storage::storage_info(&state.db, &state.settings, auth.user_id()?).await?;
    Ok(Json(info))
}

#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn update_me(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<UpdateMeRequest>,
) -> Result<Json<UserResponse>, AppError> {
    if let Some(email) = payload.email.as_deref() {
        crate::domain::validation::validate_email(email).map_err(AppError::BadRequest)?;
    }
    let user = UserRepository::update_profile(
        &state.db,
        auth.user_id()?,
        payload.display_name.as_deref(),
        payload.email.as_deref(),
    )
    .await?;
    Ok(Json(UserResponse {
        id: user.id,
        username: user.username,
        email: user.email,
        display_name: user.display_name,
    }))
}
