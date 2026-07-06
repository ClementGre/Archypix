use crate::api::middleware::auth_user::AuthUser;
use crate::domain::segmentation::SegmentationConfig;
use crate::domain::tag::TagPath;
use crate::domain::tagging::{
    RuleItem, ServiceConfig, ServiceType, SharedMappingConfig, TaggingService,
};
use crate::repository::share::IncomingShareRepository;
use crate::repository::tag::TagRepository;
use crate::repository::tagging::TaggingServiceRepository;
use crate::services;
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

fn parse_tags_allowing_protected(paths: &[String]) -> Result<Vec<String>, AppError> {
    paths
        .iter()
        .map(|p| {
            TagPath::parse(p, true)
                .map(|t| t.as_ltree().to_string())
                .map_err(AppError::BadRequest)
        })
        .collect()
}

// ─── Response types ────────────────────────────────────────────────────────────

/// Flat service response (no config payload) — used by create and update.
#[derive(Debug, Serialize)]
pub struct ServiceResponse {
    pub id: Uuid,
    pub name: String,
    pub service_type: ServiceType,
    pub requires: Vec<String>,
    pub excludes: Vec<String>,
    pub enabled: bool,
    pub position: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Clone)]
pub struct RuleResponse {
    pub id: Uuid,
    /// Structured JSONB predicate tree (feature 13).
    pub predicate: serde_json::Value,
    pub assign_tag: String,
}

/// Tagged-enum response for a service with its type-specific config.
#[derive(Debug, Serialize)]
#[serde(tag = "service_type", rename_all = "snake_case")]
pub enum ServiceDetailResponse {
    SharedTagMapping {
        #[serde(flatten)]
        base: ServiceResponse,
        incoming_share_id: Uuid,
        assign_tags: Vec<String>,
        /// Derived (not stored): the referenced incoming share is absent or not active (§10.1).
        is_broken: bool,
    },
    Rule {
        #[serde(flatten)]
        base: ServiceResponse,
        rules: Vec<RuleResponse>,
    },
    Segmentation {
        #[serde(flatten)]
        base: ServiceResponse,
        config: SegmentationConfig,
    },
}

fn service_to_response(s: &TaggingService) -> ServiceResponse {
    ServiceResponse {
        id: s.id,
        name: s.name.clone(),
        service_type: s.service_type,
        requires: s.requires.clone(),
        excludes: s.excludes.clone(),
        enabled: s.enabled,
        position: s.position,
        created_at: s.created_at,
        updated_at: s.updated_at,
    }
}

/// Build the type-specific detail response. `active_shares` is the set of the user's active
/// incoming-share ids, used to derive `is_broken` for mapping services.
fn service_to_detail(svc: TaggingService, active_shares: &HashSet<Uuid>) -> ServiceDetailResponse {
    let base = service_to_response(&svc);
    match svc.service_type {
        ServiceType::SharedTagMapping => {
            let cfg = svc.mapping_config().unwrap_or(SharedMappingConfig {
                incoming_share_id: Uuid::nil(),
                assign_tags: vec![],
            });
            ServiceDetailResponse::SharedTagMapping {
                base,
                is_broken: !active_shares.contains(&cfg.incoming_share_id),
                incoming_share_id: cfg.incoming_share_id,
                assign_tags: cfg.assign_tags,
            }
        }
        ServiceType::Rule => ServiceDetailResponse::Rule {
            base,
            rules: svc
                .rule_config()
                .unwrap_or_default()
                .rules
                .into_iter()
                .map(|r: RuleItem| RuleResponse {
                    id: r.id,
                    predicate: r.predicate,
                    assign_tag: r.assign_tag,
                })
                .collect(),
        },
        ServiceType::Segmentation => ServiceDetailResponse::Segmentation {
            base,
            config: svc
                .segmentation_config()
                .unwrap_or_else(|_| empty_segmentation()),
        },
    }
}

fn empty_segmentation() -> SegmentationConfig {
    SegmentationConfig {
        version: 1,
        root_tag: "Photos".to_string(),
        hemisphere: Default::default(),
        catch_all: None,
        bands: vec![],
    }
}

