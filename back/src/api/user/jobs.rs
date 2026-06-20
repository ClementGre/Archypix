use crate::api::middleware::auth_user::AuthUser;
use crate::domain::job::{ExifField, FullExif, Job};
use crate::infra::error::AppError;
use crate::repository::job::JobRepository;
use crate::repository::picture::PictureRepository;
use crate::services;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use tracing::debug;
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
pub async fn get_job(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<Job>, AppError> {
    debug!(user = %auth.claims.sub, job_id = %job_id, "get_job");
    let job = JobRepository::find_by_id(&state.db, job_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if job.owner_id != auth.user_id()? {
        return Err(AppError::NotFound);
    }
    Ok(Json(job))
}

/// `GET /api/authenticated/pictures/{id}/jobs` — list all jobs for a picture.
pub async fn list_picture_jobs(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(picture_id): Path<Uuid>,
) -> Result<Json<Vec<Job>>, AppError> {
    debug!(user = %auth.claims.sub, picture_id = %picture_id, "list_picture_jobs");
    let jobs = services::jobs::list_picture_jobs(&state.db, picture_id, auth.user_id()?).await?;
    Ok(Json(jobs))
}

/// `POST /api/authenticated/pictures/{id}/edit` — edit a single picture's EXIF (write-through).
/// Applies the DB change synchronously and enqueues the file reconcile; returns the updated row,
/// its `exif_sync_status`, and the reconcile `job_id` (or `null` when `unsupported`).
pub async fn enqueue_edit(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(picture_id): Path<Uuid>,
    Json(body): Json<ExifEditBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    debug!(user = %auth.claims.sub, picture_id = %picture_id, "enqueue_edit");
    let user_id = auth.user_id()?;
    let outcome = services::jobs::edit_pictures_exif(
        &state.db,
        &state.pipeline_waker,
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

/// `PATCH /api/authenticated/pictures/exif` — batch EXIF edit (§7.2).
pub async fn batch_edit_exif(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<BatchExifEditBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    debug!(user = %auth.claims.sub, count = body.picture_ids.len(), "batch_edit_exif");
    let outcome = services::jobs::edit_pictures_exif(
        &state.db,
        &state.pipeline_waker,
        auth.user_id()?,
        &body.picture_ids,
        body.set,
        body.clear,
    )
    .await?;
    Ok(Json(serde_json::json!({
        "updated": outcome.updated,
        "jobs": outcome.jobs,
        "unsupported": outcome.unsupported,
    })))
}

/// Body for a batch EXIF edit (`PATCH /pictures/exif`).
#[derive(Debug, Deserialize)]
pub struct BatchExifEditBody {
    pub picture_ids: Vec<Uuid>,
    #[serde(default)]
    pub set: FullExif,
    #[serde(default)]
    pub clear: Vec<ExifField>,
}

/// `POST /api/authenticated/pictures/{id}/exif/resync` — re-enqueue a stuck `pending` picture.
pub async fn resync_exif(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(picture_id): Path<Uuid>,
) -> Result<Json<Job>, AppError> {
    debug!(user = %auth.claims.sub, picture_id = %picture_id, "resync_exif");
    let job = services::jobs::resync_picture_exif(
        &state.db,
        &state.pipeline_waker,
        auth.user_id()?,
        picture_id,
    )
    .await?;
    Ok(Json(job))
}
