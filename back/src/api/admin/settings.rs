//! Admin runtime-settings endpoints (feature 23 §4.5). `GET` returns every field with its value,
//! provenance, lock/restart flags, and docs; `PATCH`/`DELETE` write or clear a DB override (env-locked
//! fields are rejected) and hot-swap the live snapshot so the next request sees the change.

use crate::api::middleware::auth_admin::AuthAdmin;
use crate::repository::app_settings::AppSettingsRepository;
use crate::state::AppState;
use archypix_common::error::AppError;
use archypix_common::settings::FieldMeta;
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct PatchSettingRequest {
    pub key: String,
    pub value: Value,
}

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub))]
pub async fn get_settings(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<FieldMeta>>, AppError> {
    Ok(Json(state.settings.field_meta()))
}

#[tracing::instrument(skip(_auth, state, payload), fields(user = %_auth.claims.sub, setting = %payload.key))]
pub async fn patch_setting(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<PatchSettingRequest>,
) -> Result<Json<Vec<FieldMeta>>, AppError> {
    // Reject unknown/locked/mistyped values, and normalise into the canonical stored form.
    let coerced = state
        .settings
        .validate_override_str(&payload.key, &payload.value)?;
    AppSettingsRepository::upsert(&state.db, &payload.key, &coerced).await?;
    reload(&state).await?;
    Ok(Json(state.settings.field_meta()))
}

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub, setting = %key))]
pub async fn reset_setting(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<Vec<FieldMeta>>, AppError> {
    // A locked field has no DB override to clear; report the conflict rather than silently no-op.
    if state.settings.is_locked_str(&key) {
        return Err(AppError::Conflict(format!(
            "setting '{key}' is defined by an environment variable and cannot be changed"
        )));
    }
    AppSettingsRepository::delete(&state.db, &key).await?;
    reload(&state).await?;
    Ok(Json(state.settings.field_meta()))
}

/// Rebuild the live snapshot from the current DB overrides after a write.
async fn reload(state: &AppState) -> Result<(), AppError> {
    let overrides = AppSettingsRepository::load_all(&state.db).await?;
    state.settings.reload(&overrides).map_err(|e| {
        AppError::InternalServerError(format!("failed to reload runtime settings: {e}"))
    })
}

// ── Routines tab (feature 23 §5.2 refinement) ───────────────────────────────────

#[derive(serde::Serialize)]
pub struct RoutineInfo {
    name: &'static str,
    last_started_at: Option<i64>,
    last_finished_at: Option<i64>,
    last_error: Option<String>,
    in_flight: usize,
    total_runs: u64,
    /// The settings fields that tune this routine (value + docs + env name), for inline editing.
    settings: Vec<FieldMeta>,
}

/// List all spawned routines with live status + their tuning settings (for the admin Routines tab).
#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub))]
pub async fn get_routines(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<RoutineInfo>>, AppError> {
    let meta = state.settings.field_meta();
    let infos = state
        .routine_registry
        .entries
        .iter()
        .map(|e| {
            let s = e.status.snapshot();
            let settings = meta
                .iter()
                .filter(|m| m.routine.as_deref() == Some(e.name))
                .cloned()
                .collect();
            RoutineInfo {
                name: e.name,
                last_started_at: s.last_started_at,
                last_finished_at: s.last_finished_at,
                last_error: s.last_error,
                in_flight: s.in_flight,
                total_runs: s.total_runs,
                settings,
            }
        })
        .collect();
    Ok(Json(infos))
}

/// Manually trigger a routine with its `Input::default()` (feature 23 refinement).
#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub, routine = %name))]
pub async fn trigger_routine(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, AppError> {
    let entry = state
        .routine_registry
        .entries
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| AppError::BadRequest(format!("unknown routine '{name}'")))?;
    entry.trigger.trigger_default();
    Ok(Json(
        serde_json::json!({ "triggered": true, "routine": name }),
    ))
}
