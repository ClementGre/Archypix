use crate::api::middleware::auth_user::AuthUser;
use crate::domain::user_settings::{UserSettings, UserSettingsPatch};
use crate::services;
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::extract::State;
use axum::Json;
use tracing::debug;

#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn get_settings(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<UserSettings>, AppError> {
    debug!(user = %auth.claims.sub, token_type = auth.token_type(), "get_settings");
    let settings = services::user_settings::get(&state.db, auth.user_id()?).await?;
    Ok(Json(settings))
}

#[tracing::instrument(skip(auth, state, body), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn update_settings(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UserSettingsPatch>,
) -> Result<Json<UserSettings>, AppError> {
    let settings = services::user_settings::update(&state.db, auth.user_id()?, &body).await?;
    Ok(Json(settings))
}
