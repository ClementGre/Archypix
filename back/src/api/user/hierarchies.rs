use crate::api::middleware::auth_user::AuthUser;
use crate::domain::hierarchy::HierarchyConfig;
use crate::infra::error::AppError;
use crate::repository::hierarchy::HierarchyRow;
use crate::repository::picture::{PictureSortField, SortOrder};
use crate::services;
use crate::services::hierarchy::{BrowseParams, TreeResult};
use crate::services::pictures::{PictureListResult, ThumbnailSize};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
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

pub async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<CreateHierarchyRequest>,
) -> Result<Json<HierarchyDetail>, AppError> {
    debug!(user = %auth.claims.sub, token_type = auth.token_type(), name = %payload.name, "create_hierarchy");
    let config = payload.config.unwrap_or_else(|| {
        serde_json::to_value(HierarchyConfig::default()).expect("default config serializes")
    });
    let row =
        services::hierarchy::create_hierarchy(&state.db, auth.user_id()?, &payload.name, &config)
            .await?;
    Ok(Json(row.into()))
}

pub async fn get(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<HierarchyDetail>, AppError> {
    debug!(user = %auth.claims.sub, token_type = auth.token_type(), %id, "get_hierarchy");
    let row = services::hierarchy::get_hierarchy(&state.db, auth.user_id()?, id).await?;
    Ok(Json(row.into()))
}

#[derive(Debug, Deserialize)]
pub struct UpdateHierarchyRequest {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub config: Option<serde_json::Value>,
}

pub async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateHierarchyRequest>,
) -> Result<Json<HierarchyDetail>, AppError> {
    debug!(user = %auth.claims.sub, token_type = auth.token_type(), %id, "update_hierarchy");
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

pub async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    debug!(user = %auth.claims.sub, token_type = auth.token_type(), %id, "delete_hierarchy");
    let deleted = services::hierarchy::delete_hierarchy(&state.db, auth.user_id()?, id).await?;
    if !deleted {
        return Err(AppError::NotFound);
    }
    Ok(Json(serde_json::json!({ "deleted": true })))
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

pub async fn tree(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<TreeQuery>,
) -> Result<Json<TreeResponse>, AppError> {
    debug!(user = %auth.claims.sub, token_type = auth.token_type(), %id, path = %query.path, depth = query.depth, counts = query.counts, "hierarchy_tree");
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

pub async fn browse(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<BrowseQuery>,
) -> Result<Json<PictureListResult>, AppError> {
    debug!(user = %auth.claims.sub, token_type = auth.token_type(), %id, path = %query.path, "hierarchy_browse");
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
        &state.config,
        &state.federation,
        auth.user_id()?,
        id,
        &query.path,
        params,
    )
    .await?;
    Ok(Json(result))
}
