//! All models for the federation API are defined in `clients/federation/models.rs`.
use crate::api::middleware::auth_federation::AuthFederation;
use crate::clients::federation::models::{
    FederationAuthGrant, FederationAuthRequest, FederationMessageType, FederationResponse,
    PictureEditRequest, PicturesAnnouncementRequest, PicturesUnannouncementRequest, PresignRequest,
    PresignResponse, PresignResultItem, PublicShareClaimRequest, ShareAcceptRequest,
    ShareAnnouncementRequest, ShareRejectRequest, ShareRevokeRequest,
};
use crate::infra::observability;
use crate::infra::ratelimit::{self, category};
use crate::infra::settings::keys;
use crate::services::federation::{self as fed, PresignTokenItem};
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use std::net::SocketAddr;
use tracing::debug;

/// Structural batch ceilings (feature 28 §9.1). Hardcoded, never DB-editable — they must never
/// block a legitimately-large page from a differently-configured peer, only stop absurd payloads.
const MAX_PRESIGN_BATCH: usize = 10_000;
const MAX_ANNOUNCE_BATCH: usize = 10_000;

#[tracing::instrument(skip(state, payload), fields(peer_user = %payload.username, peer_domain = %payload.requester_instance))]
pub async fn auth_request(
    State(state): State<AppState>,
    Json(payload): Json<FederationAuthRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let token = state
        .federation
        .issue_federation_token(&payload.requester_instance)?;
    state
        .federation
        .send_auth_grant(
            &payload.username,
            &payload.requester_instance,
            &FederationAuthGrant {
                issuer_instance: state.settings.get(keys::GLOBAL_DOMAIN).clone(),
                token,
                // Relative TTL — the receiver computes its own expiry against its own clock (§4.4).
                ttl_secs: state.settings.get(keys::FEDERATION_JWT_TTL_SECS),
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
    state
        .federation
        .store_federation_token(
            &payload.issuer_instance,
            &payload.token,
            payload.ttl_secs,
            &payload.nonce,
        )
        .await?;
    Ok(Json(serde_json::json!({ "stored": true })))
}

/// The single authenticated federation message endpoint (feature 28 §5.3). Verifies the peer, rate
/// limits per peer domain, checks the per-message protocol version, then dispatches to the matching
/// `services::federation::receive_*`.
#[tracing::instrument(skip(auth, state, headers, envelope), fields(peer_domain = %auth.claims.sub))]
pub async fn message(
    auth: AuthFederation,
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(envelope): Json<serde_json::Value>,
) -> Result<Json<FederationResponse>, AppError> {
    observability::maybe_set_remote_parent(&headers, &auth.claims.sub, &state.settings);

    // Per-peer-domain frequency limit (§9.1). Large by default — never trips normal behaviour.
    ratelimit::check_categorized(
        state.cache.as_ref(),
        category::FEDERATION,
        &format!("federation:{}", auth.claims.sub),
        state.settings.get(keys::FEDERATION_RATE_MAX),
        state.settings.get(keys::FEDERATION_RATE_WINDOW_SECS),
        state.settings.get(keys::RATE_LIMIT_EVENT_RETENTION_SECS),
    )
        .await?;

    let msg_version = envelope
        .get("msg_version")
        .and_then(|v| v.as_u64())
        .map(|v| v as u16)
        .unwrap_or(0);
    let msg_type = envelope
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("missing federation message type".to_string()))?
        .to_string();

    let peer = auth.claims.sub.clone();
    let response = match msg_type.as_str() {
        ShareAnnouncementRequest::TYPE_NAME => {
            check_version::<ShareAnnouncementRequest>(msg_version)?;
            let p: ShareAnnouncementRequest = decode(envelope)?;
            let (incoming_id, auto_accepted) = fed::receive_share_announcement(
                &state.db,
                &state.settings,
                &state.routines.pipeline,
                &peer,
                &p.sender_username,
                &p.sender_instance,
                &p.recipient_username,
                &p.recipient_instance,
                p.outgoing_share_id,
                &p.tag_path,
                &p.name,
                p.message.as_deref(),
                p.allow_share_back,
                p.allow_exif_edit,
                p.future,
                p.shareback_of,
            )
                .await?;
            debug!(share_id = %incoming_id, auto_accepted, "federation: incoming share stored");
            FederationResponse::ShareAnnounce(
                crate::clients::federation::models::ShareAnnouncementResponse {
                    accepted: true,
                    auto_accepted,
                },
            )
        }
        ShareAcceptRequest::TYPE_NAME => {
            check_version::<ShareAcceptRequest>(msg_version)?;
            let p: ShareAcceptRequest = decode(envelope)?;
            fed::receive_share_accept(
                &state.db,
                &state.routines.pipeline,
                &peer,
                p.outgoing_share_id,
            )
                .await?;
            FederationResponse::Ack
        }
        ShareRejectRequest::TYPE_NAME => {
            check_version::<ShareRejectRequest>(msg_version)?;
            let p: ShareRejectRequest = decode(envelope)?;
            fed::receive_share_reject(&state.db, &peer, p.outgoing_share_id).await?;
            FederationResponse::Ack
        }
        ShareRevokeRequest::TYPE_NAME => {
            check_version::<ShareRevokeRequest>(msg_version)?;
            let p: ShareRevokeRequest = decode(envelope)?;
            fed::receive_share_revoke(
                &state.db,
                state.cache.as_ref(),
                &state.federation,
                &state.settings,
                &state.routines.unannounce,
                &state.routines.pipeline,
                &peer,
                p.outgoing_share_id,
            )
                .await?;
            FederationResponse::Ack
        }
        PublicShareClaimRequest::TYPE_NAME => {
            check_version::<PublicShareClaimRequest>(msg_version)?;
            let p: PublicShareClaimRequest = decode(envelope)?;
            // Bind the proposing identity to the authenticated peer: a peer may only claim as itself.
            if p.requester_instance != peer {
                return Err(AppError::Unauthorized(
                    "Requester instance does not match the authenticated instance".to_string(),
                ));
            }
            let meta = fed::receive_public_claim(
                state.cache.as_ref(),
                &state.db,
                &state.routines.pipeline,
                &state.settings,
                &p.token,
                &p.requester_username,
                &p.requester_instance,
            )
                .await?;
            FederationResponse::PublicShareClaim(meta)
        }
        PicturesAnnouncementRequest::TYPE_NAME => {
            check_version::<PicturesAnnouncementRequest>(msg_version)?;
            let p: PicturesAnnouncementRequest = decode(envelope)?;
            if p.pictures.len() > MAX_ANNOUNCE_BATCH {
                return Err(AppError::BadRequest("announce batch too large".to_string()));
            }
            let registered = fed::receive_pictures_announcement(
                &state.db,
                state.cache.as_ref(),
                &state.settings,
                &state.routines.pipeline,
                &peer,
                &p.sender_username,
                &p.sender_instance,
                p.outgoing_share_id,
                &p.tag_path,
                p.pictures,
            )
                .await?;
            FederationResponse::PicturesAnnounce(
                crate::clients::federation::models::PicturesAnnouncementResponse { registered },
            )
        }
        PicturesUnannouncementRequest::TYPE_NAME => {
            check_version::<PicturesUnannouncementRequest>(msg_version)?;
            let p: PicturesUnannouncementRequest = decode(envelope)?;
            if p.picture_ids.len() > MAX_ANNOUNCE_BATCH {
                return Err(AppError::BadRequest("unannounce batch too large".to_string()));
            }
            fed::receive_pictures_unannouncement(
                &state.db,
                &state.routines.pipeline,
                &peer,
                p.outgoing_share_id,
                &p.picture_ids,
            )
                .await?;
            FederationResponse::Ack
        }
        PictureEditRequest::TYPE_NAME => {
            check_version::<PictureEditRequest>(msg_version)?;
            let p: PictureEditRequest = decode(envelope)?;
            // Bind the proposing identity to the authenticated peer.
            if p.requester_instance != peer {
                return Err(AppError::Unauthorized(
                    "Requester instance does not match the authenticated instance".to_string(),
                ));
            }
            fed::receive_picture_edit_request(
                &state.db,
                &state.routines.pipeline,
                &p.picture_id,
                &p.requester_username,
                &p.requester_instance,
                p.set,
                p.clear,
            )
                .await?;
            FederationResponse::PictureEdit(
                crate::clients::federation::models::PictureEditResponse { accepted: true },
            )
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "unknown federation message type: {other}"
            )));
        }
    };
    Ok(Json(response))
}

