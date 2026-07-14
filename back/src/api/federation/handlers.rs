//! All models for the federation API are defined in `clients/federation/models.rs`.
use crate::api::middleware::auth_federation::AuthFederation;
use crate::clients::federation::models::{
    FederationAuthGrant, FederationAuthRequest, PictureEditRequest, PictureEditResponse,
    PicturesAnnouncementRequest, PicturesUnannouncementRequest, PresignRequest, PresignResponse,
    PresignResultItem, PublicShareClaimRequest, PublicShareClaimResponse, ShareAcceptRequest,
    ShareAnnouncementRequest, ShareAnnouncementResponse, ShareRejectRequest, ShareRevokeRequest,
};
use crate::infra::observability;
use crate::infra::settings::keys;
use crate::services::federation::{self as fed, PresignTokenItem};
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use chrono::Utc;
use tracing::debug;

#[tracing::instrument(skip(state, payload), fields(peer_user = %payload.username, peer_domain = %payload.requester_instance))]
pub async fn auth_request(
    State(state): State<AppState>,
    Json(payload): Json<FederationAuthRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let token = state
        .federation
        .issue_federation_token(&payload.requester_instance)?;
    let expires_at = Utc::now().timestamp() + state.settings.get(keys::FEDERATION_JWT_TTL_SECS);
    state
        .federation
        .send_auth_grant(
            &payload.username,
            &payload.requester_instance,
            &FederationAuthGrant {
                issuer_instance: state.settings.get(keys::GLOBAL_DOMAIN).clone(),
                token,
                expires_at,
                scope: payload.scope,
                nonce: payload.nonce,
            },
        )
        .await?;
    Ok(Json(serde_json::json!({ "accepted": true })))
}

#[tracing::instrument(skip(state, payload), fields(issuer_instance = %payload.issuer_instance))]
pub async fn auth_grant(
    State(state): State<AppState>,
    Json(payload): Json<FederationAuthGrant>,
) -> Result<Json<serde_json::Value>, AppError> {
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

#[tracing::instrument(skip(auth, state, payload, headers), fields(peer_user = %payload.sender_username, peer_domain = %auth.claims.sub))]
pub async fn announce_share(
    auth: AuthFederation,
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(payload): Json<ShareAnnouncementRequest>,
) -> Result<Json<ShareAnnouncementResponse>, AppError> {
    observability::maybe_set_remote_parent(&headers, &auth.claims.sub, &state.settings);

    let (incoming_id, auto_accepted) = fed::receive_share_announcement(
        &state.db,
        &state.settings,
        &state.routines.pipeline,
        &auth.claims.sub,
        &payload.sender_username,
        &payload.sender_instance,
        &payload.recipient_username,
        &payload.recipient_instance,
        payload.outgoing_share_id,
        &payload.tag_path,
        &payload.name,
        payload.message.as_deref(),
        payload.allow_share_back,
        payload.allow_exif_edit,
        payload.future,
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
    Ok(Json(ShareAnnouncementResponse {
        accepted: true,
        auto_accepted,
    }))
}

#[tracing::instrument(skip(auth, state, payload, headers), fields(peer_domain = %auth.claims.sub, outgoing_share_id = %payload.outgoing_share_id))]
pub async fn revoke_share(
    auth: AuthFederation,
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(payload): Json<ShareRevokeRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    observability::maybe_set_remote_parent(&headers, &auth.claims.sub, &state.settings);
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
        &state.settings,
        &state.routines.unannounce,
        &state.routines.pipeline,
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

#[tracing::instrument(skip(auth, state, payload, headers), fields(peer_domain = %auth.claims.sub, outgoing_share_id = %payload.outgoing_share_id))]
pub async fn reject_share(
    auth: AuthFederation,
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(payload): Json<ShareRejectRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    observability::maybe_set_remote_parent(&headers, &auth.claims.sub, &state.settings);
    debug!(
        user = %auth.claims.sub,
        token_type = "federation",
        outgoing_share_id = %payload.outgoing_share_id,
        "federation: reject_share"
    );
    fed::receive_share_reject(&state.db, &auth.claims.sub, payload.outgoing_share_id).await?;
    Ok(Json(serde_json::json!({ "rejected": true })))
}

#[tracing::instrument(skip(auth, state, payload, headers), fields(peer_domain = %auth.claims.sub, outgoing_share_id = %payload.outgoing_share_id))]
pub async fn accept_share(
    auth: AuthFederation,
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(payload): Json<ShareAcceptRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    observability::maybe_set_remote_parent(&headers, &auth.claims.sub, &state.settings);
    debug!(
        user = %auth.claims.sub,
        token_type = "federation",
        outgoing_share_id = %payload.outgoing_share_id,
        "federation: accept_share"
    );
    fed::receive_share_accept(
        &state.db,
        &state.routines.pipeline,
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

#[tracing::instrument(skip(auth, state, payload, headers), fields(peer_user = %payload.sender_username, peer_domain = %auth.claims.sub, outgoing_share_id = %payload.outgoing_share_id
))]
pub async fn announce_pictures(
    auth: AuthFederation,
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(payload): Json<PicturesAnnouncementRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    observability::maybe_set_remote_parent(&headers, &auth.claims.sub, &state.settings);
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
        &state.settings,
        &state.routines.pipeline,
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

#[tracing::instrument(skip(auth, state, payload, headers), fields(peer_user = %payload.sender_username, peer_domain = %auth.claims.sub, outgoing_share_id = %payload.outgoing_share_id
))]
pub async fn unannounce_pictures(
    auth: AuthFederation,
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(payload): Json<PicturesUnannouncementRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    observability::maybe_set_remote_parent(&headers, &auth.claims.sub, &state.settings);
    debug!(
        user = %auth.claims.sub,
        token_type = "federation",
        outgoing_share_id = %payload.outgoing_share_id,
        picture_count = payload.picture_ids.len(),
        "federation: unannounce_pictures"
    );
    let deleted = fed::receive_pictures_unannouncement(
        &state.db,
        &state.routines.pipeline,
        &auth.claims.sub,
        payload.outgoing_share_id,
        &payload.picture_ids,
    )
    .await?;
    Ok(Json(
        serde_json::json!({ "unannounced": true, "pictures_deleted": deleted }),
    ))
}

