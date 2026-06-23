use crate::api::middleware::auth_user::AuthUser;
use crate::domain::job::{ExifField, FullExif};
use crate::infra::error::AppError;
use crate::services;
use crate::services::aggregate::AggregateRequest;
use crate::services::pictures::{
    PictureListParams, PictureListResult, PictureVariant, TrashBatchOutcome, UploadMetadata,
};
use crate::services::selection::{self, PictureSelection};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CreateUploadRequest {
    pub filename: String,
}

#[derive(Debug, Serialize)]
pub struct CreateUploadResponse {
    pub picture_id: Uuid,
    pub presigned_url: String,
}

#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn create_upload(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<CreateUploadRequest>,
) -> Result<Json<CreateUploadResponse>, AppError> {
    let (picture_id, presigned_url) = services::pictures::begin_upload(
        state.cache.as_ref(),
        state.storage.as_ref(),
        &state.config,
        auth.user_id()?,
        &payload.filename,
    )
    .await?;
    Ok(Json(CreateUploadResponse {
        picture_id,
        presigned_url,
    }))
}

#[derive(Debug, Deserialize)]
pub struct BatchCreateUploadRequest {
    pub filenames: Vec<String>,
}

#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn batch_create_upload(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<BatchCreateUploadRequest>,
) -> Result<Json<Vec<CreateUploadResponse>>, AppError> {
    let results = services::pictures::begin_upload_batch(
        state.cache.as_ref(),
        state.storage.as_ref(),
        &state.config,
        auth.user_id()?,
        &payload.filenames,
    )
    .await?;
    Ok(Json(
        results
            .into_iter()
            .map(|(picture_id, presigned_url)| CreateUploadResponse {
                picture_id,
                presigned_url,
            })
            .collect(),
    ))
}

#[tracing::instrument(skip(auth, state, meta), fields(user_id = %auth.claims.uid.unwrap_or_default(), picture_id = %picture_id))]
pub async fn complete_upload(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(picture_id): Path<Uuid>,
    Json(meta): Json<UploadMetadata>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Whether this completion should wake the pipeline itself. The wake is **debounced**, so a batch
    // upload's per-file completions coalesce into a single pipeline run on their own — no need for the
    // caller to defer and wake once at the end. `defer_pipeline = true` remains an opt-out for a
    // caller that wants to drive the wake itself.
    let defer_pipeline = meta.defer_pipeline;
    let picture = services::pictures::complete_upload(
        &state.db,
        state.cache.as_ref(),
        state.storage.as_ref(),
        &state.config,
        auth.user_id()?,
        picture_id,
        meta,
    )
    .await?;
    if !defer_pipeline {
        // New picture: last_pipeline_run_at = NULL by default → wake the pipeline loop. Debounced so
        // a multi-file upload collapses into one run; manual `initial_tags` are already committed in
        // the completion tx, so only background rule evaluation waits for the window.
        state.pipeline_waker.wake_debounced(auth.user_id()?);
    }
    Ok(Json(serde_json::json!({ "id": picture.id })))
}

/// Explicitly wake the caller's tagging pipeline.
#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn wake_pipeline(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    state.pipeline_waker.wake(auth.user_id()?);
    Ok(Json(serde_json::json!({ "woken": true })))
}

#[tracing::instrument(skip(auth, state, params), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<PictureListParams>,
) -> Result<Json<PictureListResult>, AppError> {
    let result = services::pictures::list_pictures(
        &state.db,
        state.cache.as_ref(),
        state.storage.as_ref(),
        &state.config,
        &state.federation,
        auth.user_id()?,
        params,
    )
    .await?;
    Ok(Json(result))
}

#[derive(Debug, Deserialize)]
pub struct PictureUrlQuery {
    pub variant: PictureVariant,
}

