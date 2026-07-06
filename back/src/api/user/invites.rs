//! Invite management + the invitation graph (feature 23 §6). Minting is gated by the current
//! registration mode (`open`/`invite` → any user; `admin_invite` → admins only).

use crate::api::middleware::auth_user::AuthUser;
use crate::infra::settings::keys;
use crate::repository::invite::InviteRepository;
use crate::state::AppState;
use archypix_common::error::AppError;
use archypix_common::registration::{generate_invite_code, Invite, RegistrationMode};
use axum::extract::{Path, State};
use axum::Json;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct MintInviteRequest {
    /// `Some(0)` = unlimited invitation, `Some(n)` = capped. Ignored in open mode (a single tracking
    /// referral link is minted instead).
    pub max_uses: Option<i64>,
    /// `None` = never expires. Ignored in open mode (referral links never expire).
    pub expires_in_days: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct InviteResponse {
    pub code: String,
    pub max_uses: Option<i64>,
    pub uses: i64,
    pub expires_at: Option<chrono::DateTime<Utc>>,
    pub created_by: String,
}

impl From<Invite> for InviteResponse {
    fn from(i: Invite) -> Self {
        Self {
            code: i.code,
            max_uses: i.max_uses,
            uses: i.uses,
            expires_at: i.expires_at,
            created_by: i.created_by,
        }
    }
}

fn current_mode(state: &AppState) -> RegistrationMode {
    state.settings.get(keys::REGISTRATION_MODE)
}

fn use_resolver(state: &AppState) -> bool {
    state.settings.get(keys::USE_RESOLVER)
}

/// List the caller's own invites (from the resolver in resolver mode, else the local table).
async fn user_invites(state: &AppState, user: &str) -> Result<Vec<Invite>, AppError> {
    if use_resolver(state) {
        state.resolver.list_invites(user).await
    } else {
        InviteRepository::list_by(&state.db, user).await
    }
}

/// The effective registration mode where this backend's signups land: the resolver's when behind one
/// (its `registration_mode` is authoritative), else the backend's own (standalone).
async fn effective_mode(state: &AppState) -> Result<RegistrationMode, AppError> {
    if use_resolver(state) {
        state.resolver.registration_mode().await
    } else {
        Ok(current_mode(state))
    }
}

#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub))]
pub async fn mint_invite(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<MintInviteRequest>,
) -> Result<Json<InviteResponse>, AppError> {
    let mode = effective_mode(&state).await?;
    if !mode.can_mint(auth.claims.is_admin) {
        return Err(AppError::Forbidden(
            "You are not allowed to mint invites in the current registration mode".to_string(),
        ));
    }

    // Open mode ⇒ a single, never-expiring **tracking referral link** per user (feature 23 §6). Gated
    // modes ⇒ a proper invitation (capped/uncapped, optional expiry).
    let (max_uses, expires_at) = if mode == RegistrationMode::Open {
        // One referral per user: return the existing one instead of minting duplicates.
        if let Some(existing) = user_invites(&state, &auth.claims.sub)
            .await?
            .into_iter()
            .find(|i| i.is_tracking())
        {
            return Ok(Json(existing.into()));
        }
        (None, None)
    } else {
        let expires_at = payload
            .expires_in_days
            .filter(|d| *d > 0)
            .map(|d| Utc::now() + Duration::days(d));
        (payload.max_uses.or(Some(0)), expires_at)
    };

    let invite = if use_resolver(&state) {
        // Invites live in the resolver's DB (it handles registration); pin the invitee to this backend.
        let back_domain = state.settings.get(keys::BACK_DOMAIN);
        state
            .resolver
            .create_invite(&auth.claims.sub, max_uses, expires_at, Some(&back_domain))
            .await?
    } else {
        InviteRepository::create(
            &state.db,
            &generate_invite_code(),
            max_uses,
            expires_at,
            &auth.claims.sub,
        )
            .await?
    };
    Ok(Json(invite.into()))
}

#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub))]
pub async fn list_invites(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<InviteResponse>>, AppError> {
    // Each user manages only their own invites here; the fleet operator uses the resolver dashboard.
    let invites = user_invites(&state, &auth.claims.sub).await?;
    Ok(Json(invites.into_iter().map(InviteResponse::from).collect()))
}

#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub, code = %code))]
pub async fn revoke_invite(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Only the minter may revoke — verify ownership against the caller's own invite list.
    let owned = user_invites(&state, &auth.claims.sub)
        .await?
        .iter()
        .any(|i| i.code == code);
    if !owned {
        return Err(AppError::Forbidden("Not your invite".to_string()));
    }
    if use_resolver(&state) {
        state.resolver.delete_invite(&code).await?;
    } else {
        InviteRepository::delete(&state.db, &code).await?;
    }
    Ok(Json(serde_json::json!({ "revoked": true })))
}

// ── Public invite preview (register page) ────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct InvitePreview {
    /// Whether the code is currently redeemable (unknown/expired/exhausted → false).
    pub valid: bool,
    /// The inviter's username (shown in the "X invited you" message), if the code is known.
    pub invited_by: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RegistrationInfo {
    /// The effective registration mode enforced where signups land (`open`/`invite`/`admin_invite`).
    pub mode: RegistrationMode,
}

/// `GET /api/public/registration-info` — lets the register/profile UIs adapt (invite required? tracking
/// invites?) without admin auth. Standalone-only; in resolver mode the resolver serves this path.
#[tracing::instrument(skip(state))]
pub async fn registration_info(State(state): State<AppState>) -> Json<RegistrationInfo> {
    Json(RegistrationInfo {
        mode: current_mode(&state),
    })
}

/// `GET /api/public/invites/{code}` — unauthenticated preview so the register page can show
/// "X invited you to join …". Standalone-only; in resolver mode the resolver serves this path.
#[tracing::instrument(skip(state), fields(code = %code))]
pub async fn preview_invite(
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> Result<Json<InvitePreview>, AppError> {
    let invite = InviteRepository::find(&state.db, &code).await?;
    let mode = current_mode(&state);
    Ok(Json(match invite {
        Some(i) => InvitePreview {
            valid: i.is_active(mode, Utc::now()),
            invited_by: Some(i.created_by),
        },
        None => InvitePreview {
            valid: false,
            invited_by: None,
        },
    }))
}

#[derive(Debug, Serialize)]
pub struct InvitationGraph {
    /// Username who invited the caller (if any).
    pub invited_by: Option<String>,
    /// Usernames the caller has invited.
    pub invited: Vec<String>,
}

/// `GET /me/invitations` — the caller's social/admin invite trace (feature 23 §6.3).
#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub))]
pub async fn my_invitations(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<InvitationGraph>, AppError> {
    let invited_by = sqlx::query_scalar!(
        "SELECT invited_by FROM users WHERE username = $1",
        auth.claims.sub
    )
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?
        .flatten();

    let invited = sqlx::query_scalar!(
        "SELECT username FROM users WHERE invited_by = $1 ORDER BY created_at",
        auth.claims.sub
    )
        .fetch_all(&state.db)
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;

    Ok(Json(InvitationGraph {
        invited_by,
        invited,
    }))
}
