use crate::api::middleware::auth_user::AuthUser;
use crate::domain::job::{ExifField, FullExif};
use crate::services;
use crate::services::aggregate::AggregateRequest;
use crate::services::pictures::{
    BatchUploadFile, BatchUploadOutcome, PictureListParams, PictureListResult, PictureVariant,
    TrashBatchOutcome, UploadMetadata,
};
use crate::services::selection::{self, PictureSelection};
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CreateUploadRequest {
    pub filename: String,
    /// Client-declared byte size, enabling the presign-time storage-quota reservation (feature 22).
    #[serde(default)]
    pub size: Option<i64>,
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
        &state.db,
        state.cache.as_ref(),
        state.storage.as_ref(),
        &state.settings,
        auth.user_id()?,
        &payload.filename,
        payload.size,
    )
    .await?;
    Ok(Json(CreateUploadResponse {
        picture_id,
        presigned_url,
    }))
}

#[derive(Debug, Deserialize)]
pub struct BatchCreateUploadRequest {
    pub files: Vec<BatchUploadFile>,
    /// Manual tags assigned atomically to any deduplicated (conflicting) pictures — the upload-time
    /// equivalent of `complete`'s `initial_tags` for files the user already holds.
    #[serde(default)]
    pub initial_tags: Option<Vec<String>>,
    /// Front-provided import label (`Uploaded.YYYY_MM_DD_HH_MM`, fixed per batch). Duplicates are
    /// tagged with the `AlreadyExisting`[`.Deleted`] marker here (feature 15).
    #[serde(default)]
    pub upload_label: Option<String>,
}

/// One requested slot in a batch presign response. For a fresh file `presigned_url` is set and
/// `duplicate` is `false`; for a dedup hit `presigned_url` is `null`, `duplicate` is `true`, and
/// `picture_id` is the existing picture the bytes already match. `was_deleted` is `true` when the
/// matched picture is in the user's trash (it is **not** auto-restored — feature 15).
#[derive(Debug, Serialize)]
pub struct BatchUploadSlotResponse {
    pub picture_id: Uuid,
    pub presigned_url: Option<String>,
    pub duplicate: bool,
    pub was_deleted: bool,
}

