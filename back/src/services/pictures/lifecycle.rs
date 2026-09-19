use crate::domain::picture::Picture;
use crate::infra::redis::Cache;
use archypix_common::routine::RoutineHandle;
use crate::infra::s3::{self, Storage};
use crate::infra::settings::keys;
use crate::repository::dedup::DedupRepository;
use crate::repository::picture::PictureRepository;
use crate::repository::picture_version::PictureVersionRepository;
use crate::repository::tag::TagRepository;
use archypix_common::error::{AppError, map_sqlx_error};
use archypix_common::settings::Settings;
use sqlx::PgPool;
use uuid::Uuid;

/// Snapshot a picture's current original bytes as a new `picture_version` before a WebDAV
/// overwrite, per the user's `versioning_mode` (06_webdav.md §7.3):
///
/// - `None` → never snapshot (overwrite in place);
/// - `OriginalCopy` → snapshot only the first time (preserve the pristine original, once);
/// - `FullVersioning` → snapshot before every overwrite.
///
/// Reuses the version-snapshot machinery of the worker edit path: S3 copy first (no DB record
/// exists yet, so it is safe outside a transaction), then the version row in a transaction so
/// `version_number` is computed and stored atomically. Returns whether a snapshot was taken.
#[tracing::instrument(skip(db, storage, settings, picture), fields(picture_id = %picture.id))]
pub async fn snapshot_version_on_overwrite(
    db: &PgPool,
    storage: &dyn Storage,
    settings: &Settings,
    versioning_mode: crate::domain::user_settings::VersioningMode,
    picture: &Picture,
) -> Result<bool, AppError> {
    use crate::domain::user_settings::VersioningMode;
    let snapshot = match versioning_mode {
        VersioningMode::None => false,
        VersioningMode::OriginalCopy => {
            !PictureVersionRepository::has_versions(db, picture.id).await?
        }
        VersioningMode::FullVersioning => true,
    };
    if !snapshot {
        return Ok(false);
    }

    let version_id = Uuid::new_v4();
    storage
        .copy_object(
            &settings.get(keys::S3_BUCKET_PICTURES),
            &s3::picture_key(picture.local_user_id, picture.id),
            &settings.get(keys::S3_BUCKET_VERSIONS),
            &s3::version_key(picture.local_user_id, picture.id, version_id),
        )
        .await?;

    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;
    let version_num = PictureVersionRepository::next_version_number(&mut *tx, picture.id).await?;
    PictureVersionRepository::create(
        &mut *tx,
        version_id,
        picture.id,
        version_num,
        picture.file_size,
        picture.mime_type.as_deref(),
    )
    .await?;
    tx.commit().await.map_err(map_sqlx_error)?;
    Ok(true)
}

/// Soft-delete a picture the user holds (owned or received), setting `deleted_reason = 'manual'`
/// (09 §5). The row is re-dirtied and the pipeline woken: for an **owned** picture this re-announces
/// it to recipients carrying the owner-deletion lifecycle flag (it stays in share coverage until the
/// purge sweep removes it); for a **received** picture the delete is purely local (never announced,
/// never affects downstream relay). Returns the updated picture.
#[tracing::instrument(skip(db, cache, waker), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn trash_picture(
    db: &PgPool,
    cache: &dyn Cache,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    picture_id: Uuid,
) -> Result<Picture, AppError> {
    set_trashed(db, cache, waker, user_id, picture_id, true).await
}

/// Restore a soft-deleted picture (clear `deleted_at`/`deleted_reason`). For an owned picture this
/// re-announces with the lifecycle flag cleared (09 §5.1). Returns the updated picture.
#[tracing::instrument(skip(db, cache, waker), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn restore_picture(
    db: &PgPool,
    cache: &dyn Cache,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    picture_id: Uuid,
) -> Result<Picture, AppError> {
    set_trashed(db, cache, waker, user_id, picture_id, false).await
}

/// Result of a batch trash/restore: the dry-run count, or the applied count.
pub enum TrashBatchOutcome {
    DryRun(crate::services::aggregate::DryRun),
    Applied { affected: i64 },
}

/// Batch soft-delete / restore over a [`ResolvedSelection`] (feature 14 §6) — a single set-based
/// UPDATE (no per-picture loop). With `dry_run` returns the affected count without mutating.
/// Re-dirties + wakes the pipeline so owned pictures re-announce their owner-deletion lifecycle.
#[tracing::instrument(skip(db, cache, waker, sel), fields(user_id = %user_id, deleted, dry_run))]
pub async fn batch_set_trashed_selection(
    db: &PgPool,
    cache: &dyn Cache,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    sel: &crate::repository::picture::ResolvedSelection,
    deleted: bool,
    dry_run: bool,
) -> Result<TrashBatchOutcome, AppError> {
    if dry_run {
        let affected = PictureRepository::count_selection(db, user_id, sel).await?;
        return Ok(TrashBatchOutcome::DryRun(
            crate::services::aggregate::DryRun {
                affected,
                ..Default::default()
            },
        ));
    }
    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;
    let affected =
        PictureRepository::batch_set_trashed_selection(&mut *tx, user_id, sel, deleted).await?;
    if deleted {
        // Reject the touched groups; the reconcile picks each representative (feature 11 §5.3).
        DedupRepository::boomerang_dedupe_in_manual_groups(&mut *tx, user_id).await?;
    } else {
        // Restore lifts the rejection (boomerang → content_dedupe), before the reconcile runs.
        DedupRepository::dedupe_boomerang_in_live_groups(&mut *tx, user_id).await?;
    }
    tx.commit().await.map_err(map_sqlx_error)?;
    // The trashed half of the tag tree is a separate set of counts and dates (feature 34 §4).
    crate::services::tag_metadata::bust_cache(cache, user_id).await;
    waker.trigger(user_id);
    Ok(TrashBatchOutcome::Applied {
        affected: affected as i64,
    })
}

