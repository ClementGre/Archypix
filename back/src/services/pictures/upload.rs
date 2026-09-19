use crate::domain::picture::{Picture, UploadSession};
use crate::domain::tag::TagPath;
use crate::infra::redis::{Cache, RedisKey, cache_get_json, cache_set_json_ex};
use archypix_common::routine::RoutineHandle;
use crate::infra::s3::{self, Storage};
use crate::infra::settings::keys;
use crate::repository::picture::PictureRepository;
use crate::repository::tag::TagRepository;
use archypix_common::error::{AppError, map_sqlx_error};
use archypix_common::settings::Settings;
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;
use super::*;

#[tracing::instrument(skip(db, cache, storage, settings, files, initial_tags, waker), fields(user_id = %user_id, count = files.len()))]
pub async fn begin_upload_batch(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    settings: &Settings,
    user_id: Uuid,
    files: &[BatchUploadFile],
    initial_tags: &[String],
    upload_label: Option<&str>,
    waker: &RoutineHandle<Uuid>,
) -> Result<Vec<BatchUploadOutcome>, AppError> {
    if files.is_empty() {
        return Err(AppError::BadRequest("No filenames provided".to_string()));
    }
    if files.len() > 100 {
        return Err(AppError::BadRequest(
            "Cannot request more than 100 upload slots at once".to_string(),
        ));
    }

    // Validate any initial tags up front — reject malformed paths and the reserved `SharedToMe`
    // prefix, matching the `complete`/`PATCH /tags` contract.
    let initial_tags: Vec<String> = initial_tags
        .iter()
        .map(|t| {
            TagPath::parse(t, false)
                .map(|p| p.as_ltree().to_string())
                .map_err(AppError::BadRequest)
        })
        .collect::<Result<_, _>>()?;

    // Marker tags derived from the import label (validated once up front).
    let markers = match upload_label {
        Some(label) => Some(upload_marker_tags(label)?),
        None => None,
    };

    let mut outcomes = Vec::with_capacity(files.len());
    // Live (non-deleted) and trashed existing duplicates, tagged differently below.
    let mut existing_live: Vec<Uuid> = Vec::new();
    let mut existing_deleted: Vec<Uuid> = Vec::new();
    // The canonical target for each hash seen so far in this batch — a DB-existing picture or the
    // first new slot minted for it — so a second identical file in the *same* batch dedups onto the
    // first instead of minting a redundant slot (neither is committed yet, so the DB check alone
    // can't catch it). The bool carries whether that target was trashed.
    let mut seen_hashes: HashMap<&str, (Uuid, bool)> = HashMap::new();
    for file in files {
        if let Some(hash) = file.file_hash.as_deref().filter(|h| !h.is_empty()) {
            // Earlier file in this batch already claimed this hash.
            if let Some(&(picture_id, was_deleted)) = seen_hashes.get(hash) {
                outcomes.push(BatchUploadOutcome::Duplicate {
                    picture_id,
                    was_deleted,
                });
                continue;
            }
            // Already on an existing owned picture (including a trashed one — flagged, not restored).
            if let Some(existing) =
                PictureRepository::find_owned_by_hash(db, user_id, hash, true).await?
            {
                let was_deleted = existing.deleted_at.is_some();
                seen_hashes.insert(hash, (existing.id, was_deleted));
                if was_deleted {
                    existing_deleted.push(existing.id);
                } else {
                    existing_live.push(existing.id);
                }
                outcomes.push(BatchUploadOutcome::Duplicate {
                    picture_id: existing.id,
                    was_deleted,
                });
                continue;
            }
            // First time we see this hash — mint a slot and remember it as the canonical target.
            let (picture_id, presigned_url) = begin_upload(
                db,
                cache,
                storage,
                settings,
                user_id,
                &file.filename,
                file.size,
            )
            .await?;
            seen_hashes.insert(hash, (picture_id, false));
            outcomes.push(BatchUploadOutcome::New {
                picture_id,
                presigned_url,
            });
            continue;
        }
        // No hash supplied — can't dedup; always a fresh slot.
        let (picture_id, presigned_url) = begin_upload(
            db,
            cache,
            storage,
            settings,
            user_id,
            &file.filename,
            file.size,
        )
        .await?;
        outcomes.push(BatchUploadOutcome::New {
            picture_id,
            presigned_url,
        });
    }

    // Tags to land on each duplicate class: the user's `initial_tags` plus the import marker.
    let live_tags: Vec<String> = initial_tags
        .iter()
        .cloned()
        .chain(markers.as_ref().map(|(_, already, _)| already.clone()))
        .collect();
    let deleted_tags: Vec<String> = initial_tags
        .iter()
        .cloned()
        .chain(markers.as_ref().map(|(_, _, deleted)| deleted.clone()))
        .collect();

    let tag_live = !existing_live.is_empty() && !live_tags.is_empty();
    let tag_deleted = !existing_deleted.is_empty() && !deleted_tags.is_empty();
    if tag_live || tag_deleted {
        let mut tx = db
            .begin()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        if tag_live {
            // batch_assign re-dirties the (live) pictures the user already holds.
            TagRepository::batch_assign(&mut *tx, user_id, &existing_live, &live_tags).await?;
        }
        if tag_deleted {
            TagRepository::batch_assign_including_deleted(
                &mut *tx,
                user_id,
                &existing_deleted,
                &deleted_tags,
            )
            .await?;
        }
        tx.commit().await.map_err(map_sqlx_error)?;
        waker.trigger_debounced(user_id);
    }

    Ok(outcomes)
}