/// `POST /api/federation/pictures/edit_request` — owner-side handler for a recipient's EXIF edit
/// proposal (10 §4.2). The requester's instance must match the authenticated federation instance;
/// the owner re-verifies the EXIF-edit grant and applies the edit through its `edit_picture`
/// write-through, re-announcing to all recipients.
#[tracing::instrument(skip(auth, state, payload, headers), fields(peer_user = %payload.requester_username, peer_domain = %auth.claims.sub, picture_id = %payload.picture_id
))]
pub async fn edit_picture_request(
    auth: AuthFederation,
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(payload): Json<PictureEditRequest>,
) -> Result<Json<PictureEditResponse>, AppError> {
    observability::maybe_set_remote_parent(&headers, &auth.claims.sub, &state.settings);
    debug!(
        user = %payload.requester_username,
        token_type = "federation",
        requester_instance = %payload.requester_instance,
        picture_id = %payload.picture_id,
        "federation: edit_picture_request"
    );
    // Bind the proposing identity to the authenticated peer: a peer may only propose as itself.
    if payload.requester_instance != auth.claims.sub {
        return Err(AppError::Unauthorized(
            "Requester instance does not match the authenticated instance".to_string(),
        ));
    }
    fed::receive_picture_edit_request(
        &state.db,
        &state.routines.pipeline,
        &payload.picture_id,
        &payload.requester_username,
        &payload.requester_instance,
        payload.set,
        payload.clear,
    )
    .await?;
    Ok(Json(PictureEditResponse { accepted: true }))
}

/// `POST /api/federation/shares/public/claim` — owner-side handler for a visitor's Subscribe
/// (feature 27 §8/§11). The proposing instance must match the authenticated federation peer; the
/// owner mints a derived `OutgoingShare` and returns its metadata.
#[tracing::instrument(skip(auth, state, payload, headers), fields(peer_user = %payload.requester_username, peer_domain = %auth.claims.sub))]
pub async fn claim_public_share(
    auth: AuthFederation,
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(payload): Json<PublicShareClaimRequest>,
) -> Result<Json<PublicShareClaimResponse>, AppError> {
    observability::maybe_set_remote_parent(&headers, &auth.claims.sub, &state.settings);
    // Bind the proposing identity to the authenticated peer: a peer may only claim as itself.
    if payload.requester_instance != auth.claims.sub {
        return Err(AppError::Unauthorized(
            "Requester instance does not match the authenticated instance".to_string(),
        ));
    }
    let meta = fed::receive_public_claim(
        state.cache.as_ref(),
        &state.db,
        &state.routines.pipeline,
        &state.settings,
        &payload.token,
        &payload.requester_username,
        &payload.requester_instance,
    )
    .await?;
    Ok(Json(meta))
}

#[tracing::instrument(skip(state, payload))]
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
        fed::presign_by_picture_tokens(&state.db, state.storage.as_ref(), &state.settings, &items)
            .await?;
    Ok(Json(PresignResponse {
        urls: results
            .into_iter()
            .map(|(picture_token, url)| PresignResultItem { picture_token, url })
            .collect(),
    }))
}
