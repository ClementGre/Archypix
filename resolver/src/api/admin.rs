//! Fleet admin dashboard — native aggregate/self-monitoring endpoints + a per-instance proxy +
//! config-matrix fan-out (feature 23 §5). Guarded by [`AuthAdmin`] (operator session), except
//! `login`/`refresh` which are the entry points.

use crate::api::middleware::AuthAdmin;
use crate::config;
use crate::repository;
use crate::services::operator;
use crate::state::AppState;
use archypix_common::error::AppError;
use archypix_common::registration::Invite;
use axum::Json;
use axum::extract::{Path, State};
use chrono::{Duration, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Operator auth ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LoginRequest {
    pub token: String,
}
#[derive(Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}
#[derive(Serialize)]
pub struct SessionResponse {
    pub session_token: String,
    pub refresh_token: String,
    pub expires_in_secs: i64,
}

fn session_json(s: operator::Session) -> Json<SessionResponse> {
    Json(SessionResponse {
        session_token: s.session_token,
        refresh_token: s.refresh_token,
        expires_in_secs: s.expires_in_secs,
    })
}

pub async fn login(
    State(state): State<AppState>,
    Json(p): Json<LoginRequest>,
) -> Result<Json<SessionResponse>, AppError> {
    Ok(session_json(
        operator::login(&state.db, &state.jwt, &state.global_domain(), &p.token).await?,
    ))
}

pub async fn refresh(
    State(state): State<AppState>,
    Json(p): Json<RefreshRequest>,
) -> Result<Json<SessionResponse>, AppError> {
    Ok(session_json(
        operator::refresh(
            &state.db,
            &state.jwt,
            &state.global_domain(),
            &p.refresh_token,
        )
            .await?,
    ))
}

// ── Aggregate monitoring ─────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct Overview {
    pub total_users: i64,
    pub total_pictures: i64,
    pub total_storage_bytes: i64,
    pub backend_count: usize,
    pub reachable_count: usize,
    pub backends: Vec<repository::Backend>,
}

