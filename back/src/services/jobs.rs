use crate::domain::job::{
    EditPictureConfig, ExifEdit, ExifField, ExifOverrides, GenThumbnailConfig, Job, JobConfig,
};
use crate::domain::picture::ExifSyncStatus;
use crate::infra::error::AppError;
use crate::infra::pipeline::PipelineWaker;
use crate::repository::job::JobRepository;
use crate::repository::picture::PictureRepository;
use archypix_common::mime::supports_exif;
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;

/// Enqueue a thumbnail + EXIF extraction job for a picture.
///
/// Pass `is_initial = true` for the first-ever run (worker also extracts EXIF).
/// Pass `is_initial = false` to re-generate thumbnails without EXIF re-extraction.
pub async fn enqueue_thumbnail_job<'e, E>(
    ex: E,
    owner_id: Uuid,
    picture_id: Uuid,
    is_initial: bool,
) -> Result<Job, AppError>
where
    E: Executor<'e, Database = Postgres>,
{
    let config = JobConfig::GenThumbnail(GenThumbnailConfig {
        picture_id,
        is_initial,
    });
    let idempotency = if is_initial {
        Some(format!("gen_thumbnail_initial:{picture_id}"))
    } else {
        None
    };
    JobRepository::create(
        ex,
        owner_id,
        Some(picture_id),
        &config,
        idempotency.as_deref(),
    )
    .await
}

