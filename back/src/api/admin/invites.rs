//! Admin invite management (feature 24): an admin sees **every** local invite (grouped by creator in
//! the UI) and may revoke any of them. In resolver mode invites live on the resolver, so the local
//! table is empty here and invites are managed from the fleet dashboard instead.

use crate::api::middleware::auth_admin::AuthAdmin;
use crate::api::user::invites::InviteResponse;
use crate::repository::invite::InviteRepository;
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::extract::{Path, State};
use axum::Json;

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub))]
pub async fn list_invites(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<InviteResponse>>, AppError> {
    let invites = InviteRepository::list(&state.db).await?;
    Ok(Json(invites.into_iter().map(InviteResponse::from).collect()))
}

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub, code = %code))]
pub async fn revoke_invite(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    InviteRepository::delete(&state.db, &code).await?;
    Ok(Json(serde_json::json!({ "revoked": true })))
}