#[tracing::instrument(skip(db, cache, waker), fields(user_id = %user_id, picture_id = %picture_id, deleted))]
async fn set_trashed(
    db: &PgPool,
    cache: &dyn Cache,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    picture_id: Uuid,
    deleted: bool,
) -> Result<Picture, AppError> {
    use crate::repository::pipeline::PipelineRepository;
    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;
    let ok = PictureRepository::set_deleted(&mut *tx, user_id, picture_id, deleted).await?;
    if !ok {
        return Err(AppError::NotFound);
    }
    // Content-dedup rejection lifecycle (feature 11 §5.3): delete rejects the whole group (priority
    // copy → manual representative, rest → boomerang); restore lifts it (boomerangs → content_dedupe).
    if deleted {
        DedupRepository::reject_content_group(&mut *tx, user_id, picture_id).await?;
    } else {
        DedupRepository::dedupe_boomerang_siblings(&mut *tx, user_id, picture_id).await?;
    }
    // Re-dirty so the announcement reconcile re-delivers the lifecycle change (owned) and tagging
    // re-evaluates; harmless for received rows.
    PipelineRepository::invalidate(&mut *tx, &[picture_id]).await?;
    tx.commit().await.map_err(map_sqlx_error)?;
    // The trashed half of the tag tree is a separate set of counts and dates (feature 34 §4).
    crate::services::tag_metadata::bust_cache(cache, user_id).await;
    waker.trigger(user_id);
    PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)
}

/// List the content-dedup group of a picture the caller holds (feature 11 §5.5) — the survivor plus
/// its hidden `content_dedupe`/`boomerang`/`manual` siblings. The caller must hold the picture.
#[tracing::instrument(skip(db), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn picture_copies(
    db: &PgPool,
    user_id: Uuid,
    picture_id: Uuid,
) -> Result<Vec<crate::repository::dedup::CopyRow>, AppError> {
    let picture = PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if picture.local_user_id != user_id {
        return Err(AppError::NotFound);
    }
    let group = DedupRepository::list_content_group(db, user_id, picture_id).await?;
    if !group.is_empty() {
        return Ok(group);
    }
    // No content/file hash yet (still processing) → the group is just this picture.
    Ok(vec![crate::repository::dedup::CopyRow {
        id: picture.id,
        content_hash: picture.content_hash.clone(),
        file_hash: picture.file_hash.clone(),
        deleted_reason: picture.deleted_reason,
        deleted_at: picture.deleted_at,
        updated_at: picture.updated_at,
        is_owned: picture.is_owned(),
        owner_username: picture.owner_username.clone(),
        owner_instance_domain: picture.owner_instance_domain.clone(),
        owner_deleted_at: picture.owner_deleted_at.clone(),
        copy_source_owner_username: picture.copy_source_owner_username.clone(),
        copy_source_owner_instance: picture.copy_source_owner_instance.clone(),
        copy_source_picture_id: picture.copy_source_picture_id.clone(),
        filename: picture.filename.clone(),
    }])
}

/// Make `picture_id` the live survivor of its content-dedup group (feature 11 §5.5), hiding every
/// sibling as `content_dedupe`. Because the reconciler leaves a correct single-live group untouched,
/// this user choice sticks without a pin flag. The caller must hold the picture.
#[tracing::instrument(skip(db, waker), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn set_picture_survivor(
    db: &PgPool,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    picture_id: Uuid,
) -> Result<(), AppError> {
    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;

    let target = PictureRepository::find_by_id(&mut *tx, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if target.local_user_id != user_id {
        return Err(AppError::NotFound);
    }
    let was_live = target.deleted_at.is_none();
    // The previously-live sibling is the curated source of truth (its manual tag set reflects the
    // user's adds *and removes*) — capture it before the survivor flip.
    let old_live = DedupRepository::live_id_in_group(&mut *tx, user_id, picture_id).await?;

    let ok = DedupRepository::set_survivor(&mut *tx, user_id, picture_id).await?;
    if !ok {
        return Err(AppError::NotFound);
    }

    // Tag handoff (§5.5). Switching the live copy *replaces* the new survivor's manual tags with the
    // old live's exact set, so a tag the user removed from the old live stays removed. With no prior
    // live (a rejected group promoted by the user) the group's manual tags are merged in instead; a
    // re-keep of the already-sole-live copy leaves its curated set untouched.
    match old_live {
        Some(from) => {
            let paths = TagRepository::list_manual_paths(&mut *tx, from).await?;
            TagRepository::clear_manual_tags(&mut *tx, user_id, picture_id).await?;
            if !paths.is_empty() {
                TagRepository::batch_assign(&mut *tx, user_id, &[picture_id], &paths).await?;
            }
        }
        None if !was_live => {
            let paths =
                DedupRepository::group_manual_tag_paths(&mut *tx, user_id, picture_id).await?;
            if !paths.is_empty() {
                TagRepository::batch_assign(&mut *tx, user_id, &[picture_id], &paths).await?;
            }
        }
        None => {}
    }

    tx.commit().await.map_err(map_sqlx_error)?;
    // Re-announce / re-tag the now-live picture and let the reconciler confirm consistency.
    waker.trigger(user_id);
    Ok(())
}