#[tracing::instrument(skip(auth, state, query), fields(user_id = %auth.claims.uid.unwrap_or_default(), picture_id = %picture_id))]
pub async fn picture_url(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(picture_id): Path<Uuid>,
    Query(query): Query<PictureUrlQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let url = services::pictures::presign_picture_variant(
        &state.db,
        state.cache.as_ref(),
        state.storage.as_ref(),
        &state.config,
        &state.federation,
        auth.user_id()?,
        picture_id,
        query.variant,
    )
    .await?;
    Ok(Json(
        serde_json::json!({ "url": url, "variant": query.variant }),
    ))
}

#[tracing::instrument(skip(auth, state), fields(user_id = %auth.claims.uid.unwrap_or_default(), picture_id = %picture_id))]
pub async fn details(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(picture_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let d = services::pictures::get_picture_details(&state.db, auth.user_id()?, picture_id).await?;
    Ok(Json(serde_json::json!({
        "id": d.picture.id,
        "filename": d.picture.filename,
        "mime_type": d.picture.mime_type,
        "file_size": d.picture.file_size,
        "width": d.picture.width,
        "height": d.picture.height,
        "captured_at": d.picture.captured_at,
        "ingested_at": d.picture.ingested_at,
        "updated_at": d.picture.updated_at,
        "gps_lat": d.picture.gps_lat,
        "gps_lng": d.picture.gps_lng,
        "gps_alt": d.picture.gps_alt,
        "orientation": d.picture.orientation,
        "exif_data": d.picture.exif_data,
        "exif_sync_status": d.picture.exif_sync_status,
        "owner_username": d.picture.owner_username,
        "owner_instance_domain": d.picture.owner_instance_domain,
        // Trash & owner-deletion lifecycle (09 §5.3).
        "deleted_at": d.picture.deleted_at,
        "owner_deleted_at": d.picture.owner_deleted_at,
        "owner_purge_at": d.picture.owner_purge_at,
        // Recipient EXIF overrides (received pictures only).
        "local_exif_overrides": d.picture.local_exif_overrides,
        "versions": d.versions,
    })))
}

/// `POST /api/authenticated/pictures/{id}/trash` — soft-delete (owned or received) the picture.
#[tracing::instrument(skip(auth, state), fields(user_id = %auth.claims.uid.unwrap_or_default(), picture_id = %picture_id))]
pub async fn trash(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(picture_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let picture = services::pictures::trash_picture(
        &state.db,
        &state.pipeline_waker,
        auth.user_id()?,
        picture_id,
    )
    .await?;
    Ok(Json(serde_json::json!({
        "id": picture.id,
        "deleted_at": picture.deleted_at,
    })))
}

#[tracing::instrument(skip(auth, state, body), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn aggregate(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<AggregateRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let result = services::aggregate::aggregate(&state.db, auth.user_id()?, body).await?;
    Ok(Json(result))
}

/// Body for a batch trash/restore over a selection (feature 14 §6). Accepts the selection descriptor
/// or a legacy explicit `picture_ids` list; `dry_run: true` returns the affected count only.
#[derive(Debug, Deserialize)]
pub struct BatchTrashRequest {
    #[serde(default)]
    pub selection: Option<PictureSelection>,
    #[serde(default)]
    pub picture_ids: Vec<Uuid>,
    #[serde(default)]
    pub dry_run: bool,
}

async fn batch_set_trashed(
    auth: AuthUser,
    state: AppState,
    body: BatchTrashRequest,
    deleted: bool,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = auth.user_id()?;
    let sel = selection::resolve_or_explicit(
        &state.db,
        user_id,
        body.selection.as_ref(),
        body.picture_ids.clone(),
    )
    .await?;
    let outcome = services::pictures::batch_set_trashed_selection(
        &state.db,
        &state.pipeline_waker,
        user_id,
        &sel,
        deleted,
        body.dry_run,
    )
    .await?;
    Ok(Json(match outcome {
        TrashBatchOutcome::DryRun(dry) => {
            serde_json::to_value(dry).map_err(|e| AppError::InternalServerError(e.to_string()))?
        }
        TrashBatchOutcome::Applied { affected } => {
            serde_json::json!({ "affected": affected })
        }
    }))
}

#[tracing::instrument(
    skip(auth, state, body),
    fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default(), dry_run = body.dry_run)
)]
pub async fn batch_trash(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<BatchTrashRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    batch_set_trashed(auth, state, body, true).await
}

