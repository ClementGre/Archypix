use crate::api::middleware::auth_user::AuthUser;
use crate::domain::user_settings::{UserSettings, VersioningMode};
use crate::services;
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::extract::State;
use axum::Json;
use serde::Deserialize;
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

#[derive(Debug, Deserialize)]
pub struct UpdateSettingsBody {
    #[serde(default)]
    pub versioning_mode: Option<VersioningMode>,
    /// Retention window (days) before a soft-deleted owned picture is physically purged (09 §5.1).
    #[serde(default)]
    pub trash_retention_days: Option<i32>,
}

#[tracing::instrument(skip(auth, state, body), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn update_settings(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UpdateSettingsBody>,
) -> Result<Json<UserSettings>, AppError> {
    let settings = services::user_settings::update(
        &state.db,
        auth.user_id()?,
        body.versioning_mode,
        body.trash_retention_days,
    )
    .await?;
    Ok(Json(settings))
}
