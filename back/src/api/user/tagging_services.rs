use crate::api::middleware::auth_user::AuthUser;
use crate::domain::pipeline::Predicate;
use crate::domain::tag::TagPath;
use crate::domain::tagging::{
    RuleTaggingRule, SegmentationRule, ServiceType, SharedTagMappingRule, TaggingService,
};
use crate::infra::error::AppError;
use crate::repository::tag::TagRepository;
use crate::repository::tagging::{
    RuleTaggingRuleRepository, SegmentationRuleRepository, SharedTagMappingRuleRepository,
    TaggingServiceRepository,
};
use crate::services;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

fn parse_tag(raw: &str, allow_protected: bool) -> Result<String, AppError> {
    TagPath::parse(raw, allow_protected)
        .map(|p| p.as_ltree().to_string())
        .map_err(AppError::BadRequest)
}
fn parse_tags_allowing_protected(paths: &[String]) -> Result<Vec<String>, AppError> {
    paths.iter().map(|p| parse_tag(p, true)).collect()
}

// ─── Response types ────────────────────────────────────────────────────────────

/// Flat service response (no rules) — used by create and update.
#[derive(Debug, Serialize)]
pub struct ServiceResponse {
    pub id: Uuid,
    pub service_type: ServiceType,
    pub requires: Vec<String>,
    pub excludes: Vec<String>,
    pub enabled: bool,
    pub position: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Clone)]
