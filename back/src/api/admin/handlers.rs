use crate::api::admin::models::{
    AdminJobResponse, AdminUserResponse, ConsistencyResponse, CreateUserRequest,
    ErroredShareResponse, FederationInstanceResponse, InstanceHealthResponse,
    InstanceStatsResponse, ListJobsQuery, UpdateUserRequest, UserStatsResponse,
};
use crate::api::middleware::auth_admin::AuthAdmin;
use crate::infra::error::AppError;
use crate::infra::redis::{RedisKey, cache_get_json, cache_set_json_ex};
use crate::repository::admin::AdminRepository;
use crate::repository::share::{IncomingShareRepository, OutgoingShareRepository};
use crate::repository::user::UserRepository;
use crate::services;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use uuid::Uuid;

const INSTANCE_STATS_TTL: u64 = 60;
const USER_STATS_TTL: u64 = 120;

// ── User management ───────────────────────────────────────────────────────────

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub))]
pub async fn list_users(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<AdminUserResponse>>, AppError> {
    let users = AdminRepository::list_users_with_storage(&state.db).await?;
    Ok(Json(
        users.into_iter().map(AdminUserResponse::from).collect(),
    ))
}

#[tracing::instrument(skip(_auth, state, payload), fields(user = %_auth.claims.sub, created_user = %payload.username))]
pub async fn create_user(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateUserRequest>,
) -> Result<Json<AdminUserResponse>, AppError> {
    let user = services::users::create_user(
        &state.db,
        &payload.username,
        &payload.email,
        &payload.display_name,
        &payload.password,
        payload.is_admin.unwrap_or(false),
    )
    .await?;
    Ok(Json(AdminUserResponse {
        id: user.id,
        username: user.username,
        email: user.email,
        display_name: user.display_name,
        is_admin: user.is_admin,
        storage_bytes: 0,
    }))
}

#[tracing::instrument(skip(_auth, state, payload), fields(user = %_auth.claims.sub, target_user_id = %user_id))]
pub async fn update_user(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Json(payload): Json<UpdateUserRequest>,
) -> Result<Json<AdminUserResponse>, AppError> {
    let user = UserRepository::update(
        &state.db,
        user_id,
        payload.display_name.as_deref(),
        payload.is_admin,
    )
    .await?;
    // Storage is not available without an extra query here; return 0 for update responses.
    Ok(Json(AdminUserResponse {
        id: user.id,
        username: user.username,
        email: user.email,
        display_name: user.display_name,
        is_admin: user.is_admin,
        storage_bytes: 0,
    }))
}

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub, target_user_id = %user_id))]
pub async fn delete_user(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    UserRepository::delete(&state.db, user_id).await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

// ── Instance health ───────────────────────────────────────────────────────────

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub))]
pub async fn get_instance(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<InstanceHealthResponse>, AppError> {
    let db_connected = sqlx::query_scalar!("SELECT 1 AS ping")
        .fetch_one(&state.db)
        .await
        .is_ok();

    let redis_connected = state
        .cache
        .get_str(RedisKey::UploadSession(Uuid::nil()))
        .await
        .is_ok();

    let last_worker_activity_at = AdminRepository::instance_stats(&state.db)
        .await
        .ok()
        .and_then(|s| {
            s.last_worker_activity_at
                .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        });

    Ok(Json(InstanceHealthResponse {
        global_domain: state.config.global_domain.clone(),
        back_domain: state.config.back_domain.clone(),
        db_connected,
        redis_connected,
        last_worker_activity_at,
    }))
}

// ── Instance-wide analytics (cached) ─────────────────────────────────────────

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub))]
pub async fn get_instance_stats(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<InstanceStatsResponse>, AppError> {
    if let Some(cached) =
        cache_get_json::<InstanceStatsResponse>(state.cache.as_ref(), RedisKey::AdminStats).await?
    {
        return Ok(Json(cached));
    }

    let stats = AdminRepository::instance_stats(&state.db).await?;
    let _ = cache_set_json_ex(
        state.cache.as_ref(),
        RedisKey::AdminStats,
        &stats,
        INSTANCE_STATS_TTL,
    )
    .await;
    Ok(Json(stats))
}

// ── Per-user analytics (cached) ───────────────────────────────────────────────

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub, target_user_id = %user_id))]
pub async fn get_user_stats(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
) -> Result<Json<UserStatsResponse>, AppError> {
    UserRepository::find_by_id(&state.db, user_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let key = RedisKey::AdminUserStats(user_id);
    if let Some(cached) = cache_get_json::<UserStatsResponse>(state.cache.as_ref(), key).await? {
        return Ok(Json(cached));
    }

    let stats = AdminRepository::user_stats(&state.db, user_id).await?;
    let _ = cache_set_json_ex(state.cache.as_ref(), key, &stats, USER_STATS_TTL).await;
    Ok(Json(stats))
}

// ── User shares ───────────────────────────────────────────────────────────────

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub, target_user_id = %user_id))]
pub async fn get_user_shares(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    UserRepository::find_by_id(&state.db, user_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let outgoing = OutgoingShareRepository::list_by_owner(&state.db, user_id).await?;
    let incoming = IncomingShareRepository::list_by_recipient(&state.db, user_id).await?;

    Ok(Json(serde_json::json!({
        "outgoing": outgoing,
        "incoming": incoming,
    })))
}

// ── Pipeline wake ─────────────────────────────────────────────────────────────

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub, target_user_id = %user_id))]
pub async fn wake_user_pipeline(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    UserRepository::find_by_id(&state.db, user_id)
        .await?
        .ok_or(AppError::NotFound)?;

    state.pipeline_waker.wake(user_id);
    Ok(Json(serde_json::json!({ "woken": true })))
}

