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
use tracing::debug;

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

pub async fn register(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<RegisterRequest>,
) -> Result<Json<UserResponse>, AppError> {
    debug!(user = %payload.username, token_type = "-", "register");
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
    )
    .await?;
    Ok(Json(UserResponse {
        id: user.id,
        username: user.username,
        email: user.email,
        display_name: user.display_name,
    }))
}

pub async fn get_public(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<UserResponse>, AppError> {
    debug!(user = %username, token_type = "-", "get_public");
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

pub async fn update_me(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<UpdateMeRequest>,
) -> Result<Json<UserResponse>, AppError> {
    debug!(user = %auth.claims.sub, token_type = auth.token_type(), "update_me");
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