pub struct SharedTagMappingRuleResponse {
    pub id: Uuid,
    pub incoming_share_id: Uuid,
    pub assign_tag: String,
    pub is_broken: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct RuleTaggingRuleResponse {
    pub id: Uuid,
    pub predicate: String,
    pub assign_tag: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct SegmentationRuleResponse {
    pub id: Uuid,
    pub name: String,
    pub date_start: NaiveDateTime,
    pub date_end: NaiveDateTime,
    pub assign_tag: String,
    pub parent_segment_id: Option<Uuid>,
}

/// Tagged-enum response for a service with its rules.
///
/// A service has exactly one rule type; the `service_type` discriminator tells
/// the caller which field is present.
#[derive(Debug, Serialize)]
#[serde(tag = "service_type", rename_all = "snake_case")]
pub enum ServiceDetailResponse {
    SharedTagMapping {
        id: Uuid,
        requires: Vec<String>,
        excludes: Vec<String>,
        enabled: bool,
        position: i32,
        created_at: NaiveDateTime,
        updated_at: NaiveDateTime,
        mappings: Vec<SharedTagMappingRuleResponse>,
    },
    Rule {
        id: Uuid,
        requires: Vec<String>,
        excludes: Vec<String>,
        enabled: bool,
        position: i32,
        created_at: NaiveDateTime,
        updated_at: NaiveDateTime,
        rules: Vec<RuleTaggingRuleResponse>,
    },
    Segmentation {
        id: Uuid,
        requires: Vec<String>,
        excludes: Vec<String>,
        enabled: bool,
        position: i32,
        created_at: NaiveDateTime,
        updated_at: NaiveDateTime,
        segments: Vec<SegmentationRuleResponse>,
    },
}

// ─── Converters ────────────────────────────────────────────────────────────────

fn service_to_response(s: TaggingService) -> ServiceResponse {
    ServiceResponse {
        id: s.id,
        service_type: s.service_type,
        requires: s.requires,
        excludes: s.excludes,
        enabled: s.enabled,
        position: s.position,
        created_at: s.created_at,
        updated_at: s.updated_at,
    }
}

fn mapping_to_response(r: SharedTagMappingRule) -> SharedTagMappingRuleResponse {
    SharedTagMappingRuleResponse {
        id: r.id,
        incoming_share_id: r.incoming_share_id,
        assign_tag: r.assign_tag,
        is_broken: r.is_broken,
    }
}

fn rule_to_response(r: RuleTaggingRule) -> RuleTaggingRuleResponse {
    RuleTaggingRuleResponse {
        id: r.id,
        predicate: r.predicate,
        assign_tag: r.assign_tag,
    }
}

fn segment_to_response(r: SegmentationRule) -> SegmentationRuleResponse {
    SegmentationRuleResponse {
        id: r.id,
        name: r.name,
        date_start: r.date_start,
        date_end: r.date_end,
        assign_tag: r.assign_tag,
        parent_segment_id: r.parent_segment_id,
    }
}

// ─── Service CRUD ──────────────────────────────────────────────────────────────

/// GET /pipeline — list all services for the user with their rules.
///
/// Groups service IDs by type and queries only the relevant rule table for
/// each group — no cross-type fetches.
#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn list_services(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<ServiceDetailResponse>>, AppError> {
    let user_id = auth.user_id()?;
    let services = TaggingServiceRepository::list_by_owner(&state.db, user_id).await?;

    // Partition IDs by type so we only query the tables we need.
    let mut mapping_ids: Vec<Uuid> = vec![];
    let mut rule_ids: Vec<Uuid> = vec![];
    let mut segment_ids: Vec<Uuid> = vec![];
    for svc in &services {
        match svc.service_type {
            ServiceType::SharedTagMapping => mapping_ids.push(svc.id),
            ServiceType::Rule => rule_ids.push(svc.id),
            ServiceType::Segmentation => segment_ids.push(svc.id),
        }
    }

    let (all_mappings, all_rules, all_segments) = tokio::try_join!(
        SharedTagMappingRuleRepository::list_for_services(&state.db, &mapping_ids),
        RuleTaggingRuleRepository::list_for_services(&state.db, &rule_ids),
        SegmentationRuleRepository::list_for_services(&state.db, &segment_ids),
    )?;

    let responses = services
        .into_iter()
        .map(|svc| {
            let id = svc.id;
            let (requires, excludes, enabled, position, created_at, updated_at) = (
                svc.requires.clone(),
                svc.excludes.clone(),
                svc.enabled,
                svc.position,
                svc.created_at,
                svc.updated_at,
            );
            match svc.service_type {
                ServiceType::SharedTagMapping => ServiceDetailResponse::SharedTagMapping {
                    id,
                    requires,
                    excludes,
                    enabled,
                    position,
                    created_at,
                    updated_at,
                    mappings: all_mappings
                        .iter()
                        .filter(|r| r.service_id == id)
                        .cloned()
                        .map(mapping_to_response)
                        .collect(),
                },
                ServiceType::Rule => ServiceDetailResponse::Rule {
                    id,
                    requires,
                    excludes,
                    enabled,
                    position,
                    created_at,
                    updated_at,
                    rules: all_rules
                        .iter()
                        .filter(|r| r.service_id == id)
                        .cloned()
                        .map(rule_to_response)
                        .collect(),
                },
                ServiceType::Segmentation => ServiceDetailResponse::Segmentation {
                    id,
                    requires,
                    excludes,
                    enabled,
                    position,
                    created_at,
                    updated_at,
                    segments: all_segments
                        .iter()
                        .filter(|r| r.service_id == id)
                        .cloned()
                        .map(segment_to_response)
                        .collect(),
                },
            }
        })
        .collect();

    Ok(Json(responses))
}

#[derive(Debug, Deserialize)]
pub struct CreateServiceRequest {
    pub service_type: ServiceType,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub excludes: Vec<String>,
}

/// POST /pipeline — create a new tagging service.
#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn create_service(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<CreateServiceRequest>,
) -> Result<Json<ServiceResponse>, AppError> {
    let user_id = auth.user_id()?;
    let requires = parse_tags_allowing_protected(&payload.requires)?;
    let excludes = parse_tags_allowing_protected(&payload.excludes)?;
    let service = TaggingServiceRepository::create(
        &state.db,
        user_id,
        payload.service_type,
        &requires,
        &excludes,
    )
    .await?;
    // New service: last_invalidated_at = NOW() by default, so all existing pictures are dirty.
    state.pipeline_waker.wake(auth.user_id()?);
    Ok(Json(service_to_response(service)))
}

