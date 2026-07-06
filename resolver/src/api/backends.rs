//! Backend→resolver push endpoints (self-registration, heartbeat, mapping update) — all authed by a
//! shared-secret `Resolver` push token ([`AuthPush`]).

use crate::api::middleware::AuthPush;
use crate::config::setting_keys as sk;
use crate::repository;
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::Json;
use axum::extract::State;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

#[derive(Debug, Deserialize)]
pub struct RegisterBackendRequest {
    pub back_domain: String,
    pub use_https: bool,
    pub internal_url: String,
}

#[derive(Debug, Serialize)]
pub struct Ack {
    pub ok: bool,
}

pub async fn self_register(
    _auth: AuthPush,
    State(state): State<AppState>,
    Json(p): Json<RegisterBackendRequest>,
) -> Result<Json<Ack>, AppError> {
    if p.back_domain.is_empty() || p.internal_url.is_empty() {
        return Err(AppError::BadRequest(
            "back_domain and internal_url are required".to_string(),
        ));
    }
    repository::upsert_backend(&state.db, &p.back_domain, p.use_https, &p.internal_url).await?;
    info!(back_domain = %p.back_domain, "backend registered");
    Ok(Json(Ack { ok: true }))
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    pub back_domain: String,
    pub delegation_token: String,
    pub user_count: i64,
    pub picture_count: i64,
    pub storage_bytes: i64,
    pub healthy: bool,
    pub version: String,
}

/// Heartbeat consumer (feature 23 §3.2): store the delegation token + metrics, mark reachable.
pub async fn heartbeat(
    _auth: AuthPush,
    State(state): State<AppState>,
    Json(p): Json<HeartbeatRequest>,
) -> Result<Json<Ack>, AppError> {
    let ttl = state.config.get(sk::DELEGATION_STALE_SECS) as i64;
    let expires_at = Utc::now() + Duration::seconds(ttl);
    let known = repository::record_heartbeat(
        &state.db,
        &p.back_domain,
        &p.delegation_token,
        expires_at,
        p.user_count,
        p.picture_count,
        p.storage_bytes,
        p.healthy,
        &p.version,
    )
        .await?;
    if !known {
        return Err(AppError::NotFound);
    }
    debug!(back_domain = %p.back_domain, "heartbeat stored");
    Ok(Json(Ack { ok: true }))
}

#[derive(Debug, Deserialize)]
pub struct UpdateMappingRequest {
    pub username: String,
    pub back_domain: String,
}

pub async fn update_mapping(
    _auth: AuthPush,
    State(state): State<AppState>,
    Json(p): Json<UpdateMappingRequest>,
) -> Result<Json<Ack>, AppError> {
    if p.username.is_empty() || p.back_domain.is_empty() {
        return Err(AppError::BadRequest(
            "username and back_domain are required".to_string(),
        ));
    }
    repository::upsert_mapping(&state.db, &p.username, &p.back_domain).await?;
    state.cache.invalidate(&p.username).await;
    debug!(user = %p.username, back_domain = %p.back_domain, "mapping updated");
    Ok(Json(Ack { ok: true }))
}

pub async fn list_backends(
    _auth: AuthPush,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let backends: Vec<String> = repository::list_backends(&state.db)
        .await?
        .into_iter()
        .map(|b| b.back_domain)
        .collect();
    Ok(Json(serde_json::json!({ "backends": backends })))
}

// ── Backend-driven invites (feature 23 §6.2) ────────────────────────────────────
//
// In resolver mode invites live in the resolver's DB (it handles registration). A user minting an
// invite on their backend pushes it up here; the backend proxies list/revoke for that user.

use archypix_common::registration::Invite;
use chrono::DateTime;

#[derive(Debug, Deserialize)]
pub struct CreateInviteRequest {
    pub created_by: String,
    pub max_uses: Option<i64>,
    pub expires_at: Option<DateTime<Utc>>,
    pub instance_pin: Option<String>,
}

pub async fn create_invite(
    _auth: AuthPush,
    State(state): State<AppState>,
    Json(p): Json<CreateInviteRequest>,
) -> Result<Json<Invite>, AppError> {
    let code = archypix_common::registration::generate_invite_code();
    let inv = repository::create_invite(
        &state.db,
        &code,
        p.max_uses,
        p.expires_at,
        &p.created_by,
        p.instance_pin.as_deref(),
    )
        .await?;
    Ok(Json(inv))
}

#[derive(Debug, Deserialize)]
pub struct ListInvitesQuery {
    pub created_by: Option<String>,
}

pub async fn list_invites(
    _auth: AuthPush,
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<ListInvitesQuery>,
) -> Result<Json<Vec<Invite>>, AppError> {
    let invites = match q.created_by {
        Some(u) => repository::list_invites_by(&state.db, &u).await?,
        None => repository::list_invites(&state.db).await?,
    };
    Ok(Json(invites))
}

pub async fn delete_invite(
    _auth: AuthPush,
    State(state): State<AppState>,
    axum::extract::Path(code): axum::extract::Path<String>,
) -> Result<Json<Ack>, AppError> {
    repository::delete_invite(&state.db, &code).await?;
    Ok(Json(Ack { ok: true }))
}