pub async fn get_job(db: &PgPool, job_id: Uuid, user_id: Uuid) -> Result<Job, AppError> {
    let job = JobRepository::find_by_id(db, job_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if job.owner_id != user_id {
        return Err(AppError::NotFound);
    }
    Ok(job)
}

pub async fn list_picture_jobs(
    db: &PgPool,
    picture_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<Job>, AppError> {
    let picture = PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if picture.local_user_id != user_id {
        return Err(AppError::NotFound);
    }
    JobRepository::list_by_picture(db, picture_id, user_id).await
}

/// Per-picture outcome of an EXIF edit batch.
pub struct ExifEditOutcome {
    /// Pictures whose DB row was updated.
    pub updated: Vec<Uuid>,
    /// Reconcile job ids enqueued (one per supported, non-folded picture).
    pub jobs: Vec<Uuid>,
    /// Pictures whose format cannot embed EXIF — DB-only, no job (terminal divergence).
    pub unsupported: Vec<Uuid>,
}

/// Edit the EXIF of one or more owned pictures (write-through Phase 1, §4.1).
///
/// Validates the whole batch first (ownership, owned-only, not still-extracting, set/clear),
/// then in a single transaction applies the `set`/`clear` delta to every row, bumps `updated_at`,
/// resets `last_pipeline_run_at`, sets `exif_sync_status`, and enqueues a reconcile job per the §5
/// concurrency rule. The pipeline is woken once after commit.
pub async fn edit_pictures_exif(
    db: &PgPool,
    waker: &PipelineWaker,
    user_id: Uuid,
    picture_ids: &[Uuid],
    set: ExifOverrides,
    clear: Vec<ExifField>,
) -> Result<ExifEditOutcome, AppError> {
    if picture_ids.is_empty() {
        return Err(AppError::BadRequest("picture_ids must not be empty".into()));
    }
    let clear = validate_exif_edit(&set, clear)?;

    // ── Validate the whole batch before any mutation (reject on first violation) ──
    let mut pictures = Vec::with_capacity(picture_ids.len());
    for &id in picture_ids {
        let picture = PictureRepository::find_by_id(db, id)
            .await?
            .ok_or(AppError::NotFound)?;
        if picture.local_user_id != user_id {
            return Err(AppError::NotFound);
        }
        if !picture.is_owned() {
            return Err(AppError::BadRequest(format!(
                "Cannot edit picture {id}: received via federation"
            )));
        }
        if picture.thumbnails_generated_at.is_none() {
            return Err(AppError::Conflict(format!(
                "Picture {id} is still processing; try again once extraction completes"
            )));
        }
        pictures.push(picture);
    }

    // ── Apply + enqueue atomically ───────────────────────────────────────────────
    let mut outcome = ExifEditOutcome {
        updated: Vec::new(),
        jobs: Vec::new(),
        unsupported: Vec::new(),
    };
    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;

    for picture in &pictures {
        let previous = picture.exif_snapshot();
        let new_state = previous.applied(&set, &clear);

        // MIME preflight: a format that cannot embed EXIF gets a DB-only edit, no job.
        let supported = picture
            .mime_type
            .as_deref()
            .map(supports_exif)
            .unwrap_or(false);
        let status = if supported {
            ExifSyncStatus::Pending
        } else {
            ExifSyncStatus::Unsupported
        };

        PictureRepository::write_exif_snapshot(&mut *tx, picture.id, &new_state, status).await?;
        outcome.updated.push(picture.id);

        if !supported {
            outcome.unsupported.push(picture.id);
            continue;
        }

        // §5 concurrency: at most one in-flight reconcile per picture.
        if let Some(job_id) =
            enqueue_or_fold_edit(&mut tx, user_id, picture.id, &set, &clear, &previous).await?
        {
            outcome.jobs.push(job_id);
        }
    }

    tx.commit()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;

    // A metadata change re-dirties the picture (date/GPS rules, segments, announcements). Debounced:
    // an EXIF edit reconciles via a worker, and a batch edit produces a per-picture wake burst that
    // should collapse into one pipeline run.
    waker.wake_debounced(user_id);
    Ok(outcome)
}

/// Apply the §5 in-flight rule for one picture, inside the edit transaction.
///
/// - No in-flight job → insert one (`previous` = the synced file baseline, plus the delta).
/// - A `pending` (unclaimed) job → fold: recompute its delta against its own (unchanged) baseline so
///   it now targets the cumulative latest DB state. Returns no new job id.
/// - A `processing` job → do not enqueue; the completion handler re-enqueues. Returns no id.
async fn enqueue_or_fold_edit(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    user_id: Uuid,
    picture_id: Uuid,
    set: &ExifOverrides,
    clear: &[ExifField],
    previous: &crate::domain::job::ExifSnapshot,
) -> Result<Option<Uuid>, AppError> {
    let new_state = previous.applied(set, clear);

    if let Some(existing) = JobRepository::find_inflight_edit(&mut **tx, picture_id).await? {
        if existing.status == crate::domain::job::JobStatus::Pending {
            // Fold: keep the job's synced baseline; retarget its delta to the cumulative state.
            let baseline = match existing.typed_config() {
                Ok(JobConfig::EditPicture(cfg)) => cfg.exif.map(|e| e.previous).unwrap_or_default(),
                _ => previous.clone(),
            };
            let (fset, fclear) = baseline.diff_to(&new_state);
            let folded = JobConfig::EditPicture(EditPictureConfig {
                picture_id,
                exif: Some(ExifEdit {
                    set: fset,
                    clear: fclear,
                    previous: baseline,
                }),
                visual: None,
            });
            if JobRepository::update_config_if_pending(&mut **tx, existing.id, &folded).await? {
                return Ok(None);
            }
            // The job started processing between the find and the update — fall through to the
            // processing case (do not enqueue; completion re-enqueues).
        }
        // A `processing` job exists: DB edit already applied + status pending; do not enqueue.
        return Ok(None);
    }

    let config = JobConfig::EditPicture(EditPictureConfig {
        picture_id,
        exif: Some(ExifEdit {
            set: set.clone(),
            clear: clear.to_vec(),
            previous: previous.clone(),
        }),
        visual: None,
    });
    let job = JobRepository::create(&mut **tx, user_id, Some(picture_id), &config, None).await?;
    Ok(Some(job.id))
}

/// Manually re-enqueue a reconcile for a picture stuck in `pending` with no in-flight job
/// (the rare crash-mid-completion case). Returns the new job.
pub async fn resync_picture_exif(
    db: &PgPool,
    waker: &PipelineWaker,
    user_id: Uuid,
    picture_id: Uuid,
) -> Result<Job, AppError> {
    let picture = PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if picture.local_user_id != user_id || !picture.is_owned() {
        return Err(AppError::NotFound);
    }
    if picture.exif_sync_status != ExifSyncStatus::Pending {
        return Err(AppError::BadRequest(
            "picture is not awaiting EXIF reconcile".into(),
        ));
    }
    if JobRepository::find_inflight_edit(db, picture_id)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "a reconcile job is already in flight for this picture".into(),
        ));
    }
    // Re-enqueue a no-op delta: bring the file from its (unknown) state to the current DB row.
    // `previous` = the current DB snapshot; the worker rewrites every editable field from `set`.
    let snapshot = picture.exif_snapshot();
    let (set, clear) = crate::domain::job::ExifSnapshot::default().diff_to(&snapshot);
    let config = JobConfig::EditPicture(EditPictureConfig {
        picture_id,
        exif: Some(ExifEdit {
            set,
            clear,
            previous: crate::domain::job::ExifSnapshot::default(),
        }),
        visual: None,
    });
    let job = JobRepository::create(db, user_id, Some(picture_id), &config, None).await?;
    // Debounced: EXIF resync is a worker-driven reconcile path.
    waker.wake_debounced(user_id);
    Ok(job)
}