#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn batch_create_upload(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<BatchCreateUploadRequest>,
) -> Result<Json<Vec<BatchUploadSlotResponse>>, AppError> {
    let initial_tags = payload.initial_tags.unwrap_or_default();
    // The service handles dedup-target side effects (tag existing/deleted duplicates) and wakes the
    // pipeline itself when any of that happened.
    let results = services::pictures::begin_upload_batch(
        &state.db,
        state.cache.as_ref(),
        state.storage.as_ref(),
        &state.settings,
        auth.user_id()?,
        &payload.files,
        &initial_tags,
        payload.upload_label.as_deref(),
        &state.routines.pipeline,
    )
    .await?;

    Ok(Json(
        results
            .into_iter()
            .map(|r| match r {
                BatchUploadOutcome::New {
                    picture_id,
                    presigned_url,
                } => BatchUploadSlotResponse {
                    picture_id,
                    presigned_url: Some(presigned_url),
                    duplicate: false,
                    was_deleted: false,
                },
                BatchUploadOutcome::Duplicate {
                    picture_id,
                    was_deleted,
                } => BatchUploadSlotResponse {
                    picture_id,
                    presigned_url: None,
                    duplicate: true,
                    was_deleted,
                },
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
        &state.settings,
        auth.user_id()?,
        picture_id,
        meta,
    )
    .await?;
    if !defer_pipeline {
        // New picture: last_pipeline_run_at = NULL by default → wake the pipeline loop. Debounced so
        // a multi-file upload collapses into one run; manual `initial_tags` are already committed in
        // the completion tx, so only background rule evaluation waits for the window.
        state.routines.pipeline.trigger_debounced(auth.user_id()?);
    }
    Ok(Json(serde_json::json!({ "id": picture.id })))
}

/// Explicitly wake the caller's tagging pipeline.
#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn wake_pipeline(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    state.routines.pipeline.trigger(auth.user_id()?);
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
        &state.settings,
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
        &state.settings,
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
    let global_domain = state
        .settings
        .get(crate::infra::settings::keys::GLOBAL_DOMAIN);
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
        // Creator attribution (feature 26). `creator` is the resolved display (override → stored →
        // owner default); `creator_origin` is the propagated value the override sits on top of;
        // `creator_value`/`creator_override` are the raw columns driving the edit/reset affordances.
        "creator": d.picture.display_creator(&auth.claims.sub, &global_domain),
        "creator_origin": d.picture.propagated_creator(&auth.claims.sub, &global_domain),
        "creator_value": d.picture.creator,
        "creator_override": d.picture.creator_override,
        // Trash & owner-deletion lifecycle (09 §5.3).
        "deleted_at": d.picture.deleted_at,
        "owner_deleted_at": d.picture.owner_deleted_at,
        "owner_purge_at": d.picture.owner_purge_at,
        // Recipient EXIF overrides (received pictures only).
        "local_exif_overrides": d.picture.local_exif_overrides,
        // Physical-copy provenance & content-dedup grouping key (feature 11).
        "content_hash": d.picture.content_hash,
        "copy_source_owner_username": d.picture.copy_source_owner_username,
        "copy_source_owner_instance": d.picture.copy_source_owner_instance,
        "copy_source_picture_id": d.picture.copy_source_picture_id,
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
        &state.routines.pipeline,
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

/// `POST /api/authenticated/pictures/{id}/copy` — copy a received (or owned) picture into the
/// caller's library as a new, independent owned picture (feature 11 §3).
#[tracing::instrument(skip(auth, state), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default(), picture_id = %picture_id))]
pub async fn copy(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(picture_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let picture = services::pictures::copy_picture(
        &state.db,
        state.cache.as_ref(),
        state.storage.as_ref(),
        &state.settings,
        &state.federation,
        &state.routines.pipeline,
        auth.user_id()?,
        &auth.claims.sub,
        picture_id,
    )
    .await?;
    Ok(Json(serde_json::json!({ "id": picture.id })))
}

/// `GET /api/authenticated/pictures/{id}/copies` — the content-dedup group of a picture (feature 11
/// §5.5): the live survivor plus its hidden siblings, each with both hashes (so the client can show
/// "same content / EXIF-only difference" vs "different content"), state, last-edit time, and owner.
#[tracing::instrument(skip(auth, state), fields(user_id = %auth.claims.uid.unwrap_or_default(), picture_id = %picture_id))]
pub async fn copies(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(picture_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let rows = services::pictures::picture_copies(&state.db, auth.user_id()?, picture_id).await?;
    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            let state = match (r.deleted_at, r.deleted_reason) {
                (None, _) => "live",
                (Some(_), Some(crate::domain::picture::DeletedReason::Manual)) => "manual",
                (Some(_), Some(crate::domain::picture::DeletedReason::Boomerang)) => "boomerang",
                (Some(_), Some(crate::domain::picture::DeletedReason::ContentDedupe)) => {
                    "content_dedupe"
                }
                (Some(_), None) => "deleted",
            };
            serde_json::json!({
                "id": r.id,
                "filename": r.filename,
                "content_hash": r.content_hash,
                "file_hash": r.file_hash,
                "state": state,
                "updated_at": r.updated_at,
                "owned": r.is_owned,
                "owner_username": r.owner_username,
                "owner_instance": r.owner_instance_domain,
                "owner_deleted_at": r.owner_deleted_at,
                "copy_source_owner_username": r.copy_source_owner_username,
                "copy_source_owner_instance": r.copy_source_owner_instance,
                "copy_source_picture_id": r.copy_source_picture_id,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "copies": items })))
}

/// `POST /api/authenticated/pictures/{id}/copies/keep` — make this picture the live survivor of its
/// content-dedup group (feature 11 §5.5), hiding the others as `content_dedupe`.
#[tracing::instrument(skip(auth, state), fields(user_id = %auth.claims.uid.unwrap_or_default(), picture_id = %picture_id))]
pub async fn keep_copy(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(picture_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    services::pictures::set_picture_survivor(
        &state.db,
        &state.routines.pipeline,
        auth.user_id()?,
        picture_id,
    )
    .await?;
    Ok(Json(serde_json::json!({ "kept": picture_id })))
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
        &state.routines.pipeline,
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
        &state.routines.pipeline,
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
    /// Fields to override to **empty** (local mode only, 10 §6.3)
    #[serde(default)]
    pub empty: Vec<ExifField>,
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
                &state.routines.pipeline,
                user_id,
                picture_id,
                body.set,
                body.empty,
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
            // Emptying a field is expressed to the owner as a `clear` (owner-side clear nulls the
            // column, 04 §7.3); the recipient-local `empty`/`clear` distinction only exists for the
            // private override path, so fold the two here.
            let mut clear = body.clear;
            for f in body.empty {
                if !clear.contains(&f) {
                    clear.push(f);
                }
            }
            let picture = services::pictures::propose_received_exif(
                &state.db,
                state.cache.as_ref(),
                &state.settings,
                &state.federation,
                &state.routines.pipeline,
                user_id,
                &auth.claims.sub,
                picture_id,
                body.set,
                clear,
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

/// Whether a creator edit targets the recipient-local override or proposes a change to the owner
/// (feature 26 §7). `local` is the default; `propose` is phase 2 (currently `403`). Ignored for
/// owned pictures (the owner-authoritative `creator` is always set directly).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatorMode {
    #[default]
    Local,
    Propose,
}

#[derive(Debug, Deserialize)]
pub struct SetCreatorBody {
    /// The new credit. `null`/blank ⇒ reset to owner default (owned) or clear the override (received).
    /// Rejected if it begins with a reserved sigil (`@`/`#`, feature 26 §3).
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub mode: CreatorMode,
}

/// `POST /api/authenticated/pictures/{id}/creator` — set the creator credit (feature 26 §7).
///
/// - **Owned** → sets the authoritative `creator` (`null`/blank resets to owner default). The change
///   re-announces to recipients via the pipeline.
/// - **Received**, `mode: "local"` (default) → sets the recipient-local `creator_override`
///   (`null`/blank clears it). Never propagates.
/// - **Received**, `mode: "propose"` → phase 2, returns `403`.
#[tracing::instrument(skip(auth, state, body), fields(user_id = %auth.claims.uid.unwrap_or_default(), picture_id = %picture_id))]
pub async fn set_creator(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(picture_id): Path<Uuid>,
    Json(body): Json<SetCreatorBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let propose = matches!(body.mode, CreatorMode::Propose);
    let picture = services::pictures::set_picture_creator(
        &state.db,
        &state.routines.pipeline,
        auth.user_id()?,
        picture_id,
        body.value,
        propose,
    )
    .await?;
    let global_domain = state
        .settings
        .get(crate::infra::settings::keys::GLOBAL_DOMAIN);
    Ok(Json(serde_json::json!({
        "id": picture.id,
        "creator": picture.display_creator(&auth.claims.sub, &global_domain),
        "creator_origin": picture.propagated_creator(&auth.claims.sub, &global_domain),
        "creator_value": picture.creator,
        "creator_override": picture.creator_override,
        "updated_at": picture.updated_at,
    })))
}