// ── Bulk thumbnail / content-hash regeneration ─────────────────────────────────

/// Request body for `POST /api/admin/pictures/regenerate-thumbnails`.
#[derive(Debug, serde::Deserialize)]
pub struct RegenerateThumbnailsRequest {
    /// `"missing"` (default) — only owned pictures with a thumbnailable MIME, no thumbnail, older
    /// than 30 minutes (failed/never-run jobs). `"all"` — every owned picture (e.g. to recompute
    /// `content_hash` library-wide).
    #[serde(default)]
    pub scope: RegenScope,
    /// When `true`, the job also re-extracts EXIF from the file (`is_initial`); default `false` —
    /// recompute thumbnails/hashes/`content_hash` only, leaving stored EXIF untouched.
    #[serde(default)]
    pub reextract_exif: bool,
    /// Safety cap on how many jobs to enqueue in one call (1–100000, default 10000).
    pub limit: Option<i64>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegenScope {
    #[default]
    Missing,
    All,
}

#[tracing::instrument(skip(_auth, state, body), fields(user = %_auth.claims.sub))]
pub async fn regenerate_thumbnails(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Json(body): Json<RegenerateThumbnailsRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let only_missing = matches!(body.scope, RegenScope::Missing);
    let limit = body.limit.unwrap_or(10_000).clamp(1, 100_000);
    let enqueued =
        services::jobs::regenerate_thumbnails(&state.db, only_missing, body.reextract_exif, limit)
            .await?;
    // The jobs are owned per-picture; workers will pick them up by polling. No pipeline wake needed
    // (gen_thumbnail completion wakes the owner's pipeline for dedup/announce on its own).
    Ok(Json(serde_json::json!({ "enqueued": enqueued })))
}

// ── Job list ──────────────────────────────────────────────────────────────────

#[tracing::instrument(skip(_auth, state, query), fields(user = %_auth.claims.sub))]
pub async fn list_jobs(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Query(query): Query<ListJobsQuery>,
) -> Result<Json<Vec<AdminJobResponse>>, AppError> {
    let limit = query.limit.clamp(1, 200);
    let jobs = AdminRepository::list_jobs(
        &state.db,
        query.status,
        query.job_type,
        query.user_id,
        limit,
        query.offset,
    )
    .await?;
    Ok(Json(jobs))
}

// ── Stale jobs ────────────────────────────────────────────────────────────────

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub))]
pub async fn list_stale_jobs(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<AdminJobResponse>>, AppError> {
    let jobs =
        AdminRepository::list_stale_jobs(&state.db, state.config.job_processing_timeout_secs)
            .await?;
    Ok(Json(jobs))
}

// ── Job reset ─────────────────────────────────────────────────────────────────

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub, job_id = %job_id))]
pub async fn reset_job(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<AdminJobResponse>, AppError> {
    AdminRepository::reset_job(&state.db, job_id)
        .await?
        .ok_or(AppError::NotFound)
        .map(Json)
}

// ── Job cancel ────────────────────────────────────────────────────────────────

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub, job_id = %job_id))]
pub async fn cancel_job(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<AdminJobResponse>, AppError> {
    AdminRepository::cancel_job(&state.db, job_id)
        .await?
        .ok_or(AppError::NotFound)
        .map(Json)
}

// ── Errored shares (global) ───────────────────────────────────────────────────

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub))]
pub async fn list_errored_shares(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<ErroredShareResponse>>, AppError> {
    let shares = AdminRepository::list_errored_shares(&state.db).await?;
    Ok(Json(shares))
}

// ── Force-reconcile a share ───────────────────────────────────────────────────

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub, share_id = %share_id))]
pub async fn force_reconcile_share(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(share_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let owner_id = AdminRepository::clear_share_backoff(&state.db, share_id)
        .await?
        .ok_or(AppError::NotFound)?;

    state.pipeline_waker.wake(owner_id);
    Ok(Json(serde_json::json!({ "reconcile_triggered": true })))
}

// ── Active federation connections (Redis token cache) ─────────────────────────

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub))]
pub async fn list_active_federation_connections(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<String>>, AppError> {
    let keys = state.cache.scan_keys("federation:token:*").await?;
    const PREFIX: &str = "federation:token:";
    let mut domains: Vec<String> = keys
        .into_iter()
        .filter_map(|k| k.strip_prefix(PREFIX).map(str::to_string))
        .collect();
    domains.sort();
    Ok(Json(domains))
}

// ── Federation instances ──────────────────────────────────────────────────────

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub))]
pub async fn list_federation_instances(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<FederationInstanceResponse>>, AppError> {
    let instances = AdminRepository::list_federation_instances(&state.db).await?;
    Ok(Json(instances))
}

// ── Consistency check ─────────────────────────────────────────────────────────

#[tracing::instrument(skip(_auth, state), fields(user = %_auth.claims.sub))]
pub async fn get_consistency(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<ConsistencyResponse>, AppError> {
    let stats = AdminRepository::consistency_stats(&state.db).await?;
    Ok(Json(stats))
}