/// GET /pipeline/{id} — get a service with its rules.
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

    let detail = match svc.service_type {
        ServiceType::SharedTagMapping => {
            let mappings =
                SharedTagMappingRuleRepository::list_for_services(&state.db, &[svc.id]).await?;
            ServiceDetailResponse::SharedTagMapping {
                id: svc.id,
                requires: svc.requires,
                excludes: svc.excludes,
                enabled: svc.enabled,
                position: svc.position,
                created_at: svc.created_at,
                updated_at: svc.updated_at,
                mappings: mappings.into_iter().map(mapping_to_response).collect(),
            }
        }
        ServiceType::Rule => {
            let rules = RuleTaggingRuleRepository::list_for_services(&state.db, &[svc.id]).await?;
            ServiceDetailResponse::Rule {
                id: svc.id,
                requires: svc.requires,
                excludes: svc.excludes,
                enabled: svc.enabled,
                position: svc.position,
                created_at: svc.created_at,
                updated_at: svc.updated_at,
                rules: rules.into_iter().map(rule_to_response).collect(),
            }
        }
        ServiceType::Segmentation => {
            let segments =
                SegmentationRuleRepository::list_for_services(&state.db, &[svc.id]).await?;
            ServiceDetailResponse::Segmentation {
                id: svc.id,
                requires: svc.requires,
                excludes: svc.excludes,
                enabled: svc.enabled,
                position: svc.position,
                created_at: svc.created_at,
                updated_at: svc.updated_at,
                segments: segments.into_iter().map(segment_to_response).collect(),
            }
        }
    };

    Ok(Json(detail))
}

#[derive(Debug, Deserialize)]
pub struct UpdateServiceRequest {
    pub enabled: Option<bool>,
    pub requires: Option<Vec<String>>,
    pub excludes: Option<Vec<String>>,
}

/// PATCH /pipeline/{id} — update a service.
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
    state.pipeline_waker.wake(auth.user_id()?);
    Ok(Json(service_to_response(service)))
}

#[derive(Debug, Deserialize)]
pub struct DeleteServiceQuery {
    pub promote_tags: bool,
}

/// DELETE /pipeline/{id} — delete a service (cascades to all its rules).
///
/// The tags this service assigned are promoted to `manual` so the user keeps them.
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
    state.pipeline_waker.wake(user_id);
    Ok(Json(serde_json::json!({ "deleted": true })))
}

// ─── Mapping rules (shared_tag_mapping services) ───────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AddMappingRequest {
    pub incoming_share_id: Uuid,
    pub assign_tag: String,
}