/// `POST /api/authenticated/pictures/restore`: batch restore over a selection
#[tracing::instrument(
    skip(auth, state, body),
    fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default(), dry_run = body.dry_run)
)]
pub async fn batch_restore(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<BatchTrashRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    batch_set_trashed(auth, state, body, false).await
}

/// `POST /api/authenticated/pictures/{id}/restore`: restore a soft-deleted picture.
#[tracing::instrument(skip(auth, state), fields(user_id = %auth.claims.uid.unwrap_or_default(), picture_id = %picture_id))]
pub async fn restore(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(picture_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let picture = services::pictures::restore_picture(
        &state.db,
        &state.pipeline_waker,
        auth.user_id()?,
        picture_id,
    )
    .await?;
    Ok(Json(serde_json::json!({
        "id": picture.id,
        "deleted_at": picture.deleted_at,
    })))
}

/// Whether a received-picture EXIF edit is a private local override or a propose-to-owner edit
/// (10 §4.1). Defaults to `local` (always permitted; no grant required).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceivedExifMode {
    #[default]
    Local,
    Propose,
}

/// Body for a received-picture EXIF edit (`set`/`clear`, same shape as an owned edit) plus the
/// `mode` discriminator (10 §4.1).
#[derive(Debug, Deserialize)]
pub struct ReceivedExifEditBody {
    #[serde(default)]
    pub mode: ReceivedExifMode,
    #[serde(default)]
    pub set: FullExif,
    #[serde(default)]
    pub clear: Vec<ExifField>,
}

/// `POST /api/authenticated/pictures/{id}/exif` — edit a **received** picture's EXIF (10 §4.1).
///
/// - `mode: "local"` (default) → private, DB-only sticky override (09 §6.2). Returns `200`.
/// - `mode: "propose"` → send the delta to the owner, who auto-applies + re-announces; requires the
///   share to grant editing (else `403`). Clears the proposed fields' local overrides. Returns `202`
///   (the authoritative change lands asynchronously).
///
/// Owned pictures are rejected — use `POST /pictures/{id}/edit`.
#[tracing::instrument(skip(auth, state, body), fields(user_id = %auth.claims.uid.unwrap_or_default(), picture_id = %picture_id))]
pub async fn edit_received_exif(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(picture_id): Path<Uuid>,
    Json(body): Json<ReceivedExifEditBody>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), AppError> {
    let user_id = auth.user_id()?;
    match body.mode {
        ReceivedExifMode::Local => {
            let picture = services::pictures::override_received_exif(
                &state.db,
                &state.pipeline_waker,
                user_id,
                picture_id,
                body.set,
                body.clear,
            )
            .await?;
            Ok((
                axum::http::StatusCode::OK,
                Json(serde_json::json!({
                    "id": picture.id,
                    "captured_at": picture.captured_at,
                    "gps_lat": picture.gps_lat,
                    "gps_lng": picture.gps_lng,
                    "gps_alt": picture.gps_alt,
                    "orientation": picture.orientation,
                    "exif_data": picture.exif_data,
                    "local_exif_overrides": picture.local_exif_overrides,
                    "updated_at": picture.updated_at,
                })),
            ))
        }
        ReceivedExifMode::Propose => {
            let picture = services::pictures::propose_received_exif(
                &state.db,
                state.cache.as_ref(),
                &state.config,
                &state.federation,
                &state.pipeline_waker,
                user_id,
                &auth.claims.sub,
                picture_id,
                body.set,
                body.clear,
            )
            .await?;
            Ok((
                axum::http::StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "id": picture.id,
                    "captured_at": picture.captured_at,
                    "gps_lat": picture.gps_lat,
                    "gps_lng": picture.gps_lng,
                    "gps_alt": picture.gps_alt,
                    "orientation": picture.orientation,
                    "exif_data": picture.exif_data,
                    "local_exif_overrides": picture.local_exif_overrides,
                    "updated_at": picture.updated_at,
                })),
            ))
        }
    }
}
