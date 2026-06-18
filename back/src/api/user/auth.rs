use crate::api::middleware::auth_user::AuthUser;
use crate::infra::error::AppError;
use crate::repository::user::UserRepository;
use crate::services;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use tracing::debug;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize)]
pub struct LogoutRequest {
    pub refresh_token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
}

pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<TokenResponse>, AppError> {
    debug!(user = %payload.username, token_type = "-", "login");
    let tokens = services::auth::login(
        &state.db,
        state.cache.as_ref(),
        &state.jwt,
        &state.config,
        &payload.username,
        &payload.password,
    )
    .await?;
    Ok(Json(TokenResponse {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    }))
}

pub async fn refresh(
    State(state): State<AppState>,
    Json(payload): Json<RefreshRequest>,
) -> Result<Json<TokenResponse>, AppError> {
    debug!(user = "-", token_type = "-", "token refresh");
    let tokens =
        services::auth::refresh(&state.db, &state.jwt, &state.config, &payload.refresh_token)
            .await?;
    Ok(Json(TokenResponse {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    }))
}

pub async fn logout(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<LogoutRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    debug!(user = %auth.claims.sub, token_type = auth.token_type(), "logout");
    services::auth::logout(&state.db, auth.claims.uid, payload.refresh_token.as_deref()).await?;
    Ok(Json(serde_json::json!({ "logged_out": true })))
}

pub async fn me(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    debug!(user = %auth.claims.sub, token_type = auth.token_type(), "me");
    let user = UserRepository::find_by_id(&state.db, auth.user_id()?)
        .await?
        .ok_or(AppError::Unauthorized("User not found".to_string()))?;
    Ok(Json(serde_json::json!({
        "id": user.id,
        "username": user.username,
        "email": user.email,
        "display_name": user.display_name,
        "is_admin": user.is_admin,
    })))
}