async fn active_share_set(state: &AppState, user_id: Uuid) -> Result<HashSet<Uuid>, AppError> {
    Ok(
        IncomingShareRepository::active_ids_for_recipient(&state.db, user_id)
            .await?
            .into_iter()
            .collect(),
    )
}

/// Parse + validate a config for `service_type`, and (for mappings) check the referenced incoming
/// share belongs to the caller. Returns the normalized, storage-ready JSON.
async fn validate_config(
    state: &AppState,
    user_id: Uuid,
    service_type: ServiceType,
    raw: &serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    let config = ServiceConfig::parse(service_type, raw).map_err(AppError::BadRequest)?;
    if let ServiceConfig::SharedTagMapping(c) = &config {
        IncomingShareRepository::get_by_id(&state.db, c.incoming_share_id)
            .await?
            .filter(|s| s.recipient_id == user_id)
            .ok_or(AppError::NotFound)?;
    }
    Ok(config.to_value())
}

// ─── Service CRUD ──────────────────────────────────────────────────────────────

/// GET /tagging-services — list all services with their config, in pipeline execution order.
#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn list_services(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<ServiceDetailResponse>>, AppError> {
    let user_id = auth.user_id()?;
    let services = TaggingServiceRepository::list_by_owner(&state.db, user_id).await?;
    let active_shares = active_share_set(&state, user_id).await?;
    Ok(Json(
        services
            .into_iter()
            .map(|svc| service_to_detail(svc, &active_shares))
            .collect(),
    ))
}

#[derive(Debug, Deserialize)]
pub struct CreateServiceRequest {
    pub service_type: ServiceType,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub excludes: Vec<String>,
    /// Type-specific config (validated per `service_type`). Defaults to that type's empty config.
    #[serde(default)]
    pub config: Option<serde_json::Value>,
}

/// POST /tagging-services — create a service of any type with its initial config.
#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn create_service(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<CreateServiceRequest>,
) -> Result<Json<ServiceDetailResponse>, AppError> {
    let user_id = auth.user_id()?;
    let requires = parse_tags_allowing_protected(&payload.requires)?;
    let excludes = parse_tags_allowing_protected(&payload.excludes)?;

    let raw = payload
        .config
        .unwrap_or_else(|| default_config(payload.service_type));
    let config = validate_config(&state, user_id, payload.service_type, &raw).await?;

    let service = TaggingServiceRepository::create(
        &state.db,
        user_id,
        payload.service_type,
        payload.name.trim(),
        &requires,
        &excludes,
        &config,
    )
    .await?;
    // New service: last_invalidated_at = NOW() by default, so all existing pictures are dirty.
    state.routines.pipeline.trigger(user_id);

    let active_shares = active_share_set(&state, user_id).await?;
    Ok(Json(service_to_detail(service, &active_shares)))
}

/// The empty starting config for a type, used when `config` is omitted on create.
fn default_config(service_type: ServiceType) -> serde_json::Value {
    match service_type {
        ServiceType::Rule => serde_json::json!({ "rules": [] }),
        ServiceType::Segmentation => serde_json::to_value(empty_segmentation()).unwrap(),
        // No sensible default — a mapping needs an incoming share; force the caller to supply it.
        ServiceType::SharedTagMapping => serde_json::json!({}),
    }
}

