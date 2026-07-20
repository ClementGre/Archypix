//! Unauthenticated public-share view + contribution endpoints (feature 27 §6/§7), under
//! `/api/public/shares/{token}`. Authorization is the token (+ an optional unlock JWT for a
//! password-gated share + optional expiry), re-validated on every request. The owner's management
//! surface lives in `api/user/public_shares.rs`.

use crate::services::pictures::{
    BatchUploadFile, BatchUploadOutcome, PictureVariant, UploadMetadata,
};
use crate::services::shares::public;
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::Json;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::HeaderMap;
use axum::http::header::AUTHORIZATION;
use serde::Deserialize;
use std::net::SocketAddr;
use uuid::Uuid;

/// Parse an optional `Authorization: Bearer <jwt>` (the unlock session for a password-gated share).
/// Absence is fine — only password-gated shares require it.
fn optional_bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(str::to_string)
}

#[tracing::instrument(skip(state, headers))]
pub async fn meta(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Result<Json<public::PublicShareMeta>, AppError> {
    let _ = headers;
    let meta = public::public_meta(&state.db, &state.settings, &token).await?;
    Ok(Json(meta))
}

#[derive(Debug, Deserialize)]
pub struct UnlockBody {
    pub password: String,
}

#[tracing::instrument(skip(state, body))]
pub async fn unlock(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Json(body): Json<UnlockBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let jwt = public::unlock(
        &state.db,
        &state.jwt,
        &state.settings,
        &token,
        &body.password,
    )
    .await?;
    Ok(Json(serde_json::json!({ "token": jwt })))
}

fn default_page() -> u32 {
    1
}
fn default_page_size() -> u32 {
    50
}

#[derive(Debug, Deserialize)]
pub struct PicturesQuery {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
    #[serde(default)]
    pub thumbnail: Option<PictureVariant>,
}

#[tracing::instrument(skip(state, headers, query))]
pub async fn pictures(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Query(query): Query<PicturesQuery>,
) -> Result<Json<crate::services::pictures::PictureListResult>, AppError> {
    let share = public::resolve_access(
        &state.db,
        &state.jwt,
        &state.settings,
        &token,
        optional_bearer(&headers).as_deref(),
    )
    .await?;
    let result = public::list_public_pictures(
        &state.db,
        state.cache.as_ref(),
        state.storage.as_ref(),
        &state.settings,
        &state.federation,
        &share,
        query.page,
        query.page_size,
        query.thumbnail.unwrap_or(PictureVariant::Medium),
    )
    .await?;
    Ok(Json(result))
}

#[derive(Debug, Deserialize)]
pub struct UrlQuery {
    pub variant: PictureVariant,
}

#[tracing::instrument(skip(state, headers, query))]
pub async fn picture_url(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((token, picture_id)): Path<(String, Uuid)>,
    Query(query): Query<UrlQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let share = public::resolve_access(
        &state.db,
        &state.jwt,
        &state.settings,
        &token,
        optional_bearer(&headers).as_deref(),
    )
    .await?;
    let url = public::presign_public_picture(
        &state.db,
        state.cache.as_ref(),
        state.storage.as_ref(),
        &state.settings,
        &state.federation,
        &share,
        picture_id,
        query.variant,
    )
    .await?;
    Ok(Json(
        serde_json::json!({ "url": url, "variant": query.variant }),
    ))
}

#[tracing::instrument(skip(state, headers))]
pub async fn picture_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((token, picture_id)): Path<(String, Uuid)>,
) -> Result<Json<public::PublicPictureDetail>, AppError> {
    let share = public::resolve_access(
        &state.db,
        &state.jwt,
        &state.settings,
        &token,
        optional_bearer(&headers).as_deref(),
    )
    .await?;
    let detail =
        public::public_picture_detail(&state.db, &state.settings, &share, picture_id).await?;
    Ok(Json(detail))
}

#[derive(Debug, Deserialize)]
pub struct AggregateBody {
    #[serde(default)]
    pub include_ids: Vec<Uuid>,
    #[serde(default)]
    pub sections: Option<Vec<crate::services::aggregate::AggregateSection>>,
}

#[tracing::instrument(skip(state, headers, body))]
pub async fn aggregate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Json(body): Json<AggregateBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let share = public::resolve_access(
        &state.db,
        &state.jwt,
        &state.settings,
        &token,
        optional_bearer(&headers).as_deref(),
    )
    .await?;
    let result = public::public_aggregate(
        &state.db,
        &state.settings,
        &share,
        body.include_ids,
        body.sections,
    )
    .await?;
    Ok(Json(result))
}

// ── Contribution ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UploadsBody {
    #[serde(default)]
    pub contributor_name: String,
    pub files: Vec<BatchUploadFile>,
}

/// Slot response mirroring the authenticated batch-upload shape.
#[derive(Debug, serde::Serialize)]
pub struct UploadSlot {
    pub picture_id: Uuid,
    pub presigned_url: Option<String>,
    /// A dedup hit (rejected — not stored, §7).
    pub rejected: bool,
}

#[tracing::instrument(skip(state, addr, body))]
pub async fn uploads(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(token): Path<String>,
    Json(body): Json<UploadsBody>,
) -> Result<Json<Vec<UploadSlot>>, AppError> {
    // No password JWT is required to contribute — `allow_upload` is the gate (a drop box need not be
    // readable). Resolve without the bearer.
    let share =
        public::resolve_access(&state.db, &state.jwt, &state.settings, &token, None).await?;
    let outcomes = public::public_upload_batch(
        &state.db,
        state.cache.as_ref(),
        state.storage.as_ref(),
        &state.settings,
        &state.routines.pipeline,
        &share,
        &addr.ip().to_string(),
        &body.files,
    )
    .await?;
    Ok(Json(
        outcomes
            .into_iter()
            .map(|o| match o {
                BatchUploadOutcome::New {
                    picture_id,
                    presigned_url,
                } => UploadSlot {
                    picture_id,
                    presigned_url: Some(presigned_url),
                    rejected: false,
                },
                BatchUploadOutcome::Duplicate { picture_id, .. } => UploadSlot {
                    picture_id,
                    presigned_url: None,
                    rejected: true,
                },
            })
            .collect(),
    ))
}

#[derive(Debug, Deserialize)]
pub struct CompleteBody {
    #[serde(default)]
    pub contributor_name: String,
    #[serde(flatten)]
    pub meta: UploadMetadata,
}

#[tracing::instrument(skip(state, body))]
pub async fn complete_upload(
    State(state): State<AppState>,
    Path((token, picture_id)): Path<(String, Uuid)>,
    Json(body): Json<CompleteBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let share =
        public::resolve_access(&state.db, &state.jwt, &state.settings, &token, None).await?;
    let picture = public::public_complete_upload(
        &state.db,
        state.cache.as_ref(),
        state.storage.as_ref(),
        &state.settings,
        &state.routines.pipeline,
        &share,
        picture_id,
        &body.contributor_name,
        body.meta,
    )
    .await?;
    Ok(Json(serde_json::json!({ "id": picture.id })))
}