/// Compare the envelope's `msg_version` to the matched message type's `VERSION`; on mismatch return
/// a `426 Upgrade Required` with `receiver_version`, which the caller turns into a directional error
/// (§5.4).
fn check_version<M: FederationMessageType>(msg_version: u16) -> Result<(), AppError> {
    if msg_version != M::VERSION {
        return Err(AppError::Custom(
            426,
            serde_json::json!({
                "error": "version_mismatch",
                "message_type": M::TYPE_NAME,
                "receiver_version": M::VERSION,
            }),
        ));
    }
    Ok(())
}

/// Decode a message-envelope `Value` into the concrete request struct (the extra `type` /
/// `msg_version` fields are ignored).
fn decode<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Result<T, AppError> {
    serde_json::from_value(value)
        .map_err(|e| AppError::BadRequest(format!("malformed federation message: {e}")))
}

#[tracing::instrument(skip(state, payload, addr))]
pub async fn presign_pictures(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<PresignRequest>,
) -> Result<Json<PresignResponse>, AppError> {
    // Per-source-IP presign frequency limit (§9.1). Very generous — per-IP ≈ per-peer-backend here.
    ratelimit::check_categorized(
        state.cache.as_ref(),
        category::PRESIGN,
        &format!("presign:{}", addr.ip()),
        state.settings.get(keys::FEDERATION_PRESIGN_RATE_MAX),
        state.settings.get(keys::FEDERATION_PRESIGN_RATE_WINDOW_SECS),
        state.settings.get(keys::RATE_LIMIT_EVENT_RETENTION_SECS),
    )
        .await?;
    if payload.pictures.len() > MAX_PRESIGN_BATCH {
        return Err(AppError::BadRequest("presign batch too large".to_string()));
    }
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
            .map(|(picture_token, url, expires_at)| PresignResultItem {
                picture_token,
                url,
                expires_at: Some(expires_at),
            })
            .collect(),
    }))
}
