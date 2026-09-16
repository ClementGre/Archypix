use crate::api::middleware::auth_worker::AuthWorker;
use crate::api::worker::models::{
    ClaimJobResponse, ExifExtraction, JobOutcome, JobProduct, JobResponse, PictureWork,
};
use crate::domain::job::{ExifEdit, JobConfig, JobStatus, JobType};
use crate::domain::picture::ExifSyncStatus;
use crate::domain::user_settings::VersioningMode;
use crate::infra::observability;
use crate::infra::s3;
use crate::infra::settings::keys;
use crate::repository::job::JobRepository;
use crate::repository::picture::PictureRepository;
use crate::repository::picture_version::PictureVersionRepository;
use crate::repository::pipeline::PipelineRepository;
use crate::repository::share_announcement::ShareAnnouncementRepository;
use crate::repository::user_settings::UserSettingsRepository;
use crate::state::AppState;
use archypix_common::error::{AppError, map_sqlx_error};
use archypix_common::transfer::ClaimQuery;
use archypix_common::transfer::PresignedWrites;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use sqlx::Postgres;
use tracing::debug;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

#[tracing::instrument(skip(auth, state, query), fields(worker = auth.worker_id()))]
pub async fn claim_next_job(
    auth: AuthWorker,
    State(state): State<AppState>,
    Query(query): Query<ClaimQuery>,
) -> Result<Json<Option<ClaimJobResponse>>, AppError> {
    let Some(job) = JobRepository::claim_next(&state.db, auth.worker_id(), &query.types).await?
    else {
        return Ok(Json(None));
    };

    // claim_token was generated and stored by claim_next; forward it to the worker.
    let claim_token = job.claim_token.ok_or_else(|| {
        AppError::InternalServerError("claimed job has no claim_token".to_string())
    })?;

    let trace_context = job.trace_context.as_ref().and_then(|tc| {
        serde_json::from_value::<std::collections::HashMap<String, String>>(tc.0.clone()).ok()
    });

    // ML jobs have no picture and no S3 I/O for now — return early with empty presigned fields.
    if matches!(
        job.job_type,
        JobType::MlStyle | JobType::MlPeople | JobType::MlGroupLocation
    ) {
        let config = job.typed_config().map_err(|e| {
            AppError::InternalServerError(format!("failed to parse job config: {e}"))
        })?;
        return Ok(Json(Some(ClaimJobResponse {
            job_id: job.id,
            job_type: job.job_type,
            picture_id: job.picture_id,
            mime_type: None,
            config,
            presigned_read: None,
            presigned_writes: PresignedWrites::default(),
            claim_token,
            trace_context,
        })));
    }

    let picture_id = job.picture_id.ok_or_else(|| {
        AppError::InternalServerError("claimed job has no picture_id".to_string())
    })?;

    let picture = PictureRepository::find_by_id(&state.db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let original_key = s3::picture_key(picture.local_user_id, picture_id);
    let presigned_read = state
        .storage
        .presign_get_worker(&state.settings.get(keys::S3_BUCKET_PICTURES), &original_key)
        .await?;

    let mut config = job
        .typed_config()
        .map_err(|e| AppError::InternalServerError(format!("failed to parse job config: {e}")))?;
    // Late-binding EXIF target (feature 31 §3.3): the worker writes the picture's state as of the
    // claim, and the persisted config tells `complete_job` which state the file was brought to.
    if let JobConfig::EditPicture(edit) = &mut config {
        if edit.exif.is_some() {
            edit.exif = Some(ExifEdit {
                target: picture.full_exif(),
            });
            JobRepository::update_config(&state.db, job.id, &config).await?;
        }
    }

    // For edit_picture jobs: snapshot the current file as a new version BEFORE issuing the
    // presigned write URL that would overwrite it. The versioning predicate (§9):
    //   None           → never;
    //   OriginalCopy   → only the first edit (keep the pristine original, once);
    //   FullVersioning → first edit, or any visual edit (exif-only edits never add a version).
    if job.job_type == JobType::EditPicture {
        let settings =
            UserSettingsRepository::get_or_default(&state.db, picture.local_user_id).await?;
        let is_visual_edit = matches!(&config, JobConfig::EditPicture(c) if c.visual.is_some());
        let has_existing_version =
            PictureVersionRepository::has_versions(&state.db, picture_id).await?;
        let snapshot_version = match settings.versioning_mode {
            VersioningMode::None => false,
            VersioningMode::OriginalCopy => !has_existing_version,
            VersioningMode::FullVersioning => !has_existing_version || is_visual_edit,
        };
        if snapshot_version {
            let version_id = Uuid::new_v4();
            // S3 copy first (outside DB tx) — safe because no DB record exists yet.
            state
                .storage
                .copy_object(
                    &state.settings.get(keys::S3_BUCKET_PICTURES),
                    &original_key,
                    &state.settings.get(keys::S3_BUCKET_VERSIONS),
                    &s3::version_key(picture.local_user_id, picture_id, version_id),
                )
                .await?;
            // DB: insert version record in a transaction so version_number is
            // computed and stored atomically.
            let mut vtx = state.db.begin().await.map_err(|e| {
                AppError::InternalServerError(format!("failed to begin version tx: {e}"))
            })?;
            let version_num =
                PictureVersionRepository::next_version_number(&mut *vtx, picture_id).await?;
            PictureVersionRepository::create(
                &mut *vtx,
                version_id,
                picture_id,
                version_num,
                picture.file_size,
                picture.mime_type.as_deref(),
            )
            .await?;
            vtx.commit().await.map_err(map_sqlx_error)?;
        }
    }

    let thumb_key = s3::picture_key(picture.local_user_id, picture_id);
    let presigned_writes = match &config {
        JobConfig::GenThumbnail(_) => PresignedWrites::thumbnails(
            state
                .storage
                .presign_put_worker(&state.settings.get(keys::S3_BUCKET_SMALL), &thumb_key)
                .await?,
            state
                .storage
                .presign_put_worker(&state.settings.get(keys::S3_BUCKET_MEDIUM), &thumb_key)
                .await?,
            state
                .storage
                .presign_put_worker(&state.settings.get(keys::S3_BUCKET_LARGE), &thumb_key)
                .await?,
        ),
        JobConfig::EditPicture(edit_cfg) => {
            let output = state
                .storage
                .presign_put_worker(&state.settings.get(keys::S3_BUCKET_PICTURES), &original_key)
                .await?;
            if edit_cfg.visual.is_some() {
                PresignedWrites::edit_with_visual(
                    output,
                    state
                        .storage
                        .presign_put_worker(&state.settings.get(keys::S3_BUCKET_SMALL), &thumb_key)
                        .await?,
                    state
                        .storage
                        .presign_put_worker(&state.settings.get(keys::S3_BUCKET_MEDIUM), &thumb_key)
                        .await?,
                    state
                        .storage
                        .presign_put_worker(&state.settings.get(keys::S3_BUCKET_LARGE), &thumb_key)
                        .await?,
                )
            } else {
                PresignedWrites::exif_only(output)
            }
        }
        _ => PresignedWrites::default(),
    };

    Ok(Json(Some(ClaimJobResponse {
        job_id: job.id,
        job_type: job.job_type,
        picture_id: job.picture_id,
        mime_type: picture.mime_type.clone(),
        config,
        presigned_read: Some(presigned_read),
        presigned_writes,
        claim_token,
        trace_context,
    })))
}

/// The single terminal response for a claimed job (04 §"Job response").
///
/// `outcome` disposes of the job; `product` records what it managed to produce, which a job that
/// died still has (feature 33 §6.5).
#[tracing::instrument(skip(auth, state, body, headers), fields(worker = auth.worker_id(), job_id = %job_id))]
pub async fn respond_job(
    auth: AuthWorker,
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
    Json(body): Json<JobResponse>,
) -> Result<StatusCode, AppError> {
    // The worker injects its job span's context; reparent so the response is a child of the job span.
    let cx = observability::extract_from_headers(&headers);
    tracing::Span::current().set_parent(cx);

    // Read outside the transaction to fail fast on corrupt JSONB; the claim_token guard inside the
    // dispose UPDATE is what makes that safe.
    let job = JobRepository::find_by_id(&state.db, job_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let picture_id = job.picture_id;
    let config = job
        .typed_config()
        .map_err(|e| AppError::InternalServerError(format!("failed to parse job config: {e}")))?;

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(format!("failed to begin tx: {e}")))?;

    // Dispose first: claim-token guarded, and the resulting status says whether this attempt was the
    // last one. A re-queued job has settled nothing, so none of the recording below applies to it.
    let disposed = match &body.outcome {
        JobOutcome::Done => {
            let result = serde_json::json!({
                "worker_id": auth.worker_id(),
                "product": &body.product,
            });
            JobRepository::complete(&mut *tx, job_id, body.claim_token, result).await?
        }
        JobOutcome::Retry { error } => {
            JobRepository::fail(&mut *tx, job_id, body.claim_token, error, false).await?
        }
        JobOutcome::Failed { error } => {
            JobRepository::fail(&mut *tx, job_id, body.claim_token, error, true).await?
        }
    };
    let Some(job) = disposed else {
        tx.rollback().await.ok();
        return Err(AppError::Conflict(
            "job is no longer in processing state or claim token does not match".to_string(),
        ));
    };
    if job.status == JobStatus::Pending {
        tx.commit().await.map_err(map_sqlx_error)?;
        return Ok(StatusCode::NO_CONTENT);
    }
    let dead = job.status == JobStatus::Failed;

    // Record the product. The job type picks the branch, so an extraction's EXIF can never be
    // mistaken for an edit's read-back — a distinction the old `(job_type, exif)` match had to make.
    let mut reannounce_owner: Option<Uuid> = None;
    let mut needs_exif_drain = false;
    if let Some(pid) = picture_id {
        match (&body.product, &config) {
            (JobProduct::GenThumbnail(work), JobConfig::GenThumbnail(cfg)) => {
                record_extraction(&mut tx, pid, work, cfg.is_initial, dead).await?;
                // A completion is where `file_hash`/`blurhash`/`content_hash` first land. Re-mark a
                // tracked picture dirty so the pipeline delta re-announces the refreshed metadata,
                // and wake the owner's pipeline so the dedup reconciler (feature 11) sees the hash.
                if !dead {
                    if ShareAnnouncementRepository::is_picture_tracked(&mut *tx, pid).await? {
                        PipelineRepository::invalidate(&mut *tx, &[pid]).await?;
                    }
                    reannounce_owner = Some(job.owner_id);
                }
            }
            (JobProduct::EditPicture(work), JobConfig::EditPicture(cfg)) => {
                needs_exif_drain = record_edit(&mut tx, pid, work, cfg, dead).await?;
            }
            (JobProduct::Ml, _) => {}
            (product, _) => {
                tx.rollback().await.ok();
                return Err(AppError::BadRequest(format!(
                    "product {product:?} does not match job type {:?}",
                    job.job_type
                )));
            }
        }
    }

    tx.commit().await.map_err(map_sqlx_error)?;

    // Wake (post-commit) so the pipeline re-announces the freshly-hashed/thumbnailed picture.
    // Debounced for the same reason — a batch upload's thumbnails complete in a burst.
    if let Some(owner_id) = reannounce_owner {
        state.routines.pipeline.trigger_debounced(owner_id);
    }
    // An edit landed while this job ran: let the drain create the follow-up reconcile.
    if needs_exif_drain {
        state.routines.exif_drain.trigger(());
        state.routines.pipeline.trigger_debounced(job.owner_id);
    }

    Ok(StatusCode::NO_CONTENT)
}

/// The terminal sync status an [`ExifExtraction`] carries, or `None` when it carries no verdict.
/// One mapping for both directions (feature 33 §5); each caller supplies its own fallback.
fn terminal_verdict(exif: &ExifExtraction) -> Option<ExifSyncStatus> {
    match exif {
        ExifExtraction::UnsupportedMime => Some(ExifSyncStatus::UnsupportedMime),
        ExifExtraction::Failed => Some(ExifSyncStatus::UnsupportedFile),
        ExifExtraction::Extracted(_) | ExifExtraction::NotAttempted => None,
    }
}

/// Record what a `gen_thumbnail` produced. `dead` means the job will not run again (feature 33 §6.5).
///
/// A dead job's read is still good, but it must not overwrite a re-extraction that landed while it
/// was dying, and `thumbnails_generated` is whatever it actually uploaded — not an assumed `true`.
async fn record_extraction(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    picture_id: Uuid,
    work: &PictureWork,
    is_initial: bool,
    dead: bool,
) -> Result<(), AppError> {
    // A dead non-initial job was never asked for a verdict and has nothing to settle.
    if dead && !is_initial {
        return Ok(());
    }
    if dead
        && !PictureRepository::find_by_id(&mut **tx, picture_id)
            .await?
            .is_some_and(|p| p.exif_sync_status == ExifSyncStatus::Extracting)
    {
        return Ok(());
    }

    match &work.exif {
        // The file's own metadata is authoritative for the row (feature 31 §5).
        ExifExtraction::Extracted(extracted) => {
            PictureRepository::update_from_worker(
                &mut **tx,
                picture_id,
                &extracted.exif,
                work.width,
                work.height,
                work.blurhash.as_deref(),
                work.file_size,
                work.file_hash.as_deref(),
                work.content_hash.as_deref(),
                work.thumbnails_generated,
            )
            .await?;
        }
        // No read to record. `unsupported_*` are verdicts the worker reached; `NotAttempted` on a
        // dead job is an absence of one, which is what `extract_failed` records.
        exif => {
            PictureRepository::update_after_processing(
                &mut **tx,
                picture_id,
                work.thumbnails_generated,
                work.blurhash.as_deref(),
                work.file_size,
                work.file_hash.as_deref(),
                work.content_hash.as_deref(),
                work.width,
                work.height,
            )
            .await?;
            let fallback = dead.then_some(ExifSyncStatus::ExtractFailed);
            if let Some(status) = terminal_verdict(exif).or(fallback) {
                PictureRepository::set_exif_sync_status(&mut **tx, picture_id, status).await?;
            }
        }
    }
    Ok(())
}

/// Record what an `edit_picture` produced. Returns whether the EXIF drain owes a follow-up job.
///
/// On success this is state-based convergence (feature 31 §3.3): the DB is compared against the
/// target *this job wrote*, never against the read-back. On a dead job it is the write verdict.
async fn record_edit(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    picture_id: Uuid,
    work: &PictureWork,
    cfg: &archypix_common::job::EditPictureConfig,
    dead: bool,
) -> Result<bool, AppError> {
    // Job failed
    if dead {
        if cfg.exif.is_some() {
            // Update exif sync status
            let status = terminal_verdict(&work.exif).unwrap_or(ExifSyncStatus::WriteFailed);
            PictureRepository::set_exif_sync_status(&mut **tx, picture_id, status).await?;
        }
        return Ok(false);
    }

    PictureRepository::update_after_processing(
        &mut **tx,
        picture_id,
        work.thumbnails_generated,
        work.blurhash.as_deref(),
        work.file_size,
        work.file_hash.as_deref(),
        work.content_hash.as_deref(),
        work.width,
        work.height,
    )
    .await?;

    if let Some(edit) = cfg.exif.as_ref() {
        // Update file exif and exif sync status

        let picture = PictureRepository::find_by_id(&mut **tx, picture_id)
            .await?
            .ok_or(AppError::NotFound)?;
        // A re-extraction landed while this job ran (33 §6.4): it owns both writes now, and this job's
        // read-back describes the pre-overwrite bytes. Skip them rather than corrupt them.
        if picture.exif_sync_status == ExifSyncStatus::Extracting {
            debug!(picture_id = %picture_id, "edit response skipped: an extraction is in flight");
            return Ok(false);
        }
        if let Some(extracted) = work.exif.extracted() {
            PictureRepository::set_file_exif(&mut **tx, picture_id, &extracted.exif).await?;
        }

        let (status, drain) = if picture.full_exif() == edit.target {
            (ExifSyncStatus::Synced, false)
        } else {
            (ExifSyncStatus::PendingJobCreation, true)
        };
        PictureRepository::set_exif_sync_status(&mut **tx, picture_id, status).await?;
        return Ok(drain);
    }
    Ok(false)
}
