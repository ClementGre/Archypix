use crate::api::middleware::auth_user::AuthUser;
use crate::domain::user_settings::{UserSettings, VersioningMode};
use crate::infra::error::AppError;
use crate::services;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use tracing::debug;

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

pub async fn update_settings(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UpdateSettingsBody>,
) -> Result<Json<UserSettings>, AppError> {
    debug!(user = %auth.claims.sub, token_type = auth.token_type(), versioning_mode = ?body.versioning_mode, "update_settings");
    let settings = services::user_settings::update(
        &state.db,
        auth.user_id()?,
        body.versioning_mode,
        body.trash_retention_days,
    )
    .await?;
    Ok(Json(settings))
}
