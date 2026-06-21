use crate::api::middleware::auth_user::AuthUser;
use crate::domain::share::ShareStatus;
use crate::domain::tag::TagPath;
use crate::infra::error::AppError;
use crate::repository::share::{IncomingShareRepository, OutgoingShareRepository};
use crate::services;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct CreateOutgoingRequest {
    pub tag_path: String,
    pub name: String,
    pub message: Option<String>,
    pub recipient_username: String,
    pub recipient_instance: String,
    pub allow_share_back: Option<bool>,
    /// Grant recipients EXIF editing of the shared pictures (10 §3). Default `false`.
    pub allow_exif_edit: Option<bool>,
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
    /// Whether recipients may propose EXIF edits the owner auto-applies (10 §3).
    pub allow_exif_edit: bool,
    pub future: bool,
    /// ShareBack provenance: the recipient's incoming share (by its `outgoing_share_id`) this
    /// share answers. `None` for a normal share.
    pub shareback_of: Option<uuid::Uuid>,
    /// Announcement retry/backoff (set while `errored`/recovering).
    pub last_error_at: Option<NaiveDateTime>,
    pub next_retry_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    /// When the share was closed (revoked or rejected); `None` while live.
    pub revoked_at: Option<NaiveDateTime>,
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
    /// Whether the sender allows the recipient to propose EXIF edits (10 §3).
    pub allow_exif_edit: bool,
    pub future: bool,
    /// Local `/SharedToMe/<sender>/…` tag (ltree wire form) the received pictures land under.
    pub shared_tag_path: Option<String>,
    pub last_announcement_received_at: Option<NaiveDateTime>,
    pub shareback_of: Option<uuid::Uuid>,
    pub local_mapping_service_id: Option<uuid::Uuid>,
    pub created_at: NaiveDateTime,
    /// When the share was closed (revoked by the sender or rejected here); `None` while live.
    pub revoked_at: Option<NaiveDateTime>,
}

#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn create_outgoing(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<CreateOutgoingRequest>,
) -> Result<Json<ShareResponse>, AppError> {
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
        payload.allow_exif_edit.unwrap_or(false),
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
        allow_exif_edit: share.allow_exif_edit,
        future: share.future,
        shareback_of: share.shareback_of,
        last_error_at: share.last_error_at,
        next_retry_at: share.next_retry_at,
        created_at: share.created_at,
        revoked_at: share.revoked_at,
    }))
}

#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn list_outgoing(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<ShareResponse>>, AppError> {
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
                allow_exif_edit: s.allow_exif_edit,
                future: s.future,
                shareback_of: s.shareback_of,
                last_error_at: s.last_error_at,
                next_retry_at: s.next_retry_at,
                created_at: s.created_at,
                revoked_at: s.revoked_at,
            })
            .collect(),
    ))
}

#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn list_incoming(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<IncomingShareResponse>>, AppError> {
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
                allow_exif_edit: s.allow_exif_edit,
                future: s.future,
                shared_tag_path: s.shared_tag_path,
                last_announcement_received_at: s.last_announcement_received_at,
                shareback_of: s.shareback_of,
                local_mapping_service_id: s.local_mapping_service_id,
                created_at: s.created_at,
                revoked_at: s.revoked_at,
            })
            .collect(),
    ))
}

#[tracing::instrument(skip(auth, state), fields(user_id = %auth.claims.uid.unwrap_or_default(), share_id = %share_id))]
pub async fn accept_incoming(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(share_id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
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

#[tracing::instrument(skip(auth, state), fields(user_id = %auth.claims.uid.unwrap_or_default(), share_id = %share_id))]
pub async fn revoke_outgoing(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(share_id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
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

#[tracing::instrument(skip(auth, state), fields(user_id = %auth.claims.uid.unwrap_or_default(), share_id = %share_id))]
pub async fn reject_incoming(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(share_id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
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
