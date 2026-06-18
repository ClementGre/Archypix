//! All models for the federation API are defined in `clients/federation/models.rs`.
use crate::api::middleware::auth_federation::AuthFederation;
use crate::clients::federation::models::{
    FederationAuthGrant, FederationAuthRequest, PicturesAnnouncementRequest,
    PicturesUnannouncementRequest, PresignRequest, PresignResponse, PresignResultItem,
    ShareAcceptRequest, ShareAnnouncementRequest, ShareAnnouncementResponse, ShareRejectRequest,
    ShareRevokeRequest,
};
use crate::infra::error::AppError;
use crate::services::federation::{self as fed, PresignTokenItem};
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use chrono::Utc;
use tracing::debug;

pub async fn auth_request(
    State(state): State<AppState>,
    Json(payload): Json<FederationAuthRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    debug!(
        user = %payload.username,
        token_type = "federation",
        requester_instance = %payload.requester_instance,
        "federation: auth_request"
    );
    let token = state
        .federation
        .issue_federation_token(&payload.requester_instance)?;
    let expires_at = Utc::now().timestamp() + state.config.federation_jwt_ttl_secs;
    state
        .federation
        .send_auth_grant(
            &payload.username,
            &payload.requester_instance,
            &FederationAuthGrant {
                issuer_instance: state.config.global_domain.clone(),
                token,
                expires_at,
                scope: payload.scope,
                nonce: payload.nonce,
            },
        )
        .await?;
    Ok(Json(serde_json::json!({ "accepted": true })))
}

pub async fn auth_grant(
    State(state): State<AppState>,
    Json(payload): Json<FederationAuthGrant>,
) -> Result<Json<serde_json::Value>, AppError> {
    debug!(
        user = "-",
        token_type = "federation",
        issuer_instance = %payload.issuer_instance,
        "federation: auth_grant"
    );
    let ttl = payload.expires_at - Utc::now().timestamp();
    if ttl <= 0 {
        return Err(AppError::BadRequest("Token already expired".to_string()));
    }
    state
        .federation
        .store_federation_token(
            &payload.issuer_instance,
            &payload.token,
            ttl,
            &payload.nonce,
        )
        .await?;
    Ok(Json(serde_json::json!({ "stored": true })))
}

pub async fn announce_share(
    auth: AuthFederation,
    State(state): State<AppState>,
    Json(payload): Json<ShareAnnouncementRequest>,
) -> Result<Json<ShareAnnouncementResponse>, AppError> {
    debug!(
        user = %auth.claims.sub,
        token_type = "federation",
        sender = %payload.sender_username,
        sender_instance = %payload.sender_instance,
        recipient = %payload.recipient_username,
        tag_path = %payload.tag_path,
        "federation: announce_share"
    );
    let (incoming_id, auto_accepted) = fed::receive_share_announcement(
        &state.db,
        &state.config,
        &state.pipeline_waker,
        &auth.claims.sub,
        &payload.sender_username,
        &payload.sender_instance,
        &payload.recipient_username,
        &payload.recipient_instance,
        payload.outgoing_share_id,
        payload.allow_share_back,
        payload.shareback_of,
    )
    .await?;
    debug!(
        user = %auth.claims.sub,
        token_type = "federation",
        share_id = %incoming_id,
        auto_accepted,
        sender = %payload.sender_username,
        sender_instance = %payload.sender_instance,
        "federation: incoming share stored"
    );
    // `auto_accepted = true` tells the sender (a ShareBack initiator) to announce its pictures
    // itself, keeping the whole flow inside the sender's transaction (no callback into the
    // still-uncommitted sender). See federation consistency rules in 03_BACKEND_ARCHITECTURE.md.
    Ok(Json(ShareAnnouncementResponse {
        accepted: true,
        auto_accepted,
    }))
}