pub async fn overview(
    _a: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Overview>, AppError> {
    let backends = repository::list_backends(&state.db).await?;
    let (u, p, s) = repository::fleet_totals(&state.db).await?;
    let reachable = backends.iter().filter(|b| b.reachable).count();
    Ok(Json(Overview {
        total_users: u,
        total_pictures: p,
        total_storage_bytes: s,
        backend_count: backends.len(),
        reachable_count: reachable,
        backends,
    }))
}

pub async fn backends(
    _a: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<repository::Backend>>, AppError> {
    Ok(Json(repository::list_backends(&state.db).await?))
}

/// Dry-run the placement strategy for the *next* (un-pinned) signup so the dashboard can show where a
/// new user would land. `None` when no backend is eligible (all full/closed/unreachable).
pub async fn next_backend(
    _a: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let next = crate::services::selection::pick_backend(&state.db, &state.config, None)
        .await
        .ok()
        .map(|b| b.back_domain);
    Ok(Json(serde_json::json!({ "back_domain": next })))
}

#[derive(Deserialize)]
pub struct CapacityRequest {
    pub accepting_registrations: bool,
    pub max_users: Option<i64>,
}
pub async fn set_capacity(
    _a: AuthAdmin,
    State(state): State<AppState>,
    Path(back_domain): Path<String>,
    Json(p): Json<CapacityRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    repository::set_capacity(
        &state.db,
        &back_domain,
        p.accepting_registrations,
        p.max_users,
    )
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ── Resolver's own settings ──────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct PatchSettingRequest {
    pub key: String,
    pub value: Value,
}

pub async fn get_settings(
    _a: AuthAdmin,
    State(state): State<AppState>,
) -> Json<Vec<archypix_common::settings::FieldMeta>> {
    Json(state.config.field_meta())
}

pub async fn patch_setting(
    _a: AuthAdmin,
    State(state): State<AppState>,
    Json(p): Json<PatchSettingRequest>,
) -> Result<Json<Vec<archypix_common::settings::FieldMeta>>, AppError> {
    let coerced = state.config.validate_override_str(&p.key, &p.value)?;
    repository::upsert_setting(&state.db, &p.key, &coerced).await?;
    config::reload_from_db(&state.config, &state.db)
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;
    Ok(Json(state.config.field_meta()))
}

pub async fn reset_setting(
    _a: AuthAdmin,
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<Vec<archypix_common::settings::FieldMeta>>, AppError> {
    if state.config.is_locked_str(&key) {
        return Err(AppError::Conflict(format!(
            "setting '{key}' is defined by an environment variable"
        )));
    }
    repository::delete_setting(&state.db, &key).await?;
    config::reload_from_db(&state.config, &state.db)
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;
    Ok(Json(state.config.field_meta()))
}

// ── Invites ──────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct MintInviteRequest {
    pub max_uses: Option<i64>,
    pub expires_in_days: Option<i64>,
    pub instance_pin: Option<String>,
}
#[derive(Serialize)]
pub struct InviteResponse {
    pub code: String,
    pub max_uses: Option<i64>,
    pub uses: i64,
    pub expires_at: Option<chrono::DateTime<Utc>>,
    pub created_by: String,
    pub instance_pin: Option<String>,
}
impl From<Invite> for InviteResponse {
    fn from(i: Invite) -> Self {
        Self {
            code: i.code,
            max_uses: i.max_uses,
            uses: i.uses,
            expires_at: i.expires_at,
            created_by: i.created_by,
            instance_pin: i.instance_pin,
        }
    }
}

pub async fn list_invites(
    _a: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<InviteResponse>>, AppError> {
    Ok(Json(
        repository::list_invites(&state.db)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}

pub async fn mint_invite(
    _a: AuthAdmin,
    State(state): State<AppState>,
    Json(p): Json<MintInviteRequest>,
) -> Result<Json<InviteResponse>, AppError> {
    let code = archypix_common::registration::generate_invite_code();
    let expires_at = p
        .expires_in_days
        .filter(|d| *d > 0)
        .map(|d| Utc::now() + Duration::days(d));
    let inv = repository::create_invite(
        &state.db,
        &code,
        p.max_uses,
        expires_at,
        "operator",
        p.instance_pin.as_deref(),
    )
        .await?;
    Ok(Json(inv.into()))
}

pub async fn revoke_invite(
    _a: AuthAdmin,
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> Result<Json<Value>, AppError> {
    repository::delete_invite(&state.db, &code).await?;
    Ok(Json(serde_json::json!({ "revoked": true })))
}

// ── Routines (feature 24) ──────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct RoutineInfo {
    name: &'static str,
    last_started_at: Option<i64>,
    last_finished_at: Option<i64>,
    last_error: Option<String>,
    in_flight: usize,
    total_runs: u64,
    settings: Vec<archypix_common::settings::FieldMeta>,
}

pub async fn get_routines(
    _a: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<RoutineInfo>>, AppError> {
    let meta = state.config.field_meta();
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

pub async fn trigger_routine(
    _a: AuthAdmin,
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
    Ok(Json(serde_json::json!({ "triggered": true, "routine": name })))
}

// ── Per-instance proxy + config-matrix ───────────────────────────────────────────

/// `ANY /instances/{back_domain}/api/admin/{*path}` → replay the delegation token to the backend's
/// `/api/admin/*` and return its response (feature 23 §5.3). JSON in/out.
pub async fn proxy(
    _a: AuthAdmin,
    State(state): State<AppState>,
    Path((back_domain, path)): Path<(String, String)>,
    method: Method,
    body: Option<Json<Value>>,
) -> Result<Json<Value>, AppError> {
    let subpath = format!("/api/admin/{path}");
    let (_status, out) = state
        .backends
        .proxy_json(&back_domain, method, &subpath, body.map(|b| b.0))
        .await?;
    Ok(Json(out))
}

/// Fan out `GET /api/admin/settings` to every reachable backend and return, per field, the distinct
/// values across backends (highlighting divergence + version/field drift) (feature 23 §5.4).
pub async fn config_matrix(
    _a: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Value>, AppError> {
    let backends = repository::list_backends(&state.db).await?;
    let mut per_backend = serde_json::Map::new();
    for b in backends.into_iter().filter(|b| b.reachable) {
        match state
            .backends
            .get_json(&b.back_domain, "/api/admin/settings")
            .await
        {
            Ok(v) => {
                per_backend.insert(b.back_domain, v);
            }
            Err(e) => {
                per_backend.insert(b.back_domain, serde_json::json!({ "error": e.to_string() }));
            }
        }
    }
    Ok(Json(Value::Object(per_backend)))
}

#[derive(Deserialize)]
pub struct ConfigMatrixPatch {
    pub key: String,
    pub value: Value,
    /// `"all"` (default) or an explicit list of `back_domain`s.
    #[serde(default)]
    pub targets: Option<Vec<String>>,
}

/// Fan out `PATCH /api/admin/settings` to the target backends (feature 23 §5.4). **Best-effort**: a
/// locked/failed field on one backend never aborts the others; returns a per-backend result list.
pub async fn config_matrix_patch(
    _a: AuthAdmin,
    State(state): State<AppState>,
    Json(p): Json<ConfigMatrixPatch>,
) -> Result<Json<Value>, AppError> {
    let all = repository::list_backends(&state.db).await?;
    let targets: Vec<String> = match &p.targets {
        Some(list) => list.clone(),
        None => all
            .iter()
            .filter(|b| b.reachable)
            .map(|b| b.back_domain.clone())
            .collect(),
    };
    let body = serde_json::json!({ "key": p.key, "value": p.value });
    let mut results = serde_json::Map::new();
    for d in targets {
        let outcome = match state
            .backends
            .proxy_json(&d, Method::PATCH, "/api/admin/settings", Some(body.clone()))
            .await
        {
            Ok((status, _)) => {
                serde_json::json!({ "ok": (200..300).contains(&status), "status": status })
            }
            Err(e) => serde_json::json!({ "ok": false, "error": e.to_string() }),
        };
        results.insert(d, outcome);
    }
    Ok(Json(Value::Object(results)))
}
