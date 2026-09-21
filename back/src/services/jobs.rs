use crate::clients::federation::FederationClient;
use crate::domain::job::{
    EditPictureConfig, ExifEdit, ExifField, FullExif, GenThumbnailConfig, Job, JobConfig,
};
use crate::domain::picture::{ExifSyncStatus, Picture};
use crate::domain::routine::RecheckScope;
use crate::infra::redis::Cache;
use crate::repository::job::JobRepository;
use crate::repository::picture::{PictureRepository, ResolvedSelection};
use crate::repository::share::IncomingShareRepository;
use crate::services::aggregate::DryRun;
use archypix_common::error::{AppError, map_sqlx_error};
use archypix_common::routine::RoutineHandle;
use archypix_common::mime::{supports_exif, supports_video};
use archypix_common::settings::Settings;
use chrono::NaiveDateTime;
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;

/// Enqueue a thumbnail + EXIF extraction job for a picture.
///
/// Pass `is_initial = true` for a run that (re-)extracts EXIF from the file (first upload, or a
/// WebDAV overwrite whose bytes changed). An extraction is keyed on the bytes it will read, so a
/// retried enqueue of the same version returns the job already in flight while an overwrite with
/// new bytes always gets its own job — even while the previous one is still `processing` on the old
/// bytes. Without a hash the key degrades to the picture, which is enough for the upload path
/// (the row is brand new, so nothing can collide).
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
        metadata_only: false,
    });
    let idempotency = match (is_initial, file_hash) {
        (true, Some(hash)) => Some(format!("gen_thumbnail_extract:{picture_id}:{hash}")),
        (true, None) => Some(format!("gen_thumbnail_initial:{picture_id}")),
        (false, _) => None,
    };
    match idempotency {
        Some(key) => {
            JobRepository::create_idempotent(ex, owner_id, Some(picture_id), &config, &key)
                .await
                .map(|c| c.job)
        }
        None => JobRepository::create(ex, owner_id, Some(picture_id), &config).await,
    }
}

/// The status a freshly ingested row starts in (feature 33 §4.1). An extraction job follows, except
/// for a format that carries no readable metadata at all — pre-stamping those avoids a pointless
/// edit-refusal window. An unknown MIME is `Extracting`: the worker attempts the read anyway and
/// resolves it to a real verdict.
pub fn ingest_exif_status(mime_type: Option<&str>) -> ExifSyncStatus {
    match mime_type {
        Some(m) if !supports_exif(m) && !supports_video(m) => ExifSyncStatus::UnsupportedMime,
        _ => ExifSyncStatus::Extracting,
    }
}