#[tracing::instrument(skip(db, cache, storage, settings), fields(user_id = %user_id))]
pub async fn begin_upload(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    settings: &Settings,
    user_id: Uuid,
    filename: &str,
    declared_size: Option<i64>,
) -> Result<(Uuid, String), AppError> {
    if filename.trim().is_empty() {
        return Err(AppError::BadRequest("Filename cannot be empty".to_string()));
    }

    // Quota gate (feature 22 §5.3): with a declared size, reject if it would push the effective
    // usage over quota, then reserve the slot; without one, apply the coarse at-quota gate.
    match declared_size {
        Some(size) => {
            if !crate::services::storage::fits(cache, db, user_id, size).await? {
                return Err(AppError::PayloadTooLarge(
                    "upload would exceed your storage quota".to_string(),
                ));
            }
        }
        None => {
            if crate::services::storage::at_or_over_quota(cache, db, user_id).await? {
                return Err(AppError::PayloadTooLarge(
                    "storage quota reached".to_string(),
                ));
            }
        }
    }

    let picture_id = Uuid::new_v4();
    let s3_key_staging = format!("staging/{}/{}", user_id, picture_id);

    let presigned_url = storage
        .presign_put(&settings.get(keys::S3_BUCKET_STAGING), &s3_key_staging)
        .await?;

    // Reserve the declared bytes for the presign→complete window (auto-releases on TTL).
    if let Some(size) = declared_size {
        crate::services::storage::reserve(cache, settings, user_id, picture_id, size).await?;
    }

    let session = UploadSession {
        user_id,
        picture_id,
        s3_key_staging,
        filename: filename.to_string(),
    };
    cache_set_json_ex(
        cache,
        RedisKey::UploadSession(picture_id),
        &session,
        settings.get(keys::S3_PRESIGN_TTL_SECS) + 60,
    )
    .await?;

    Ok((picture_id, presigned_url))
}

