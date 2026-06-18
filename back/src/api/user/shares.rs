use crate::api::middleware::auth_user::AuthUser;
use crate::domain::share::ShareStatus;
use crate::domain::tag::TagPath;
use crate::infra::error::AppError;
use crate::repository::share::{IncomingShareRepository, OutgoingShareRepository};
use crate::services;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use tracing::debug;

#[derive(Debug, Deserialize)]
pub struct CreateOutgoingRequest {
    pub tag_path: String,
    pub name: String,
    pub message: Option<String>,
    pub recipient_username: String,
    pub recipient_instance: String,
    pub allow_share_back: Option<bool>,
    pub future: Option<bool>,
    pub shareback_of: Option<uuid::Uuid>,
}

#[derive(Debug, Serialize)]
pub struct ShareResponse {
    pub id: uuid::Uuid,
    pub tag_path: String,
    pub name: String,
    pub message: Option<String>,
    pub recipient_username: String,
    pub recipient_instance: String,
    pub status: ShareStatus,
    pub allow_share_back: bool,
    pub future: bool,
}

#[derive(Debug, Serialize)]
pub struct IncomingShareResponse {
    pub id: uuid::Uuid,
    pub sender_username: String,
    pub sender_instance: String,
    pub name: String,
    pub message: Option<String>,
    pub outgoing_share_id: uuid::Uuid,
    pub status: ShareStatus,
    pub allow_share_back: bool,
    pub local_mapping_service_id: Option<uuid::Uuid>,
}

pub async fn create_outgoing(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<CreateOutgoingRequest>,
) -> Result<Json<ShareResponse>, AppError> {
    debug!(
        user = %auth.claims.sub,
        token_type = auth.token_type(),
        tag_path = %payload.tag_path,
        recipient = %payload.recipient_username,
        "create_outgoing_share"
    );
    let tag_path = TagPath::parse(&payload.tag_path, true).map_err(AppError::BadRequest)?;
    let share = services::shares::create_outgoing_share(
        &state.db,
        state.cache.as_ref(),
        &state.federation,
        &state.config,
        &state.pipeline_waker,
        auth.user_id()?,
        &auth.claims.sub,
        tag_path.as_ltree(),
        &payload.name,
        payload.message.as_deref(),
        &payload.recipient_username,
        &payload.recipient_instance,
        payload.allow_share_back.unwrap_or(true),
        payload.future.unwrap_or(true),
        payload.shareback_of,
    )
    .await?;
    Ok(Json(ShareResponse {
        id: share.id,
        tag_path: share.tag_path,
        name: share.name,
        message: share.message,
        recipient_username: share.recipient_username,
        recipient_instance: share.recipient_instance,
        status: share.status,
        allow_share_back: share.allow_share_back,
        future: share.future,
    }))
}

pub async fn list_outgoing(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<ShareResponse>>, AppError> {
    debug!(user = %auth.claims.sub, token_type = auth.token_type(), "list_outgoing_shares");
    let shares = OutgoingShareRepository::list_by_owner(&state.db, auth.user_id()?).await?;
    Ok(Json(
        shares
            .into_iter()
            .map(|s| ShareResponse {
                id: s.id,
                tag_path: s.tag_path,
                name: s.name,
                message: s.message,
                recipient_username: s.recipient_username,
                recipient_instance: s.recipient_instance,
                status: s.status,
                allow_share_back: s.allow_share_back,
                future: s.future,
            })
            .collect(),
    ))
}

pub async fn list_incoming(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<IncomingShareResponse>>, AppError> {
    debug!(user = %auth.claims.sub, token_type = auth.token_type(), "list_incoming_shares");
    let shares = IncomingShareRepository::list_by_recipient(&state.db, auth.user_id()?).await?;
    Ok(Json(
        shares
            .into_iter()
            .map(|s| IncomingShareResponse {
                id: s.id,
                sender_username: s.sender_username,
                sender_instance: s.sender_instance,
                name: s.name,
                message: s.message,
                outgoing_share_id: s.outgoing_share_id,
                status: s.status,
                allow_share_back: s.allow_share_back,
                local_mapping_service_id: s.local_mapping_service_id,
            })
            .collect(),
    ))
}

pub async fn accept_incoming(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(share_id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    debug!(user = %auth.claims.sub, token_type = auth.token_type(), share_id = %share_id, "accept_incoming_share");
    services::shares::accept_incoming_share(
        &state.db,
        state.cache.as_ref(),
        &state.federation,
        &state.config,
        &state.pipeline_waker,
        auth.user_id()?,
        &auth.claims.sub,
        share_id,
    )
    .await?;
    // Pictures are announced asynchronously: the sender's OutgoingShare moves to
    // `pending_first_announcement` and the pipeline announces + activates it.
    Ok(Json(serde_json::json!({ "accepted": true })))
}

pub async fn revoke_outgoing(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(share_id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    debug!(user = %auth.claims.sub, token_type = auth.token_type(), share_id = %share_id, "revoke_outgoing_share");
    services::shares::revoke_outgoing_share(
        &state.db,
        state.cache.as_ref(),
        &state.federation,
        &state.config,
        &state.task_queue,
        &state.pipeline_waker,
        auth.user_id()?,
        &auth.claims.sub,
        share_id,
    )
    .await?;
    Ok(Json(serde_json::json!({ "revoked": true })))
}

pub async fn reject_incoming(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(share_id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    debug!(user = %auth.claims.sub, token_type = auth.token_type(), share_id = %share_id, "reject_incoming_share");
    services::shares::reject_incoming_share(
        &state.db,
        state.cache.as_ref(),
        &state.federation,
        &state.config,
        &state.task_queue,
        &state.pipeline_waker,
        auth.user_id()?,
        &auth.claims.sub,
        share_id,
    )
    .await?;
    Ok(Json(serde_json::json!({ "rejected": true })))
}
