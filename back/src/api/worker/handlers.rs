use crate::api::middleware::auth_worker::AuthWorker;
use crate::api::worker::models::{ClaimJobResponse, CompleteJobRequest, FailJobRequest};
use crate::domain::job::{EditPictureConfig, ExifEdit, JobConfig, JobStatus, JobType};
use crate::domain::picture::ExifSyncStatus;
use crate::domain::user_settings::VersioningMode;
use crate::infra::error::{AppError, map_sqlx_error};
use crate::infra::s3;
use crate::repository::job::JobRepository;
use crate::repository::picture::PictureRepository;
use crate::repository::picture_version::PictureVersionRepository;
use crate::repository::pipeline::PipelineRepository;
use crate::repository::share_announcement::ShareAnnouncementRepository;
use crate::repository::user_settings::UserSettingsRepository;
use crate::state::AppState;
use archypix_common::transfer::ClaimQuery;
use archypix_common::transfer::PresignedWrites;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use tracing::debug;
use uuid::Uuid;

pub async fn claim_next_job(
    auth: AuthWorker,
    State(state): State<AppState>,
    Query(query): Query<ClaimQuery>,
) -> Result<Json<Option<ClaimJobResponse>>, AppError> {
    let Some(job) = JobRepository::claim_next(&state.db, auth.worker_id(), &query.types).await?
    else {
        return Ok(Json(None));
    };

    debug!(
        worker = auth.worker_id(),
        job_type = job.job_type.to_string(),
        job_id = %job.id,
        "worker: claim_next_job"
    );

    // claim_token was generated and stored by claim_next; forward it to the worker.
    let claim_token = job.claim_token.ok_or_else(|| {
        AppError::InternalServerError("claimed job has no claim_token".to_string())
    })?;

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
        .presign_get_worker(&state.config.s3_bucket_pictures, &original_key)
        .await?;

    let config = job
        .typed_config()
        .map_err(|e| AppError::InternalServerError(format!("failed to parse job config: {e}")))?;

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
                    &state.config.s3_bucket_pictures,
                    &original_key,
                    &state.config.s3_bucket_versions,
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
                .presign_put_worker(&state.config.s3_bucket_small, &thumb_key)
                .await?,
            state
                .storage
                .presign_put_worker(&state.config.s3_bucket_medium, &thumb_key)
                .await?,
            state
                .storage
                .presign_put_worker(&state.config.s3_bucket_large, &thumb_key)
                .await?,
        ),
        JobConfig::EditPicture(edit_cfg) => {
            let output = state
                .storage
                .presign_put_worker(&state.config.s3_bucket_pictures, &original_key)
                .await?;
            if edit_cfg.visual.is_some() {
                PresignedWrites::edit_with_visual(
                    output,
                    state
                        .storage
                        .presign_put_worker(&state.config.s3_bucket_small, &thumb_key)
                        .await?,
                    state
                        .storage
                        .presign_put_worker(&state.config.s3_bucket_medium, &thumb_key)
                        .await?,
                    state
                        .storage
                        .presign_put_worker(&state.config.s3_bucket_large, &thumb_key)
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
    })))
}

