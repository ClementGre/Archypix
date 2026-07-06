use crate::api::middleware::auth_user::AuthUser;
use crate::domain::hierarchy::HierarchyConfig;
use crate::repository::hierarchy::HierarchyRow;
use crate::repository::picture::{PictureSortField, SortOrder};
use crate::services;
use crate::services::hierarchy::{BrowseParams, TreeResult};
use crate::services::pictures::{PictureListResult, ThumbnailSize};
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::debug;
use uuid::Uuid;

// ─── Response models ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct HierarchySummary {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct HierarchyDetail {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub config: serde_json::Value,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl From<HierarchyRow> for HierarchySummary {
    fn from(r: HierarchyRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            enabled: r.enabled,
        }
    }
}

impl From<HierarchyRow> for HierarchyDetail {
    fn from(r: HierarchyRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            enabled: r.enabled,
            config: r.config,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TreeResponse {
    pub path: String,
    pub directories: Vec<services::hierarchy::TreeEntry>,
}

impl From<TreeResult> for TreeResponse {
    fn from(r: TreeResult) -> Self {
        Self {
            path: r.path,
            directories: r.directories,
        }
    }
}

// ─── CRUD ────────────────────────────────────────────────────────────────────────

#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<HierarchySummary>>, AppError> {
    debug!(user = %auth.claims.sub, token_type = auth.token_type(), "list_hierarchies");
    let rows = services::hierarchy::list_hierarchies(&state.db, auth.user_id()?).await?;
    Ok(Json(rows.into_iter().map(HierarchySummary::from).collect()))
}

#[derive(Debug, Deserialize)]
pub struct CreateHierarchyRequest {
    pub name: String,
    /// Full `config` JSONB (§4.1). Defaults to an empty node tree when omitted.
    pub config: Option<serde_json::Value>,
}

#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<CreateHierarchyRequest>,
) -> Result<Json<HierarchyDetail>, AppError> {
    let config = payload.config.unwrap_or_else(|| {
        serde_json::to_value(HierarchyConfig::default()).expect("default config serializes")
    });
    let row =
        services::hierarchy::create_hierarchy(&state.db, auth.user_id()?, &payload.name, &config)
            .await?;
    Ok(Json(row.into()))
}

#[tracing::instrument(skip(auth, state), fields(user_id = %auth.claims.uid.unwrap_or_default(), hierarchy_id = %id))]
pub async fn get(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<HierarchyDetail>, AppError> {
    let row = services::hierarchy::get_hierarchy(&state.db, auth.user_id()?, id).await?;
    Ok(Json(row.into()))
}

#[derive(Debug, Deserialize)]
pub struct UpdateHierarchyRequest {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub config: Option<serde_json::Value>,
}

#[tracing::instrument(skip(auth, state, payload), fields(user_id = %auth.claims.uid.unwrap_or_default(), hierarchy_id = %id))]
pub async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateHierarchyRequest>,
) -> Result<Json<HierarchyDetail>, AppError> {
    let row = services::hierarchy::update_hierarchy(
        &state.db,
        auth.user_id()?,
        id,
        payload.name.as_deref(),
        payload.enabled,
        payload.config.as_ref(),
    )
    .await?;
    Ok(Json(row.into()))
}

#[tracing::instrument(skip(auth, state), fields(user_id = %auth.claims.uid.unwrap_or_default(), hierarchy_id = %id))]
pub async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let deleted = services::hierarchy::delete_hierarchy(&state.db, auth.user_id()?, id).await?;
    if !deleted {
        return Err(AppError::NotFound);
    }
    Ok(Json(serde_json::json!({ "deleted": true })))
}

// ─── WebDAV token management (06_webdav.md §17) ────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct WebdavResponse {
    pub url: String,
    pub token: String,
    pub use_redirect: bool,
    pub enabled: bool,
}

impl From<services::hierarchy::WebdavInfo> for WebdavResponse {
    fn from(i: services::hierarchy::WebdavInfo) -> Self {
        Self {
            url: i.url,
            token: i.token,
            use_redirect: i.use_redirect,
            enabled: i.enabled,
        }
    }
}

