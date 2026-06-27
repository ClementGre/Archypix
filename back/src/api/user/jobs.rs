use crate::api::middleware::auth_user::AuthUser;
use crate::domain::job::{ExifField, FullExif, Job};
use crate::infra::error::AppError;
use crate::repository::job::JobRepository;
use crate::repository::picture::PictureRepository;
use crate::services;
use crate::services::jobs::{BatchExifMode, ExifBatchOutcome};
use crate::services::selection::{self, PictureSelection};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use uuid::Uuid;

/// Body for a single-picture EXIF edit (`set`/`clear` shape, §7.3).
#[derive(Debug, Deserialize)]
pub struct ExifEditBody {
    #[serde(default)]
    pub set: FullExif,
    #[serde(default)]
    pub clear: Vec<ExifField>,
}

/// `GET /api/authenticated/jobs/{id}` — get the status of a single job.
#[tracing::instrument(skip(auth, state), fields(user_id = %auth.claims.uid.unwrap_or_default(), job_id = %job_id))]
pub async fn get_job(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<Job>, AppError> {
    let job = JobRepository::find_by_id(&state.db, job_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if job.owner_id != auth.user_id()? {
        return Err(AppError::NotFound);
    }
    Ok(Json(job))
}

/// `GET /api/authenticated/pictures/{id}/jobs` — list all jobs for a picture.
#[tracing::instrument(skip(auth, state), fields(user_id = %auth.claims.uid.unwrap_or_default(), picture_id = %picture_id))]
pub async fn list_picture_jobs(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(picture_id): Path<Uuid>,
) -> Result<Json<Vec<Job>>, AppError> {
    let jobs = services::jobs::list_picture_jobs(&state.db, picture_id, auth.user_id()?).await?;
    Ok(Json(jobs))
}

/// `POST /api/authenticated/pictures/{id}/edit` — edit a single picture's EXIF (write-through).
/// Applies the DB change synchronously and enqueues the file reconcile; returns the updated row,
/// its `exif_sync_status`, and the reconcile `job_id` (or `null` when `unsupported`).
#[tracing::instrument(skip(auth, state, body), fields(user_id = %auth.claims.uid.unwrap_or_default(), picture_id = %picture_id))]
pub async fn enqueue_edit(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(picture_id): Path<Uuid>,
    Json(body): Json<ExifEditBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = auth.user_id()?;
    let outcome = services::jobs::edit_pictures_exif(
        &state.db,
        &state.routines.pipeline,
        user_id,
        &[picture_id],
        body.set,
        body.clear,
    )
    .await?;
    let picture = PictureRepository::find_by_id(&state.db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(serde_json::json!({
        "id": picture.id,
        "exif_sync_status": picture.exif_sync_status,
        "captured_at": picture.captured_at,
        "gps_lat": picture.gps_lat,
        "gps_lng": picture.gps_lng,
        "gps_alt": picture.gps_alt,
        "orientation": picture.orientation,
        "exif_data": picture.exif_data,
        "updated_at": picture.updated_at,
        "job_id": outcome.jobs.first().copied(),
    })))
}

/// Owned pictures take the deferred-job write-through; received pictures take a recipient-local override (or, in `suggest` mode where the share grants it. Convergence is tracked through the `exif_sync` histogram. With `dry_run: true` returns the affected breakdown without mutating.
#[tracing::instrument(
    skip(auth, state, body),
    fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default(), dry_run = body.dry_run)
)]
pub async fn batch_edit_exif(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<BatchExifEditBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = auth.user_id()?;
    let sel = selection::resolve_or_explicit(
        &state.db,
        user_id,
        body.selection.as_ref(),
        body.picture_ids.clone(),
    )
    .await?;
    let outcome = services::jobs::batch_edit_exif_selection(
        &state.db,
        &state.routines.pipeline,
        &state.routines.exif_drain,
        state.cache.as_ref(),
        &state.config,
        &state.federation,
        user_id,
        &auth.claims.sub,
        &sel,
        body.set,
        body.clear,
        body.mode,
        body.dry_run,
    )
    .await?;
    Ok(Json(match outcome {
        ExifBatchOutcome::DryRun(dry) => {
            serde_json::to_value(dry).map_err(|e| AppError::InternalServerError(e.to_string()))?
        }
        ExifBatchOutcome::Applied {
            affected,
            edited,
            suggested,
            local_override,
            unsupported,
        } => serde_json::json!({
            "affected": affected,
            "edited": edited,
            "suggested": suggested,
            "local_override": local_override,
            "unsupported": unsupported,
        }),
    }))
}

/// Body for a batch EXIF edit (`PATCH /pictures/exif`). Accepts the selection descriptor or a legacy
/// explicit `picture_ids` list.
#[derive(Debug, Deserialize)]
pub struct BatchExifEditBody {
    #[serde(default)]
    pub selection: Option<PictureSelection>,
    #[serde(default)]
    pub picture_ids: Vec<Uuid>,
    #[serde(default)]
    pub set: FullExif,
    #[serde(default)]
    pub clear: Vec<ExifField>,
    #[serde(default)]
    pub mode: BatchExifMode,
    #[serde(default)]
    pub dry_run: bool,
}

/// `POST /api/authenticated/pictures/{id}/exif/resync` — re-enqueue a stuck `pending` picture.
#[tracing::instrument(skip(auth, state), fields(user_id = %auth.claims.uid.unwrap_or_default(), picture_id = %picture_id))]
pub async fn resync_exif(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(picture_id): Path<Uuid>,
) -> Result<Json<Job>, AppError> {
    let job = services::jobs::resync_picture_exif(
        &state.db,
        &state.routines.pipeline,
        auth.user_id()?,
        picture_id,
    )
    .await?;
    Ok(Json(job))
}
