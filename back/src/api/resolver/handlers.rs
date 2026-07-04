use crate::api::middleware::auth_resolver::AuthResolver;
use crate::api::resolver::models::{CreateUserRequest, UserResponse};
use crate::infra::error::AppError;
use crate::repository::user::UserRepository;
use crate::services;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};

#[tracing::instrument(skip(auth, state), fields(user = %username, requester = %auth.claims.sub))]
pub async fn get_user(
    auth: AuthResolver,
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<UserResponse>, AppError> {
    let user = UserRepository::find_by_username(&state.db, &username)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(UserResponse::from(user)))
}

#[tracing::instrument(skip(auth, state, payload), fields(user = %payload.username, requester = %auth.claims.sub))]
pub async fn create_user(
    auth: AuthResolver,
    State(state): State<AppState>,
    Json(payload): Json<CreateUserRequest>,
) -> Result<Json<UserResponse>, AppError> {
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
    Ok(Json(UserResponse::from(user)))
}