pub async fn complete_job(
    auth: AuthWorker,
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
    Json(body): Json<CompleteJobRequest>,
) -> Result<StatusCode, AppError> {
    debug!(worker = auth.worker_id(), job_id = %job_id, "worker: complete_job");

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

    // Update picture columns from worker output. Width/height come from the top-level fields
    if let (Some(extracted), Some(pid)) = (&body.exif, picture_id) {
        let e = &extracted.exif;
        let camera_patch = serde_json::to_value(&e.camera).ok();
        PictureRepository::update_from_worker(
            &mut *tx,
            pid,
            body.width,
            body.height,
            e.captured_at,
            e.gps_lat,
            e.gps_lng,
            e.gps_alt,
            e.orientation,
            body.blurhash.as_deref(),
            camera_patch,
            body.file_size,
            body.file_hash.as_deref(),
        )
        .await?;
    } else if let Some(pid) = picture_id {
        // No EXIF (edit_picture or non-initial gen_thumbnail): still update
        // thumbnails_generated_at, blurhash, file_size, file_hash, and decoded dimensions.
        PictureRepository::update_after_processing(
            &mut *tx,
            pid,
            body.thumbnails_generated,
            body.blurhash.as_deref(),
            body.file_size,
            body.file_hash.as_deref(),
            body.width,
            body.height,
        )
        .await?;
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
    let mut reannounce_owner: Option<Uuid> = None;
    if job.job_type == JobType::GenThumbnail {
        if let Some(pid) = picture_id {
            if ShareAnnouncementRepository::is_picture_tracked(&mut *tx, pid).await? {
                PipelineRepository::invalidate(&mut *tx, &[pid]).await?;
                reannounce_owner = Some(job.owner_id);
            }
        }
    }

    // EXIF reconcile convergence (§5): the file now equals this job's `new` state. If the DB still
    // equals `new`, the picture is in sync; otherwise a newer edit moved it on while we processed —
    // enqueue a follow-up that brings the file from `new` to the current DB row. The just-completed
    // job is now `completed`, so the in-flight unique index permits the new pending insert.
    let mut requeue = false;
    if let (Some(cfg), Some(pid)) = (&edit_cfg, picture_id) {
        if let Some(edit) = &cfg.exif {
            let picture = PictureRepository::find_by_id(&mut *tx, pid)
                .await?
                .ok_or(AppError::NotFound)?;
            let new_state = edit.new_state();
            let current = picture.full_exif();
            if current == new_state {
                PictureRepository::set_exif_sync_status(&mut *tx, pid, ExifSyncStatus::Synced)
                    .await?;
            } else {
                let (set, clear) = new_state.diff_to(&current);
                let follow_up = JobConfig::EditPicture(EditPictureConfig {
                    picture_id: pid,
                    exif: Some(ExifEdit {
                        set,
                        clear,
                        previous: new_state,
                    }),
                    visual: None,
                });
                JobRepository::create(&mut *tx, picture.local_user_id, Some(pid), &follow_up, None)
                    .await?;
                requeue = true;
            }
        }
    }

    tx.commit().await.map_err(map_sqlx_error)?;

    // A follow-up reconcile was enqueued — wake the worker fleet indirectly via the pipeline is not
    // needed (workers poll), but waking the pipeline lets dependent rules re-evaluate promptly.
    // Debounced: worker completions arrive one-per-picture and should collapse into one run.
    if requeue {
        if let Some(job) = &completed {
            state.pipeline_waker.wake_debounced(job.owner_id);
            return Ok(StatusCode::NO_CONTENT);
        }
    }

    // Wake (post-commit) so the pipeline re-announces the freshly-hashed/thumbnailed picture.
    // Debounced for the same reason — a batch upload's thumbnails complete in a burst.
    if let Some(owner_id) = reannounce_owner {
        state.pipeline_waker.wake_debounced(owner_id);
    }

    Ok(StatusCode::NO_CONTENT)
}

pub async fn fail_job(
    auth: AuthWorker,
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
    Json(body): Json<FailJobRequest>,
) -> Result<StatusCode, AppError> {
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

    // Revert on permanent failure (§4.3): the file was never overwritten (upload is the last
    // fallible step), so roll the DB back to `previous` and re-sync at the old state — but only if
    // the row still equals this job's `new` (else a newer edit owns the state).
    if job.status == JobStatus::Failed && job.job_type == JobType::EditPicture {
        if let (Ok(JobConfig::EditPicture(cfg)), Some(pid)) = (job.typed_config(), job.picture_id) {
            if let Some(edit) = cfg.exif {
                let picture = PictureRepository::find_by_id(&state.db, pid)
                    .await?
                    .ok_or(AppError::NotFound)?;
                if picture.full_exif() == edit.new_state() {
                    PictureRepository::write_exif_snapshot(
                        &state.db,
                        pid,
                        &edit.previous,
                        ExifSyncStatus::Synced,
                    )
                    .await?;
                    sqlx::query!(
                        "UPDATE jobs SET error_message = $2 WHERE id = $1",
                        job_id,
                        format!("{} (DB reverted to previous EXIF)", body.error),
                    )
                    .execute(&state.db)
                    .await
                    .map_err(map_sqlx_error)?;
                    // A revert is itself a metadata change — re-dirty + wake the pipeline.
                    // Debounced: this is a worker-driven completion path.
                    state.pipeline_waker.wake_debounced(job.owner_id);
                }
            }
        }
    }
    Ok(StatusCode::NO_CONTENT)
}
