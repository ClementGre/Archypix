//! Authenticated public-share endpoints (feature 27): the owner's management surface
//! (`/api/authenticated/shares/public`) plus the logged-in visitor's Convert actions (save a copy /
//! subscribe). The unauthenticated view/contribute surface lives in `api/user/public_view.rs`.

use crate::api::middleware::auth_user::AuthUser;
use crate::domain::public_share::{PublicPermissions, PublicShare};
use crate::services::shares::public::{self, PublicShareInput};
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::Json;
use axum::extract::{Path, State};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The owner-facing view of a public share (includes the secret `token` so the owner can copy the
/// link, and the derived-share / contribution counts for the management list).
#[derive(Debug, Serialize)]
pub struct PublicShareResponse {
    pub id: Uuid,
    pub tag_path: String,
    pub name: String,
    pub message: Option<String>,
    pub token: String,
    pub has_password: bool,
    pub expires_at: Option<NaiveDateTime>,
    pub permissions: PublicPermissions,
    pub status: crate::domain::public_share::PublicShareStatus,
    pub created_at: NaiveDateTime,
    pub revoked_at: Option<NaiveDateTime>,
    pub derived_share_count: i64,
    pub contribution_count: i64,
}

impl PublicShareResponse {
    fn from_share(s: PublicShare, derived: i64, contributions: i64) -> Self {
        Self {
            permissions: s.permissions(),
            has_password: s.password_hash.is_some(),
            id: s.id,
            tag_path: s.tag_path,
            name: s.name,
            message: s.message,
            token: s.token,
            expires_at: s.expires_at,
            status: s.status,
            created_at: s.created_at,
            revoked_at: s.revoked_at,
            derived_share_count: derived,
            contribution_count: contributions,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PublicShareBody {
    pub tag_path: String,
    pub name: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub expires_at: Option<NaiveDateTime>,
    #[serde(default)]
    pub allow_originals: bool,
    #[serde(default)]
    pub allow_upload: bool,
    #[serde(default)]
    pub allow_share_back: bool,
    #[serde(default)]
    pub conv_allow_exif_edit: bool,
    #[serde(default = "default_true")]
    pub conv_future: bool,
    /// PATCH only: keep the existing password hash (ignore `password`).
    #[serde(default)]
    pub keep_password: bool,
}

fn default_true() -> bool {
    true
}

impl PublicShareBody {
    fn into_input(self) -> PublicShareInput {
        PublicShareInput {
            tag_path: self.tag_path,
            name: self.name,
            message: self.message,
            password: self.password,
            expires_at: self.expires_at,
            allow_originals: self.allow_originals,
            allow_upload: self.allow_upload,
            allow_share_back: self.allow_share_back,
            conv_allow_exif_edit: self.conv_allow_exif_edit,
            conv_future: self.conv_future,
        }
    }
}

#[tracing::instrument(skip(auth, state, body), fields(user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<PublicShareBody>,
) -> Result<Json<PublicShareResponse>, AppError> {
    let share = public::create_public_share(
        &state.db,
        &state.settings,
        auth.user_id()?,
        body.into_input(),
    )
    .await?;
    Ok(Json(PublicShareResponse::from_share(share, 0, 0)))
}

#[tracing::instrument(skip(auth, state), fields(user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<PublicShareResponse>>, AppError> {
    let rows = public::list_public_shares_with_counts(&state.db, auth.user_id()?).await?;
    Ok(Json(
        rows.into_iter()
            .map(|(s, d, c)| PublicShareResponse::from_share(s, d, c))
            .collect(),
    ))
}

#[tracing::instrument(skip(auth, state, body), fields(user_id = %auth.claims.uid.unwrap_or_default(), share_id = %id))]
pub async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<PublicShareBody>,
) -> Result<Json<PublicShareResponse>, AppError> {
    let keep_password = body.keep_password;
    let share = public::update_public_share(
        &state.db,
        auth.user_id()?,
        id,
        body.into_input(),
        keep_password,
    )
    .await?;
    Ok(Json(PublicShareResponse::from_share(share, 0, 0)))
}

#[derive(Debug, Default, Deserialize)]
pub struct RevokeBody {
    #[serde(default)]
    pub cascade_derived: bool,
    #[serde(default)]
    pub trash_contributions: bool,
}

#[tracing::instrument(skip(auth, state, body), fields(user_id = %auth.claims.uid.unwrap_or_default(), share_id = %id))]
pub async fn revoke(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<RevokeBody>,
) -> Result<Json<public::PublicRevokeOutcome>, AppError> {
    let outcome = public::revoke_public_share(
        &state.db,
        state.cache.as_ref(),
        &state.federation,
        &state.settings,
        &state.routines.unannounce,
        &state.routines.pipeline,
        auth.user_id()?,
        &auth.claims.sub,
        id,
        body.cascade_derived,
        body.trash_contributions,
    )
    .await?;
    Ok(Json(outcome))
}

#[derive(Debug, Default, Deserialize)]
pub struct TrashContributionsBody {
    /// Restrict to a single contributor's `#name` (already sigil-prefixed). Omit to trash all.
    #[serde(default)]
    pub contributor: Option<String>,
}

#[tracing::instrument(skip(auth, state, body), fields(user_id = %auth.claims.uid.unwrap_or_default(), share_id = %id))]
pub async fn trash_contributions(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<TrashContributionsBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let owner_id = auth.user_id()?;
    let share = crate::repository::public_share::PublicShareRepository::find_by_id(&state.db, id)
        .await?
        .filter(|s| s.owner_id == owner_id)
        .ok_or(AppError::NotFound)?;
    let trashed = public::trash_contributions(
        &state.db,
        &state.routines.pipeline,
        owner_id,
        &share.tag_path,
        body.contributor.as_deref(),
    )
    .await?;
    Ok(Json(serde_json::json!({ "trashed": trashed })))
}

// ── Convert (logged-in visitor acting on another owner's share) ──────────────────

#[derive(Debug, Deserialize)]
pub struct SaveCopyBody {
    pub owner_username: String,
    pub owner_instance: String,
    pub token: String,
    pub picture_id: Uuid,
}

#[tracing::instrument(skip(auth, state, body), fields(user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn save_copy(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<SaveCopyBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let picture = public::public_save_copy(
        &state.db,
        state.cache.as_ref(),
        state.storage.as_ref(),
        &state.settings,
        &state.federation,
        &state.routines.pipeline,
        auth.user_id()?,
        &body.owner_username,
        &body.owner_instance,
        &body.token,
        body.picture_id,
    )
    .await?;
    Ok(Json(serde_json::json!({ "id": picture.id })))
}

#[derive(Debug, Deserialize)]
pub struct SubscribeBody {
    pub owner_username: String,
    pub owner_instance: String,
    pub token: String,
}

#[tracing::instrument(skip(auth, state, body), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn subscribe(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<SubscribeBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let meta = public::public_subscribe(
        &state.db,
        state.cache.as_ref(),
        &state.federation,
        &state.settings,
        &state.routines.pipeline,
        auth.user_id()?,
        &auth.claims.sub,
        &body.owner_username,
        &body.owner_instance,
        &body.token,
    )
    .await?;
    Ok(Json(serde_json::json!({
        "outgoing_share_id": meta.outgoing_share_id,
        "name": meta.name,
        "tag_path": meta.tag_path,
        "allow_share_back": meta.allow_share_back,
    })))
}