/// `GET /{id}/webdav` — the mount URL + token (minted on first access).
#[tracing::instrument(skip(auth, state), fields(user_id = %auth.claims.uid.unwrap_or_default(), hierarchy_id = %id))]
pub async fn webdav_get(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<WebdavResponse>, AppError> {
    let info =
        services::hierarchy::get_webdav_info(&state.db, &state.settings, auth.user_id()?, id)
            .await?;
    Ok(Json(info.into()))
}

/// `POST /{id}/webdav/regenerate` — rotate the token.
#[tracing::instrument(skip(auth, state), fields(user_id = %auth.claims.uid.unwrap_or_default(), hierarchy_id = %id))]
pub async fn webdav_regenerate(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<WebdavResponse>, AppError> {
    let info = services::hierarchy::regenerate_webdav_token(
        &state.db,
        &state.settings,
        auth.user_id()?,
        id,
    )
        .await?;
    Ok(Json(info.into()))
}

#[derive(Debug, Deserialize)]
pub struct WebdavPatchRequest {
    pub use_redirect: bool,
}

/// `PATCH /{id}/webdav` — toggle the read strategy.
#[tracing::instrument(skip(auth, state, payload), fields(user_id = %auth.claims.uid.unwrap_or_default(), hierarchy_id = %id))]
pub async fn webdav_patch(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<WebdavPatchRequest>,
) -> Result<Json<WebdavResponse>, AppError> {
    let user_id = auth.user_id()?;
    services::hierarchy::set_webdav_use_redirect(&state.db, user_id, id, payload.use_redirect)
        .await?;
    let info =
        services::hierarchy::get_webdav_info(&state.db, &state.settings, user_id, id).await?;
    Ok(Json(info.into()))
}

// ─── Navigation ──────────────────────────────────────────────────────────────────

fn default_depth() -> u32 {
    1
}

#[derive(Debug, Deserialize)]
pub struct TreeQuery {
    #[serde(default)]
    pub path: String,
    #[serde(default = "default_depth")]
    pub depth: u32,
    #[serde(default)]
    pub counts: bool,
}

#[tracing::instrument(skip(auth, state, query), fields(user_id = %auth.claims.uid.unwrap_or_default(), hierarchy_id = %id))]
pub async fn tree(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<TreeQuery>,
) -> Result<Json<TreeResponse>, AppError> {
    let result = services::hierarchy::resolve_tree(
        &state.db,
        auth.user_id()?,
        id,
        &query.path,
        query.depth,
        query.counts,
    )
    .await?;
    Ok(Json(result.into()))
}

fn default_page() -> u32 {
    1
}
fn default_page_size() -> u32 {
    50
}

#[derive(Debug, Deserialize)]
pub struct BrowseQuery {
    #[serde(default)]
    pub path: String,
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
    #[serde(default)]
    pub sort: PictureSortField,
    #[serde(default)]
    pub order: SortOrder,
    #[serde(default)]
    pub include_deleted: bool,
    #[serde(default)]
    pub owned_only: bool,
    #[serde(default)]
    pub shared_with_me: bool,
    pub captured_after: Option<DateTime<Utc>>,
    pub captured_before: Option<DateTime<Utc>>,
    pub thumbnail: Option<ThumbnailSize>,
}

#[tracing::instrument(skip(auth, state, query), fields(user_id = %auth.claims.uid.unwrap_or_default(), hierarchy_id = %id))]
pub async fn browse(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<BrowseQuery>,
) -> Result<Json<PictureListResult>, AppError> {
    let params = BrowseParams {
        page: query.page,
        page_size: query.page_size,
        sort: query.sort,
        order: query.order,
        include_deleted: query.include_deleted,
        owned_only: query.owned_only,
        shared_with_me: query.shared_with_me,
        captured_after: query.captured_after,
        captured_before: query.captured_before,
        thumbnail: query.thumbnail,
    };
    let result = services::hierarchy::browse(
        &state.db,
        state.cache.as_ref(),
        state.storage.as_ref(),
        &state.settings,
        &state.federation,
        auth.user_id()?,
        id,
        &query.path,
        params,
    )
    .await?;
    Ok(Json(result))
}
