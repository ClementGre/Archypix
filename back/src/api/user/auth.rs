use crate::api::middleware::auth_user::AuthUser;
use crate::repository::user::UserRepository;
use crate::services;
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

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

#[tracing::instrument(skip(state, payload), fields(user = %payload.username))]
pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<TokenResponse>, AppError> {
    let tokens = services::auth::login(
        &state.db,
        state.cache.as_ref(),
        &state.jwt,
        &state.settings,
        &payload.username,
        &payload.password,
    )
    .await?;
    Ok(Json(TokenResponse {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    }))
}

#[tracing::instrument(skip(state, payload))]
pub async fn refresh(
    State(state): State<AppState>,
    Json(payload): Json<RefreshRequest>,
) -> Result<Json<TokenResponse>, AppError> {
    let tokens = services::auth::refresh(
        &state.db,
        &state.jwt,
        state.settings.as_ref(),
        &payload.refresh_token,
    )
        .await?;
    Ok(Json(TokenResponse {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    }))
}

#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn logout(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<LogoutRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    services::auth::logout(&state.db, auth.claims.uid, payload.refresh_token.as_deref()).await?;
    Ok(Json(serde_json::json!({ "logged_out": true })))
}

#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn me(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user = UserRepository::find_by_id(&state.db, auth.user_id()?)
        .await?
        .ok_or(AppError::Unauthorized("User not found".to_string()))?;
    Ok(Json(serde_json::json!({
        "id": user.id,
        "username": user.username,
        "email": user.email,
        "display_name": user.display_name,
        "is_admin": user.is_admin,
        // Whether this instance sits behind a resolver — the frontend only offers the fleet dashboard then.
        "use_resolver": state.settings.get(crate::infra::settings::keys::USE_RESOLVER),
    })))
}