#[tracing::instrument(skip(db, cache, storage, settings, meta), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn complete_upload(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    settings: &Settings,
    user_id: Uuid,
    picture_id: Uuid,
    meta: UploadMetadata,
) -> Result<Picture, AppError> {
    let session: UploadSession = cache_get_json(cache, RedisKey::UploadSession(picture_id))
        .await?
        .ok_or_else(|| AppError::BadRequest("Upload session not found or expired".to_string()))?;

    if session.user_id != user_id {
        return Err(AppError::Unauthorized(
            "Upload session belongs to another user".to_string(),
        ));
    }

    // Validate any initial tags up front (before touching S3) — reject malformed paths and the
    // reserved `SharedToMe` prefix, matching the `PATCH /tags` contract (07_security_audit.md §2.5).
    let mut initial_tags: Vec<String> = match meta.initial_tags.as_ref() {
        Some(tags) => tags
            .iter()
            .map(|t| {
                TagPath::parse(t, false)
                    .map(|p| p.as_ltree().to_string())
                    .map_err(AppError::BadRequest)
            })
            .collect::<Result<_, _>>()?,
        None => Vec::new(),
    };
    // Feature 15: tag a freshly-uploaded picture with the front's import label (`Uploaded....`).
    if let Some(label) = meta.upload_label.as_deref() {
        let (base, _, _) = upload_marker_tags(label)?;
        initial_tags.push(base);
    }

    // S3: copy staging → pictures, then delete staging (S3 ops can't be in a DB tx)
    let pictures_key = s3::picture_key(user_id, picture_id);
    storage
        .copy_object(
            &settings.get(keys::S3_BUCKET_STAGING),
            &session.s3_key_staging,
            &settings.get(keys::S3_BUCKET_PICTURES),
            &pictures_key,
        )
        .await?;
    storage
        .delete_object(
            &settings.get(keys::S3_BUCKET_STAGING),
            &session.s3_key_staging,
        )
        .await?;

    // Authoritative size: read it back from S3 rather than trusting the client value
    let file_size = match storage
        .object_size(&settings.get(keys::S3_BUCKET_PICTURES), &pictures_key)
        .await
    {
        Ok(size) => Some(size),
        Err(e) => {
            tracing::warn!(picture_id = %picture_id, error = ?e, "complete_upload: S3 HEAD failed; falling back to client-reported size");
            meta.file_size
        }
    };

    // Quota hard check (feature 22 §5.3): the authoritative size is known. Release this upload's
    // reservation first (so it is not double-counted), then verify the committed usage plus this
    // object fits. On overflow, delete the promoted object and abort — no orphan bytes, `413`.
    crate::services::storage::release(cache, user_id, picture_id).await;
    if let Some(size) = file_size {
        if !crate::services::storage::fits(cache, db, user_id, size).await? {
            let _ = storage
                .delete_object(&settings.get(keys::S3_BUCKET_PICTURES), &pictures_key)
                .await;
            return Err(AppError::PayloadTooLarge(
                "upload would exceed your storage quota".to_string(),
            ));
        }
    }

    // Single DB transaction: create picture row, thumbnail job.
    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;

    let picture = PictureRepository::create(
        &mut *tx,
        picture_id,
        user_id,
        Some(session.filename.as_str()),
        meta.mime_type.as_deref(),
        file_size,
        meta.width,
        meta.height,
        meta.exif_data.clone(),
        meta.captured_at,
        meta.original_file_created_at,
        crate::services::jobs::ingest_exif_status(meta.mime_type.as_deref()),
    )
    .await?;

    // Persist any client-computed SHA-256 as the provisional hash
    if let Some(hash) = meta.file_hash.as_deref() {
        PictureRepository::set_file_hash(&mut *tx, picture_id, hash, file_size).await?;
    }

    // Enqueue initial thumbnail generation + EXIF extraction inside the same transaction
    crate::services::jobs::enqueue_thumbnail_job(
        &mut *tx,
        user_id,
        picture_id,
        true,
        meta.file_hash.as_deref(),
    )
    .await?;

    // Assign any caller-supplied initial manual tags
    if !initial_tags.is_empty() {
        TagRepository::batch_assign(&mut *tx, user_id, &[picture_id], &initial_tags).await?;
    }

    tx.commit().await.map_err(map_sqlx_error)?;

    // The trigger just committed the new bytes → drop the cached committed mirror so the next
    // quota check recomputes (feature 22 §5.2).
    crate::services::storage::invalidate_committed(cache, user_id).await;

    // Cache cleanup is after commit. A failure here is non-fatal
    if let Err(e) = cache.del(RedisKey::UploadSession(picture_id)).await {
        tracing::warn!(picture_id = %picture_id, error = ?e, "failed to delete upload session from cache");
    }

    Ok(picture)
}

