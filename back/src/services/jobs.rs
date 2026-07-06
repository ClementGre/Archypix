use crate::clients::federation::FederationClient;
use crate::domain::job::{
    EditPictureConfig, ExifEdit, ExifField, FullExif, GenThumbnailConfig, Job, JobConfig,
};
use crate::domain::picture::ExifSyncStatus;
use crate::infra::redis::Cache;
use crate::infra::routine::RoutineHandle;
use crate::repository::job::JobRepository;
use crate::repository::picture::{PictureRepository, ResolvedSelection};
use crate::repository::share::IncomingShareRepository;
use crate::services::aggregate::DryRun;
use archypix_common::error::{map_sqlx_error, AppError};
use archypix_common::mime::{supports_exif, MIME_TYPES_EXIF};
use archypix_common::settings::Settings;
use sqlx::{Executor, PgPool, Postgres};
use std::sync::Arc;
use uuid::Uuid;

/// Enqueue a thumbnail + EXIF extraction job for a picture.
///
/// Pass `is_initial = true` for a run that (re-)extracts EXIF from the file (first upload, or a
/// WebDAV overwrite whose bytes changed).
#[tracing::instrument(skip(ex), fields(owner_id = %owner_id, picture_id = %picture_id))]
pub async fn enqueue_thumbnail_job<'e, E>(
    ex: E,
    owner_id: Uuid,
    picture_id: Uuid,
    is_initial: bool,
    file_hash: Option<&str>,
) -> Result<Job, AppError>
where
    E: Executor<'e, Database = Postgres>,
{
    let config = JobConfig::GenThumbnail(GenThumbnailConfig {
        picture_id,
        is_initial,
    });
    let idempotency = match (is_initial, file_hash) {
        (true, Some(hash)) => Some(format!("gen_thumbnail_extract:{picture_id}:{hash}")),
        (true, None) => Some(format!("gen_thumbnail_initial:{picture_id}")),
        (false, _) => None,
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

/// Admin: (re)enqueue `gen_thumbnail` jobs for owned pictures (feature 11 helper).
///
/// `only_missing` restricts to pictures with a thumbnailable MIME, no thumbnail, and older than
/// 30 minutes (failed/never-run jobs); `false` targets the whole owned library (e.g. to recompute
/// `content_hash`). `reextract_exif` controls whether the job re-extracts EXIF (`is_initial`): keep
/// it `false` to recompute only thumbnails/hashes/`content_hash` without touching stored EXIF, or
/// `true` to also re-extract EXIF from the file. Pictures with an in-flight `gen_thumbnail` job are
/// skipped. Returns the number of jobs enqueued.
#[tracing::instrument(skip(db))]
pub async fn regenerate_thumbnails(
    db: &PgPool,
    only_missing: bool,
    reextract_exif: bool,
    limit: i64,
) -> Result<usize, AppError> {
    let thumbnailable: Vec<String> = archypix_common::mime::thumbnailable_mimes()
        .map(str::to_lowercase)
        .collect();
    let targets =
        PictureRepository::find_for_thumbnail_regen(db, only_missing, &thumbnailable, limit)
            .await?;
    let mut enqueued = 0usize;
    for (picture_id, owner_id) in targets {
        // No idempotency key (the initial-upload key may already exist); the in-flight guard in the
        // query prevents duplicate concurrent jobs.
        let config = JobConfig::GenThumbnail(GenThumbnailConfig {
            picture_id,
            is_initial: reextract_exif,
        });
        if JobRepository::create(db, owner_id, Some(picture_id), &config, None)
            .await
            .is_ok()
        {
            enqueued += 1;
        }
    }
    Ok(enqueued)
}

#[tracing::instrument(skip(db), fields(user_id = %user_id, job_id = %job_id))]
pub async fn get_job(db: &PgPool, job_id: Uuid, user_id: Uuid) -> Result<Job, AppError> {
    let job = JobRepository::find_by_id(db, job_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if job.owner_id != user_id {
        return Err(AppError::NotFound);
    }
    Ok(job)
}

#[tracing::instrument(skip(db), fields(user_id = %user_id, picture_id = %picture_id))]
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
#[tracing::instrument(skip(db, waker, set, clear), fields(user_id = %user_id))]
pub async fn edit_pictures_exif(
    db: &PgPool,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    picture_ids: &[Uuid],
    set: FullExif,
    clear: Vec<ExifField>,
) -> Result<ExifEditOutcome, AppError> {
    if picture_ids.is_empty() {
        return Err(AppError::BadRequest("picture_ids must not be empty".into()));
    }
    let (_empty, clear) = crate::domain::validation::validate_exif_edit(&set, vec![], clear)
        .map_err(AppError::BadRequest)?;

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
        let previous = picture.full_exif();
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
    waker.trigger_debounced(user_id);
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
    set: &FullExif,
    clear: &[ExifField],
    previous: &FullExif,
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
#[tracing::instrument(skip(db, waker), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn resync_picture_exif(
    db: &PgPool,
    waker: &RoutineHandle<Uuid>,
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
    let snapshot = picture.full_exif();
    let (set, clear) = FullExif::default().diff_to(&snapshot);
    let config = JobConfig::EditPicture(EditPictureConfig {
        picture_id,
        exif: Some(ExifEdit {
            set,
            clear,
            previous: FullExif::default(),
        }),
        visual: None,
    });
    let job = JobRepository::create(db, user_id, Some(picture_id), &config, None).await?;
    // Debounced: EXIF resync is a worker-driven reconcile path.
    waker.trigger_debounced(user_id);
    Ok(job)
}

/// Field-level validation of an EXIF edit. Expands a GPS clear to lat+lng+alt, then rejects a field
/// that appears in both `set` and `clear`, out-of-range GPS, and an invalid orientation.
/// Whether a batch EXIF edit applies locally or proposes to owners where the share allows (§6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchExifMode {
    /// Owned → write-through; received → recipient-local override.
    #[default]
    Local,
    /// Owned → write-through; received with an EXIF-edit grant → propose to owner; received without
    /// the grant → fall back to a local override.
    Suggest,
}

/// Result of a batch EXIF edit: the dry-run breakdown, or the applied per-mode counts.
pub enum ExifBatchOutcome {
    DryRun(DryRun),
    Applied {
        affected: i64,
        edited: i64,
        suggested: i64,
        local_override: i64,
        unsupported: i64,
    },
}

/// The lower-cased MIME whitelist for formats that embed EXIF (feeds the set-based partition).
fn supported_mimes() -> Vec<String> {
    MIME_TYPES_EXIF.iter().map(|m| m.to_lowercase()).collect()
}

/// Batch EXIF edit over a [`ResolvedSelection`] (feature 14 §5–§6). Owned pictures take the
/// **deferred-job** write-through (a single set-based UPDATE that stamps `pending_job_creation`; the
/// drain creates the reconcile jobs). Received pictures take the recipient-local override merge (also
/// set-based) — or, in `Suggest` mode and where the share grants editing, a propose-to-owner edit.
///
/// With `dry_run` the call returns the §6.1 affected breakdown without mutating. The federation
/// deps are only used by `Suggest`-mode proposals.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, pipeline_waker, exif_drain, cache, settings, federation, sel, set, empty, clear), fields(user_id = %user_id, dry_run))]
pub async fn batch_edit_exif_selection(
    db: &PgPool,
    pipeline_waker: &RoutineHandle<Uuid>,
    exif_drain: &RoutineHandle<()>,
    cache: &dyn Cache,
    settings: &Settings,
    federation: &FederationClient,
    user_id: Uuid,
    requester_username: &str,
    sel: &ResolvedSelection,
    set: FullExif,
    empty: Vec<ExifField>,
    clear: Vec<ExifField>,
    mode: BatchExifMode,
    dry_run: bool,
) -> Result<ExifBatchOutcome, AppError> {
    let (empty, clear) = crate::domain::validation::validate_exif_edit(&set, empty, clear)
        .map_err(AppError::BadRequest)?;
    // Where emptying == nulling the column — owned write-through and propose-to-owner — `empty` folds
    // into `clear`. Only the received-local override keeps the empty/clear distinction (empty = a
    // sticky `null` claim, clear = drop the claim so the owner's value flows through).
    let mut null_clear = clear.clone();
    for &f in &empty {
        if !null_clear.contains(&f) {
            null_clear.push(f);
        }
    }
    let mimes = supported_mimes();

    if dry_run {
        let affected = PictureRepository::count_selection(db, user_id, sel).await?;
        let owned_total = PictureRepository::count_owned_selection(db, user_id, sel).await?;
        let owned_unsupported =
            PictureRepository::count_owned_unsupported_selection(db, user_id, sel, &mimes).await?;
        let received_total = affected - owned_total;
        let suggested = if mode == BatchExifMode::Suggest {
            PictureRepository::count_selection_received_suggestable(db, user_id, sel).await?
        } else {
            0
        };
        return Ok(ExifBatchOutcome::DryRun(DryRun {
            affected,
            edited: Some(owned_total - owned_unsupported),
            suggested: Some(suggested),
            local_override: Some(received_total - suggested),
            unsupported: Some(owned_unsupported),
            ..Default::default()
        }));
    }

    // ── Owned: deferred write-through, set-based (supported + unsupported partitions) ──
    let mut tx = db.begin().await.map_err(map_sqlx_error)?;
    let edited = PictureRepository::batch_apply_exif_owned_selection(
        &mut *tx,
        user_id,
        sel,
        &set,
        &null_clear,
        true,
        &mimes,
    )
    .await? as i64;
    let unsupported = PictureRepository::batch_apply_exif_owned_selection(
        &mut *tx,
        user_id,
        sel,
        &set,
        &null_clear,
        false,
        &mimes,
    )
    .await? as i64;
    tx.commit().await.map_err(map_sqlx_error)?;
    if edited > 0 {
        // New `pending_job_creation` rows → wake the drain to create their reconcile jobs.
        exif_drain.trigger(());
    }

    // ── Received ──
    let mut suggested = 0i64;
    let mut local_override = 0i64;
    match mode {
        BatchExifMode::Local => {
            let (patch, clear_keys) =
                crate::domain::received_exif::override_patch(&set, &empty, &clear);
            local_override = PictureRepository::batch_apply_exif_received_local_selection(
                db,
                user_id,
                sel,
                &patch,
                &clear_keys,
            )
            .await? as i64;
        }
        BatchExifMode::Suggest => {
            let received =
                PictureRepository::resolve_selection_received_ids(db, user_id, sel).await?;
            for pic_id in received {
                let grant = IncomingShareRepository::find_active_exif_editable_for_picture(
                    db, pic_id, user_id,
                )
                .await?
                .is_some();
                if grant {
                    match crate::services::pictures::propose_received_exif(
                        db,
                        cache,
                        settings,
                        federation,
                        pipeline_waker,
                        user_id,
                        requester_username,
                        pic_id,
                        set.clone(),
                        null_clear.clone(),
                    )
                    .await
                    {
                        Ok(_) => suggested += 1,
                        Err(e) => {
                            tracing::warn!(picture_id = %pic_id, error = ?e, "batch EXIF: propose to owner failed; skipping");
                        }
                    }
                } else {
                    crate::services::pictures::override_received_exif(
                        db,
                        pipeline_waker,
                        user_id,
                        pic_id,
                        set.clone(),
                        empty.clone(),
                        clear.clone(),
                    )
                    .await?;
                    local_override += 1;
                }
            }
        }
    }

    // A metadata change re-dirties the pictures (date/GPS rules, segments, announcements). Owned and
    // received-local set-based paths reset `last_pipeline_run_at`; the per-picture received paths
    // wake on their own. Debounced: a batch produces a burst that should collapse into one run.
    pipeline_waker.trigger_debounced(user_id);

    Ok(ExifBatchOutcome::Applied {
        affected: edited + unsupported + suggested + local_override,
        edited,
        suggested,
        local_override,
        unsupported,
    })
}

/// Create the deferred `edit_picture` reconcile jobs for up to `limit` pictures stamped
/// `pending_job_creation` (feature 14 §5). Mirrors the resync no-op edit: the worker rewrites every
/// editable field from the current DB snapshot. Flips each picture to `pending`. Returns the count
/// of jobs created.
#[tracing::instrument(skip(db))]
pub async fn create_deferred_exif_jobs(db: &PgPool, limit: i64) -> Result<usize, AppError> {
    let pending = PictureRepository::find_pending_job_creation(db, limit).await?;
    let mut created = 0usize;
    for (picture_id, owner_id) in pending {
        let Some(picture) = PictureRepository::find_by_id(db, picture_id).await? else {
            continue;
        };
        // Bring the file from its (unknown) state to the current DB row: previous = empty, set =
        // the full snapshot. Identical shape to a manual resync.
        let snapshot = picture.full_exif();
        let (set, clear) = FullExif::default().diff_to(&snapshot);
        let config = JobConfig::EditPicture(EditPictureConfig {
            picture_id,
            exif: Some(ExifEdit {
                set,
                clear,
                previous: FullExif::default(),
            }),
            visual: None,
        });
        let mut tx = db.begin().await.map_err(map_sqlx_error)?;
        JobRepository::create(&mut *tx, owner_id, Some(picture_id), &config, None).await?;
        PictureRepository::set_exif_sync_status(&mut *tx, picture_id, ExifSyncStatus::Pending)
            .await?;
        tx.commit().await.map_err(map_sqlx_error)?;
        created += 1;
    }
    Ok(created)
}