/// The status a physical copy inherits (feature 33 §4.1): the source's verdict about the identical
/// bytes, except that a source nothing ever read successfully has no verdict to lend — the copy
/// takes `extract_failed`, which keeps edits allowed and lets a re-extract settle it.
pub fn copy_exif_status(source: ExifSyncStatus) -> ExifSyncStatus {
    if source.never_read() {
        ExifSyncStatus::ExtractFailed
    } else {
        source
    }
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
    let mut enqueued: Vec<Uuid> = Vec::new();
    for (picture_id, owner_id) in targets {
        // No idempotency key: an admin regen is content-agnostic, and `find_for_thumbnail_regen`
        // already excludes pictures with an in-flight `gen_thumbnail`.
        let config = JobConfig::GenThumbnail(GenThumbnailConfig {
            picture_id,
            is_initial: reextract_exif,
            metadata_only: false,
        });
        JobRepository::create(db, owner_id, Some(picture_id), &config).await?;
        enqueued.push(picture_id);
    }
    // A re-extraction is authoritative, so the rows must refuse edits until it lands (§8, §12 E).
    if reextract_exif {
        PictureRepository::set_exif_sync_status_bulk(
            db,
            &enqueued,
            None,
            ExifSyncStatus::Extracting,
        )
        .await?;
    }
    Ok(enqueued.len())
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
#[tracing::instrument(skip(db, cache, waker, set, clear), fields(user_id = %user_id))]
pub async fn edit_pictures_exif(
    db: &PgPool,
    cache: &dyn Cache,
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
        // 04 §11.2: reject edits until the extraction has landed, so it can't race/overwrite the
        // edit. `extracting` is the only state that refuses them (feature 33 §4.1).
        if picture.exif_sync_status == ExifSyncStatus::Extracting {
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
        let new_state = picture.full_exif().applied(&set, &clear);

        // The one derivation feature 33 §10 keeps: re-checking `supports_exif` on the already-loaded
        // row is free, and closes the window between an allowlist change and the recheck sweep. This
        // is where a video first becomes `unsupported_mime` (§4.3). Both terminal verdicts suppress
        // job creation, and a file verdict is never relabelled as a MIME one.
        //
        // An unknown MIME is treated as writable, matching `ingest_exif_status` and the worker's
        // read: absence of a MIME is a gap in our metadata, not evidence about the file, and
        // `unsupported_mime` is terminal — stamping it from missing information would suppress
        // every future job on a guess. Attempt the write and let the worker return a real verdict.
        let writable = picture
            .mime_type
            .as_deref()
            .map(supports_exif)
            .unwrap_or(true);
        let supported = writable && !picture.exif_sync_status.suppresses_jobs();
        let status = match picture.exif_sync_status {
            _ if supported => ExifSyncStatus::Pending,
            ExifSyncStatus::UnsupportedFile => ExifSyncStatus::UnsupportedFile,
            _ => ExifSyncStatus::UnsupportedMime,
        };

        PictureRepository::write_exif_snapshot(&mut *tx, picture.id, &new_state, status).await?;
        outcome.updated.push(picture.id);

        if !supported {
            outcome.unsupported.push(picture.id);
            continue;
        }

        // §5 concurrency: at most one in-flight reconcile per picture.
        if let Some(job_id) = enqueue_if_absent_edit(&mut tx, user_id, picture.id).await? {
            outcome.jobs.push(job_id);
        }
    }

    tx.commit()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;

    // An edit to `captured_at` moves every covering tag's derived range (feature 34 §4).
    crate::services::tag_metadata::bust_cache(cache, user_id).await;
    // A metadata change re-dirties the picture (date/GPS rules, segments, announcements). Debounced:
    // an EXIF edit reconciles via a worker, and a batch edit produces a per-picture wake burst that
    // should collapse into one pipeline run.
    waker.trigger_debounced(user_id);
    Ok(outcome)
}

/// The EXIF reconcile job config for a picture. `target` is a placeholder: the backend rebinds it
/// to the live DB snapshot when a worker claims the job (feature 31 §3.3).
fn exif_reconcile_config(picture_id: Uuid) -> JobConfig {
    JobConfig::EditPicture(EditPictureConfig {
        picture_id,
        exif: Some(ExifEdit::default()),
        visual: None,
    })
}

/// Ensure there is at most one in-flight EXIF reconcile job for a picture.
///
/// A pending job needs no folding: it picks up the latest DB snapshot at claim-time. A job already
/// `processing` wrote an older snapshot, and its completion handler stamps `pending_job_creation`
/// so the drain enqueues the follow-up.
async fn enqueue_if_absent_edit(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    user_id: Uuid,
    picture_id: Uuid,
) -> Result<Option<Uuid>, AppError> {
    if JobRepository::find_inflight_edit(&mut **tx, picture_id)
        .await?
        .is_some()
    {
        return Ok(None);
    }

    let config = exif_reconcile_config(picture_id);
    let job = JobRepository::create(&mut **tx, user_id, Some(picture_id), &config).await?;
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
    if !matches!(
        picture.exif_sync_status,
        ExifSyncStatus::Pending | ExifSyncStatus::WriteFailed
    ) {
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
    let config = exif_reconcile_config(picture_id);
    PictureRepository::set_exif_sync_status(db, picture_id, ExifSyncStatus::Pending).await?;
    let job = JobRepository::create(db, user_id, Some(picture_id), &config).await?;
    // Debounced: EXIF resync is a worker-driven reconcile path.
    waker.trigger_debounced(user_id);
    Ok(job)
}

/// Re-read a picture's EXIF from the file (feature 33 §8). Distinct from [`resync_picture_exif`],
/// which pushes the DB the other way: extraction makes the **file** authoritative (31 §5), so it is
/// refused while the row holds an unsynced DB edit that would be silently discarded.
///
/// Carries no idempotency key: a re-extract is content-agnostic (it reads whatever is on disk now),
/// so there is nothing to key on, and an explicit user action deserves an explicit `409` rather than
/// the silent dedupe an idempotency key would give it. An in-flight `gen_thumbnail` guard does that.
#[tracing::instrument(skip(db, waker), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn reextract_picture_exif(
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
    if matches!(
        picture.exif_sync_status,
        ExifSyncStatus::Pending | ExifSyncStatus::PendingJobCreation | ExifSyncStatus::WriteFailed
    ) {
        return Err(AppError::Conflict(
            "picture has EXIF edits that have not reached the file; sync or revert them first"
                .into(),
        ));
    }
    if JobRepository::has_inflight_thumbnail(db, picture_id).await? {
        return Err(AppError::Conflict(
            "an extraction job is already in flight for this picture".into(),
        ));
    }

    let mut tx = db.begin().await.map_err(map_sqlx_error)?;
    let config = JobConfig::GenThumbnail(GenThumbnailConfig {
        picture_id,
        is_initial: true,
        metadata_only: false,
    });
    let job = JobRepository::create(&mut *tx, user_id, Some(picture_id), &config).await?;
    PictureRepository::set_exif_sync_status(&mut *tx, picture_id, ExifSyncStatus::Extracting)
        .await?;
    tx.commit().await.map_err(map_sqlx_error)?;
    waker.trigger_debounced(user_id);
    Ok(job)
}

/// One bounded tick of the admin EXIF recheck sweep (feature 33 §8): enqueue a re-extraction for up
/// to `limit` rows in `scope` after the keyset cursor `after`, stamping each `extracting`. Returns
/// the number enqueued and the cursor to resume from; a short count means the worklist is drained.
///
/// The cursor is what terminates a sweep: a re-extracted row returns to a status the scope may
/// match (always, for `synced`), and without it the sweep would pick it up again. Rows holding
/// unsynced DB edits are never in scope, so no edit can be discarded. An already-thumbnailed row
/// reads metadata only (feature 36 §5).
#[tracing::instrument(skip(db, mime_types))]
pub async fn recheck_exif_batch(
    db: &PgPool,
    scope: RecheckScope,
    mime_types: Option<&[String]>,
    sweep_id: Uuid,
    after: Option<(NaiveDateTime, Uuid)>,
    limit: i64,
) -> Result<(usize, Option<(NaiveDateTime, Uuid)>), AppError> {
    let targets = PictureRepository::find_by_exif_sync_status(
        db,
        &[scope.status()],
        mime_types,
        after,
        limit,
    )
    .await?;
    let cursor = targets.last().map(|t| (t.ingested_at, t.id)).or(after);
    let mut enqueued = Vec::new();
    for t in targets {
        let config = JobConfig::GenThumbnail(GenThumbnailConfig {
            picture_id: t.id,
            is_initial: true,
            metadata_only: t.has_thumbnails,
        });
        let key = format!("gen_thumbnail_reextract:{}:{sweep_id}", t.id);
        JobRepository::create_idempotent(db, t.local_user_id, Some(t.id), &config, &key).await?;
        enqueued.push(t.id);
    }
    PictureRepository::set_exif_sync_status_bulk(db, &enqueued, None, ExifSyncStatus::Extracting)
        .await?;
    Ok((enqueued.len(), cursor))
}

/// Reset a picture's DB EXIF to its persisted physical-file snapshot (`file_exif`), the user's way
/// out of a `write_failed` divergence (feature 31 §4). Returns the updated row.
#[tracing::instrument(skip(db, cache, waker), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn revert_picture_exif_to_file(
    db: &PgPool,
    cache: &dyn Cache,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    picture_id: Uuid,
) -> Result<Picture, AppError> {
    let picture = PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if picture.local_user_id != user_id || !picture.is_owned() {
        return Err(AppError::NotFound);
    }
    let Some(file_exif) = picture.file_exif.as_ref() else {
        return Err(AppError::Conflict(
            "picture has no physical EXIF snapshot yet".into(),
        ));
    };
    // A reconcile in flight would write the pre-revert target back and re-diverge the row.
    if JobRepository::find_inflight_edit(db, picture_id)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "a reconcile job is already in flight for this picture".into(),
        ));
    }
    PictureRepository::write_exif_snapshot(db, picture_id, &file_exif.0, ExifSyncStatus::Synced)
        .await?;
    crate::services::tag_metadata::bust_cache(cache, user_id).await;
    waker.trigger_debounced(user_id);
    PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)
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
    if dry_run {
        let affected = PictureRepository::count_selection(db, user_id, sel).await?;
        let owned_total = PictureRepository::count_owned_selection(db, user_id, sel).await?;
        let owned_unsupported =
            PictureRepository::count_owned_unsupported_selection(db, user_id, sel).await?;
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

    // ── Owned: deferred write-through, set-based (one statement, status partition) ──
    let mut tx = db.begin().await.map_err(map_sqlx_error)?;
    let counts = PictureRepository::batch_apply_exif_owned_selection(
        &mut *tx,
        user_id,
        sel,
        &set,
        &null_clear,
    )
    .await?;
    let (edited, unsupported) = (counts.edited, counts.unsupported);
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
                        cache,
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

    crate::services::tag_metadata::bust_cache(cache, user_id).await;
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
        let config = exif_reconcile_config(picture_id);
        let mut tx = db.begin().await.map_err(map_sqlx_error)?;
        JobRepository::create(&mut *tx, owner_id, Some(picture_id), &config).await?;
        PictureRepository::set_exif_sync_status(&mut *tx, picture_id, ExifSyncStatus::Pending)
            .await?;
        tx.commit().await.map_err(map_sqlx_error)?;
        created += 1;
    }
    Ok(created)
}