/// POST /pipeline/{id}/mappings — add a mapping rule to a SharedTagMapping service.
#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default(), service_id = %service_id
))]
pub async fn add_mapping(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(service_id): Path<Uuid>,
    Json(payload): Json<AddMappingRequest>,
) -> Result<Json<SharedTagMappingRuleResponse>, AppError> {
    let user_id = auth.user_id()?;
    let svc = TaggingServiceRepository::get_by_owner_and_id(&state.db, user_id, service_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if svc.service_type != ServiceType::SharedTagMapping {
        return Err(AppError::BadRequest(
            "service is not a shared_tag_mapping type".into(),
        ));
    }
    let assign_tag = parse_tag(&payload.assign_tag, false)?;
    let rule = SharedTagMappingRuleRepository::create(
        &state.db,
        service_id,
        payload.incoming_share_id,
        &assign_tag,
    )
    .await?;
    TaggingServiceRepository::touch_invalidated(&state.db, service_id).await?;
    state.pipeline_waker.wake(auth.user_id()?);
    Ok(Json(mapping_to_response(rule)))
}

/// DELETE /pipeline/{id}/mappings/{rule_id} — remove a mapping rule.
#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default(), service_id = %service_id, rule_id = %rule_id
))]
pub async fn delete_mapping(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((service_id, rule_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = auth.user_id()?;
    let deleted =
        SharedTagMappingRuleRepository::delete(&state.db, user_id, service_id, rule_id).await?;
    if !deleted {
        return Err(AppError::NotFound);
    }
    TaggingServiceRepository::touch_invalidated(&state.db, service_id).await?;
    state.pipeline_waker.wake(auth.user_id()?);
    Ok(Json(serde_json::json!({ "deleted": true })))
}

// ─── Rule tagging rules ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AddRuleRequest {
    pub predicate: String,
    pub assign_tag: String,
}

/// POST /pipeline/{id}/rules — add a rule to a Rule tagging service.
#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default(), service_id = %service_id
))]
pub async fn add_rule(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(service_id): Path<Uuid>,
    Json(payload): Json<AddRuleRequest>,
) -> Result<Json<RuleTaggingRuleResponse>, AppError> {
    let user_id = auth.user_id()?;
    let svc = TaggingServiceRepository::get_by_owner_and_id(&state.db, user_id, service_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if svc.service_type != ServiceType::Rule {
        return Err(AppError::BadRequest("service is not a rule type".into()));
    }
    let assign_tag = parse_tag(&payload.assign_tag, false)?;
    // Validate predicate syntax before persisting.
    Predicate::parse(&payload.predicate).map_err(AppError::BadRequest)?;
    let rule =
        RuleTaggingRuleRepository::create(&state.db, service_id, &payload.predicate, &assign_tag)
            .await?;
    TaggingServiceRepository::touch_invalidated(&state.db, service_id).await?;
    state.pipeline_waker.wake(auth.user_id()?);
    Ok(Json(rule_to_response(rule)))
}

/// DELETE /pipeline/{id}/rules/{rule_id} — remove a rule.
#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default(), service_id = %service_id, rule_id = %rule_id
))]
pub async fn delete_rule(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((service_id, rule_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = auth.user_id()?;
    let deleted =
        RuleTaggingRuleRepository::delete(&state.db, user_id, service_id, rule_id).await?;
    if !deleted {
        return Err(AppError::NotFound);
    }
    TaggingServiceRepository::touch_invalidated(&state.db, service_id).await?;
    state.pipeline_waker.wake(auth.user_id()?);
    Ok(Json(serde_json::json!({ "deleted": true })))
}

// ─── Segmentation rules ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AddSegmentRequest {
    pub name: String,
    pub date_start: NaiveDateTime,
    pub date_end: NaiveDateTime,
    pub assign_tag: String,
    pub parent_segment_id: Option<Uuid>,
}

/// POST /pipeline/{id}/segments — add a segment to a Segmentation service.
#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default(), service_id = %service_id
))]
pub async fn add_segment(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(service_id): Path<Uuid>,
    Json(payload): Json<AddSegmentRequest>,
) -> Result<Json<SegmentationRuleResponse>, AppError> {
    let user_id = auth.user_id()?;
    let svc = TaggingServiceRepository::get_by_owner_and_id(&state.db, user_id, service_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if svc.service_type != ServiceType::Segmentation {
        return Err(AppError::BadRequest(
            "service is not a segmentation type".into(),
        ));
    }
    if payload.date_end <= payload.date_start {
        return Err(AppError::BadRequest(
            "date_end must be after date_start".into(),
        ));
    }
    let assign_tag = parse_tag(&payload.assign_tag, false)?;
    let segment = SegmentationRuleRepository::create(
        &state.db,
        service_id,
        &payload.name,
        payload.date_start,
        payload.date_end,
        &assign_tag,
        payload.parent_segment_id,
    )
    .await?;
    TaggingServiceRepository::touch_invalidated(&state.db, service_id).await?;
    state.pipeline_waker.wake(auth.user_id()?);
    Ok(Json(segment_to_response(segment)))
}

/// DELETE /pipeline/{id}/segments/{segment_id} — remove a segment.
#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default(), service_id = %service_id, segment_id = %segment_id
))]
pub async fn delete_segment(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((service_id, segment_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = auth.user_id()?;
    let deleted =
        SegmentationRuleRepository::delete(&state.db, user_id, service_id, segment_id).await?;
    if !deleted {
        return Err(AppError::NotFound);
    }
    TaggingServiceRepository::touch_invalidated(&state.db, service_id).await?;
    state.pipeline_waker.wake(auth.user_id()?);
    Ok(Json(serde_json::json!({ "deleted": true })))
}

// ─── Reorder ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ReorderServicesRequest {
    /// Ordered list of Rule and Segmentation service IDs. SharedTagMapping services are
    /// excluded — they always run first and cannot be reordered.
    pub ordered_ids: Vec<Uuid>,
}

/// POST /tagging-services/reorder — set the execution order of Rule and Segmentation services.
///
/// The caller sends the complete desired ordering as a list of service IDs. Each service
/// gets `position = its index` in the list. SharedTagMapping services must not be included.
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
