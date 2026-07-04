use crate::api::middleware::auth_user::AuthUser;
use crate::infra::error::AppError;
use crate::infra::ratelimit;
use crate::repository::user::UserRepository;
use crate::services;
use crate::state::AppState;
use axum::Json;
use axum::extract::{ConnectInfo, Path, State};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: uuid::Uuid,
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
    if state.config.use_resolver {
        return Err(AppError::BadRequest(
            "Registration is handled by the resolver".to_string(),
        ));
    }
    // Throttle account-creation spam per source IP (07_security_audit.md §2.2).
    ratelimit::check(
        state.cache.as_ref(),
        &format!("register:{}", addr.ip()),
        state.config.rate_limit_register_max,
        state.config.rate_limit_register_window_secs,
    )
    .await?;
    let user = services::users::create_user(
        &state.db,
        &payload.username,
        &payload.email,
        &payload.display_name,
        &payload.password,
        false,
        Some(state.config.default_storage_quota_bytes),
    )
    .await?;
    Ok(Json(UserResponse {
        id: user.id,
        username: user.username,
        email: user.email,
        display_name: user.display_name,
    }))
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
    let info = services::storage::storage_info(&state.db, &state.config, auth.user_id()?).await?;
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