/// Field-level validation of an EXIF edit. Expands a GPS clear to lat+lng+alt, then rejects a field
/// that appears in both `set` and `clear`, out-of-range GPS, and an invalid orientation.
fn validate_exif_edit(
    set: &ExifOverrides,
    clear: Vec<ExifField>,
) -> Result<Vec<ExifField>, AppError> {
    // Clearing any GPS component clears all three together.
    let mut clear = clear;
    if clear
        .iter()
        .any(|f| matches!(f, ExifField::GpsLat | ExifField::GpsLng | ExifField::GpsAlt))
    {
        for f in [ExifField::GpsLat, ExifField::GpsLng, ExifField::GpsAlt] {
            if !clear.contains(&f) {
                clear.push(f);
            }
        }
    }

    for &f in &clear {
        if field_in_set(set, f) {
            return Err(AppError::BadRequest(format!(
                "field {f:?} present in both set and clear"
            )));
        }
    }
    if let Some(lat) = set.gps_lat {
        if !(-90.0..=90.0).contains(&lat) {
            return Err(AppError::BadRequest("gps_lat out of range [-90,90]".into()));
        }
    }
    if let Some(lng) = set.gps_lng {
        if !(-180.0..=180.0).contains(&lng) {
            return Err(AppError::BadRequest(
                "gps_lng out of range [-180,180]".into(),
            ));
        }
    }
    if let Some(o) = set.orientation {
        if !(1..=8).contains(&o) {
            return Err(AppError::BadRequest("orientation must be 1..=8".into()));
        }
    }
    Ok(clear)
}

fn field_in_set(set: &ExifOverrides, f: ExifField) -> bool {
    match f {
        ExifField::CapturedAt => set.captured_at.is_some(),
        ExifField::GpsLat => set.gps_lat.is_some(),
        ExifField::GpsLng => set.gps_lng.is_some(),
        ExifField::GpsAlt => set.gps_alt.is_some(),
        ExifField::Orientation => set.orientation.is_some(),
        ExifField::CameraBrand => set.camera_brand.is_some(),
        ExifField::CameraModel => set.camera_model.is_some(),
        ExifField::FocalLengthMm => set.focal_length_mm.is_some(),
        ExifField::FNumber => set.f_number.is_some(),
        ExifField::IsoSpeed => set.iso_speed.is_some(),
        ExifField::ExposureTimeNum => set.exposure_time_num.is_some(),
        ExifField::ExposureTimeDen => set.exposure_time_den.is_some(),
    }
}