pub async fn revoke_share(
    auth: AuthFederation,
    State(state): State<AppState>,
    Json(payload): Json<ShareRevokeRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    debug!(
        user = %auth.claims.sub,
        token_type = "federation",
        outgoing_share_id = %payload.outgoing_share_id,
        "federation: revoke_share"
    );
    let deleted = fed::receive_share_revoke(
        &state.db,
        state.cache.as_ref(),
        &state.federation,
        &state.config,
        &state.task_queue,
        &state.pipeline_waker,
        &auth.claims.sub,
        payload.outgoing_share_id,
    )
    .await?;
    debug!(
        user = %auth.claims.sub,
        token_type = "federation",
        outgoing_share_id = %payload.outgoing_share_id,
        deleted_pictures = deleted,
        "federation: share revoked"
    );
    Ok(Json(
        serde_json::json!({ "revoked": true, "pictures_deleted": deleted }),
    ))
}

pub async fn reject_share(
    auth: AuthFederation,
    State(state): State<AppState>,
    Json(payload): Json<ShareRejectRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    debug!(
        user = %auth.claims.sub,
        token_type = "federation",
        outgoing_share_id = %payload.outgoing_share_id,
        "federation: reject_share"
    );
    fed::receive_share_reject(&state.db, &auth.claims.sub, payload.outgoing_share_id).await?;
    Ok(Json(serde_json::json!({ "rejected": true })))
}

pub async fn accept_share(
    auth: AuthFederation,
    State(state): State<AppState>,
    Json(payload): Json<ShareAcceptRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    debug!(
        user = %auth.claims.sub,
        token_type = "federation",
        outgoing_share_id = %payload.outgoing_share_id,
        "federation: accept_share"
    );
    fed::receive_share_accept(
        &state.db,
        &state.pipeline_waker,
        &auth.claims.sub,
        payload.outgoing_share_id,
    )
    .await?;
    debug!(
        outgoing_share_id = %payload.outgoing_share_id,
        "federation: share accepted — first announcement queued to pipeline"
    );
    Ok(Json(serde_json::json!({ "accepted": true })))
}

pub async fn announce_pictures(
    auth: AuthFederation,
    State(state): State<AppState>,
    Json(payload): Json<PicturesAnnouncementRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    debug!(
        user = %auth.claims.sub,
        token_type = "federation",
        outgoing_share_id = %payload.outgoing_share_id,
        picture_count = payload.pictures.len(),
        "federation: announce_pictures"
    );
    let registered = fed::receive_pictures_announcement(
        &state.db,
        state.cache.as_ref(),
        &state.config,
        &state.pipeline_waker,
        &auth.claims.sub,
        &payload.sender_username,
        &payload.sender_instance,
        payload.outgoing_share_id,
        &payload.tag_path,
        payload.pictures,
    )
    .await?;
    debug!(
        outgoing_share_id = %payload.outgoing_share_id,
        registered,
        "federation: pictures registered"
    );
    Ok(Json(serde_json::json!({ "registered": registered })))
}

pub async fn unannounce_pictures(
    auth: AuthFederation,
    State(state): State<AppState>,
    Json(payload): Json<PicturesUnannouncementRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    debug!(
        user = %auth.claims.sub,
        token_type = "federation",
        outgoing_share_id = %payload.outgoing_share_id,
        picture_count = payload.picture_ids.len(),
        "federation: unannounce_pictures"
    );
    let deleted = fed::receive_pictures_unannouncement(
        &state.db,
        &state.pipeline_waker,
        &auth.claims.sub,
        payload.outgoing_share_id,
        &payload.picture_ids,
    )
    .await?;
    Ok(Json(
        serde_json::json!({ "unannounced": true, "pictures_deleted": deleted }),
    ))
}

pub async fn presign_pictures(
    State(state): State<AppState>,
    Json(payload): Json<PresignRequest>,
) -> Result<Json<PresignResponse>, AppError> {
    debug!(
        picture_count = payload.pictures.len(),
        "federation: presign_picture"
    );
    let items: Vec<PresignTokenItem> = payload
        .pictures
        .iter()
        .map(|p| PresignTokenItem {
            picture_token: p.picture_token,
            variant: p.variant.clone(),
        })
        .collect();
    let results =
        fed::presign_by_picture_tokens(&state.db, state.storage.as_ref(), &state.config, &items)
            .await?;
    Ok(Json(PresignResponse {
        urls: results
            .into_iter()
            .map(|(picture_token, url)| PresignResultItem { picture_token, url })
            .collect(),
    }))
}
