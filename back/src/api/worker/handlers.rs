use crate::api::middleware::auth_worker::AuthWorker;
use crate::api::worker::models::{ClaimJobResponse, CompleteJobRequest, FailJobRequest};
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

#[tracing::instrument(skip(auth, state, body, headers), fields(worker = auth.worker_id(), job_id = %job_id))]
pub async fn complete_job(
    auth: AuthWorker,
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
    Json(body): Json<CompleteJobRequest>,
) -> Result<StatusCode, AppError> {
    // The worker injects its job span's context; reparent so completion is child of the job span.
    let cx = observability::extract_from_headers(&headers);
    tracing::Span::current().set_parent(cx);

    // Read job outside the transaction to get type/config early (fail fast on
    // corrupt JSONB). The claim_token guard inside the UPDATE makes this safe.
    let job = JobRepository::find_by_id(&state.db, job_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let picture_id = job.picture_id;

    // Pre-parse edit_picture config so we fail before touching the DB if it is corrupt.
    let edit_cfg = if job.job_type == JobType::EditPicture {
        match job.typed_config() {
            Ok(JobConfig::EditPicture(c)) => Some(c),
            Ok(_) => None,
            Err(e) => {
                return Err(AppError::InternalServerError(format!(
                    "failed to parse job config: {e}"
                )));
            }
        }
    } else {
        None
    };

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(format!("failed to begin tx: {e}")))?;

    // Update picture columns from worker output.
    if let Some(pid) = picture_id {
        match (&job.job_type, &body.exif) {
            // Extraction path (initial ingest / external overwrite): file state becomes authoritative.
            (JobType::GenThumbnail, Some(extracted)) => {
                PictureRepository::update_from_worker(
                    &mut *tx,
                    pid,
                    &extracted.exif,
                    body.width,
                    body.height,
                    body.blurhash.as_deref(),
                    body.file_size,
                    body.file_hash.as_deref(),
                    body.content_hash.as_deref(),
                )
                .await?;
            }
            // Edit path: keep DB EXIF as-is; only processing metadata lands here.
            _ => {
                PictureRepository::update_after_processing(
                    &mut *tx,
                    pid,
                    body.thumbnails_generated,
                    body.blurhash.as_deref(),
                    body.file_size,
                    body.file_hash.as_deref(),
                    body.content_hash.as_deref(),
                    body.width,
                    body.height,
                )
                .await?;
            }
        }
    }

    let result = serde_json::json!({
        "worker_id": auth.worker_id(),
        "has_exif": body.exif.is_some(),
        "has_blurhash": body.blurhash.is_some(),
        "thumbnails_generated": body.thumbnails_generated,
    });

    // Mark job complete — returns None if claim_token mismatch or wrong status.
    let completed = JobRepository::complete(&mut *tx, job_id, body.claim_token, result).await?;
    if completed.is_none() {
        tx.rollback().await.ok();
        return Err(AppError::Conflict(
            "job is no longer in processing state or claim token does not match".to_string(),
        ));
    }

    // Re-announce on thumbnail completion (D): a `gen_thumbnail` job is usually what first computes
    // `file_hash`/`blurhash`/`thumbnails_generated_at`, but the picture may have already been
    // announced (with those fields still null) before the worker finished. The `updated_at` trigger
    // has bumped the row past its recorded `announced_updated_at`, so re-marking it dirty makes the
    // pipeline's delta deliver the refreshed metadata. Gated on tracking-table membership: a picture
    // that was never announced has nothing to refresh.
    // A `gen_thumbnail` completion is also where `content_hash` first lands, so wake the owner's
    // pipeline (feature 11) to run the dedup reconciler — a freshly-ingested picture may dedupe
    // against an existing copy. Done for every gen_thumbnail completion, not only tracked ones.
    let mut reannounce_owner: Option<Uuid> = None;
    if job.job_type == JobType::GenThumbnail {
        if let Some(pid) = picture_id {
            if ShareAnnouncementRepository::is_picture_tracked(&mut *tx, pid).await? {
                PipelineRepository::invalidate(&mut *tx, &[pid]).await?;
            }
            reannounce_owner = Some(job.owner_id);
        }
    }

    // State-based EXIF convergence: record what the worker read back from the file, then compare the
    // DB against the target this job wrote — never against the read-back (feature 31 §3.3).
    let mut needs_exif_drain = false;
    if let (Some(cfg), Some(pid)) = (&edit_cfg, picture_id) {
        if let Some(edit) = &cfg.exif {
            if let Some(extracted) = &body.exif {
                PictureRepository::set_file_exif(&mut *tx, pid, &extracted.exif).await?;
            }
            let picture = PictureRepository::find_by_id(&mut *tx, pid)
                .await?
                .ok_or(AppError::NotFound)?;
            let (status, drain) = if picture.full_exif() == edit.target {
                (ExifSyncStatus::Synced, false)
            } else {
                (ExifSyncStatus::PendingJobCreation, true)
            };
            PictureRepository::set_exif_sync_status(&mut *tx, pid, status).await?;
            needs_exif_drain = drain;
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

#[tracing::instrument(skip(auth, state, body, headers), fields(worker = auth.worker_id(), job_id = %job_id))]
pub async fn fail_job(
    auth: AuthWorker,
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
    Json(body): Json<FailJobRequest>,
) -> Result<StatusCode, AppError> {
    let cx = observability::extract_from_headers(&headers);
    tracing::Span::current().set_parent(cx);
    debug!(
        worker = auth.worker_id(),
        job_id = %job_id,
        permanent = body.permanent,
        error = %body.error,
        "worker: fail_job"
    );
    let updated = JobRepository::fail(
        &state.db,
        job_id,
        body.claim_token,
        &body.error,
        body.permanent,
    )
    .await?;
    let Some(job) = updated else {
        return Err(AppError::Conflict(
            "job is no longer in processing state or claim token does not match".to_string(),
        ));
    };

    // Permanent EXIF reconcile failure (feature 31 §3.3/§6): keep the DB edit and surface the
    // divergence for an explicit user action. A file that cannot carry the metadata is terminal.
    if job.status == JobStatus::Failed && job.job_type == JobType::EditPicture {
        if let (Ok(JobConfig::EditPicture(cfg)), Some(pid)) = (job.typed_config(), job.picture_id) {
            if cfg.exif.is_some() {
                let status = if body.unsupported {
                    ExifSyncStatus::Unsupported
                } else {
                    ExifSyncStatus::WriteFailed
                };
                PictureRepository::set_exif_sync_status(&state.db, pid, status).await?;
            }
        }
    }
    Ok(StatusCode::NO_CONTENT)
}