/// GET /tagging-services/{id} — get a service with its config.
#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default(), service_id = %service_id))]
pub async fn get_service(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(service_id): Path<Uuid>,
) -> Result<Json<ServiceDetailResponse>, AppError> {
    let user_id = auth.user_id()?;
    let svc = TaggingServiceRepository::get_by_owner_and_id(&state.db, user_id, service_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let active_shares = active_share_set(&state, user_id).await?;
    Ok(Json(service_to_detail(svc, &active_shares)))
}

#[derive(Debug, Deserialize)]
pub struct UpdateServiceRequest {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub requires: Option<Vec<String>>,
    pub excludes: Option<Vec<String>>,
}

/// PATCH /tagging-services/{id} — update a service's name / enabled / gates.
#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default(), service_id = %service_id
))]
pub async fn update_service(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(service_id): Path<Uuid>,
    Json(payload): Json<UpdateServiceRequest>,
) -> Result<Json<ServiceResponse>, AppError> {
    let user_id = auth.user_id()?;
    let requires = payload
        .requires
        .as_deref()
        .map(parse_tags_allowing_protected)
        .transpose()?;
    let excludes = payload
        .excludes
        .as_deref()
        .map(parse_tags_allowing_protected)
        .transpose()?;
    let service = TaggingServiceRepository::update(
        &state.db,
        user_id,
        service_id,
        payload.name.as_deref().map(str::trim),
        payload.enabled,
        requires.as_deref(),
        excludes.as_deref(),
    )
    .await?
    .ok_or(AppError::NotFound)?;
    // Disabling a service makes its tags no longer live — drop them now. Re-enabling and any
    // other config change re-derives tags on the next pipeline run (via touch_invalidated).
    if payload.enabled == Some(false) {
        TagRepository::remove_service_tags(&state.db, service_id).await?;
    }
    TaggingServiceRepository::touch_invalidated(&state.db, service_id).await?;
    state.routines.pipeline.trigger(user_id);
    Ok(Json(service_to_response(&service)))
}

#[derive(Debug, Deserialize)]
pub struct ReplaceConfigRequest {
    /// The full type-specific config, validated against the service's stored type.
    pub config: serde_json::Value,
}

/// PUT /tagging-services/{id}/config — replace a service's whole type-specific config.
///
/// One uniform editing path for all three service types (rules / segmentation bands / mapping tags):
/// the array/band order in the submitted config *is* the stored order — there is no separate
/// reorder/add/remove sub-resource.
#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default(), service_id = %service_id
))]
pub async fn replace_config(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(service_id): Path<Uuid>,
    Json(payload): Json<ReplaceConfigRequest>,
) -> Result<Json<ServiceDetailResponse>, AppError> {
    let user_id = auth.user_id()?;
    let svc = TaggingServiceRepository::get_by_owner_and_id(&state.db, user_id, service_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let config = validate_config(&state, user_id, svc.service_type, &payload.config).await?;
    TaggingServiceRepository::set_config(&state.db, user_id, service_id, svc.service_type, &config)
        .await?;
    TaggingServiceRepository::touch_invalidated(&state.db, service_id).await?;
    state.routines.pipeline.trigger(user_id);

    let svc = TaggingServiceRepository::get_by_owner_and_id(&state.db, user_id, service_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let active_shares = active_share_set(&state, user_id).await?;
    Ok(Json(service_to_detail(svc, &active_shares)))
}

#[derive(Debug, Deserialize)]
pub struct DeleteServiceQuery {
    pub promote_tags: bool,
}

/// DELETE /tagging-services/{id} — delete a service. Its assigned tags are promoted to `manual`
/// (or removed) per `promote_tags`.
#[tracing::instrument(skip(auth, state, query), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default(), service_id = %service_id
))]
pub async fn delete_service(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(service_id): Path<Uuid>,
    Query(query): Query<DeleteServiceQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = auth.user_id()?;
    let deleted =
        services::tagging::delete_service(&state.db, user_id, service_id, query.promote_tags)
            .await?;
    if !deleted {
        return Err(AppError::NotFound);
    }
    state.routines.pipeline.trigger(user_id);
    Ok(Json(serde_json::json!({ "deleted": true })))
}

#[derive(Debug, Deserialize)]
pub struct ReorderServicesRequest {
    /// Ordered list of Rule and Segmentation service IDs. SharedTagMapping services are
    /// excluded — they always run first and cannot be reordered.
    pub ordered_ids: Vec<Uuid>,
}

/// POST /tagging-services/reorder — set the execution order of Rule and Segmentation services.
#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn reorder_services(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<ReorderServicesRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = auth.user_id()?;
    TaggingServiceRepository::reorder_services(&state.db, user_id, &payload.ordered_ids).await?;
    Ok(Json(serde_json::json!({ "reordered": true })))
}
