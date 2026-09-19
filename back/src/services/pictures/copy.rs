use crate::clients::federation::FederationClient;
use crate::domain::picture::Picture;
use crate::infra::redis::Cache;
use archypix_common::routine::RoutineHandle;
use crate::infra::s3::{self, Storage};
use crate::infra::settings::keys;
use crate::repository::picture::PictureRepository;
use crate::repository::tag::TagRepository;
use crate::services::users::find_local_user_id;
use archypix_common::error::{AppError, map_sqlx_error};
use archypix_common::settings::Settings;
use sqlx::PgPool;
use uuid::Uuid;

/// Copy a received (or owned) picture into the caller's library as a new, independent owned picture
/// (feature 11 §3): a fresh id, `copy_source_*` provenance root, server-side byte copy (S3 copy for a
/// local source, presign+fetch for a cross-instance owner), seeded effective EXIF, and a
/// `gen_thumbnail` enqueue. See doc/features/11 §3.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, storage, settings, federation, waker), fields(user_id = %user_id, source_id = %source_picture_id))]
pub async fn copy_picture(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    settings: &Settings,
    federation: &FederationClient,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    caller_username: &str,
    source_picture_id: Uuid,
) -> Result<Picture, AppError> {
    let source = PictureRepository::find_by_id(db, source_picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if source.local_user_id != user_id {
        return Err(AppError::NotFound);
    }
    let global_domain = settings.get(keys::GLOBAL_DOMAIN);
    copy_source_into_library(
        db,
        cache,
        storage,
        settings,
        federation,
        waker,
        user_id,
        &source,
        caller_username,
        &global_domain,
    )
    .await
}

/// Copy a picture the caller **holds via a public-share coverage grant** (feature 27 §8, "save a
/// copy") into their library. The source is the public-share owner's local row (same-backend only —
/// the coverage check ran against the owner on this backend). Provenance + creator carry the origin,
/// not the copier. Cross-instance save-a-copy is a follow-up (§10, the deepest escalation).
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, storage, settings, federation, waker, source), fields(dest_user_id = %dest_user_id, source_id = %source.id))]
pub async fn copy_covered_picture(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    settings: &Settings,
    federation: &FederationClient,
    waker: &RoutineHandle<Uuid>,
    dest_user_id: Uuid,
    source: &Picture,
    source_owner_username: &str,
    source_owner_domain: &str,
) -> Result<Picture, AppError> {
    copy_source_into_library(
        db,
        cache,
        storage,
        settings,
        federation,
        waker,
        dest_user_id,
        source,
        source_owner_username,
        source_owner_domain,
    )
    .await
}

/// Shared physical-copy core (feature 11 §3 + feature 27 §8): quota-gate, copy the source bytes into
/// `dest_user_id`'s library (S3 copy for a local source, presign+fetch for a cross-instance owner),
/// root-resolve `copy_source_*` provenance, carry the source's creator (attribution travels), and
/// enqueue `gen_thumbnail`. `source_owner_username`/`source_owner_domain` identify who owns `source`
/// locally (the caller for a self-copy, the public-share owner for a coverage copy) — used only for an
/// **owned** source's provenance/creator materialisation.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, storage, settings, federation, waker, source), fields(dest_user_id = %dest_user_id, source_id = %source.id))]
async fn copy_source_into_library(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    settings: &Settings,
    federation: &FederationClient,
    waker: &RoutineHandle<Uuid>,
    dest_user_id: Uuid,
    source: &Picture,
    source_owner_username: &str,
    source_owner_domain: &str,
) -> Result<Picture, AppError> {
    let user_id = dest_user_id;
    // Whether the source's local owner is the destination user (a self-copy) — decides whether an
    // owned source's owner-default creator stays NULL or is materialised to the real owner.
    let same_owner = source.local_user_id == dest_user_id;

    // Quota gate (feature 22 §6): a copy becomes a new owned picture — bill it upfront. `507` before
    // the S3 copy so no bytes are written when over quota.
    if !crate::services::storage::fits(cache, db, user_id, source.file_size.unwrap_or(0)).await? {
        return Err(AppError::InsufficientStorage(
            "copying this picture would exceed your storage quota".to_string(),
        ));
    }

    let new_id = Uuid::new_v4();
    let new_key = s3::picture_key(user_id, new_id);

    // ── Copy the bytes into the destination's pictures object ─────────────────
    if source.is_owned() {
        storage
            .copy_object(
                &settings.get(keys::S3_BUCKET_PICTURES),
                &s3::picture_key(source.local_user_id, source.id),
                &settings.get(keys::S3_BUCKET_PICTURES),
                &new_key,
            )
            .await?;
    } else {
        let owner_username = source.owner_username.as_deref().unwrap_or_default();
        let owner_instance = source.owner_instance_domain.as_deref().unwrap_or_default();
        let remote_id: Uuid = source
            .remote_picture_id
            .as_deref()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                AppError::InternalServerError("received picture missing remote_picture_id".into())
            })?;
        if let Some(owner_id) =
            find_local_user_id(cache, db, settings, owner_username, owner_instance).await?
        {
            storage
                .copy_object(
                    &settings.get(keys::S3_BUCKET_PICTURES),
                    &s3::picture_key(owner_id, remote_id),
                    &settings.get(keys::S3_BUCKET_PICTURES),
                    &new_key,
                )
                .await?;
        } else {
            // Cross-instance: the owner must be reachable. Presign the original via the picture's
            // own token, download the bytes, and upload them under the caller's key.
            let token = TagRepository::find_active_picture_token(db, source.id)
                .await?
                .ok_or_else(|| {
                    AppError::Unauthorized(format!(
                        "no active presign token for picture {}",
                        source.id
                    ))
                })?;
            let mut urls = federation
                .presign_remote_pictures(owner_username, owner_instance, &[(token, "original")])
                .await?;
            let url = urls.remove(&token).map(|r| r.url).ok_or_else(|| {
                AppError::InternalServerError("owner backend returned no presigned URL".into())
            })?;
            let resp = reqwest::get(&url)
                .await
                .map_err(|e| AppError::InternalServerError(format!("copy fetch failed: {e}")))?
                .error_for_status()
                .map_err(|e| AppError::InternalServerError(format!("copy fetch status: {e}")))?;
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| AppError::InternalServerError(format!("copy read failed: {e}")))?
                .to_vec();
            storage
                .put_object(
                    &settings.get(keys::S3_BUCKET_PICTURES),
                    &new_key,
                    bytes,
                    source.mime_type.as_deref(),
                )
                .await?;
        }
    }

    // ── Provenance root (§3 / §7.1): point at the genuine original, not the intermediary ─────
    let (cs_user, cs_instance, cs_pic) = if source.copy_source_picture_id.is_some() {
        (
            source.copy_source_owner_username.clone(),
            source.copy_source_owner_instance.clone(),
            source.copy_source_picture_id.clone(),
        )
    } else if source.is_owned() {
        (
            Some(source_owner_username.to_string()),
            Some(source_owner_domain.to_string()),
            Some(source.id.to_string()),
        )
    } else {
        (
            source.owner_username.clone(),
            source.owner_instance_domain.clone(),
            source.remote_picture_id.clone(),
        )
    };

    // ── Creator carries with the content (§6): the source's propagated value, never the copier ──
    // Owned source with an unset creator: a *self-copy* stays owner-default (NULL ⇒ the copier, who
    // now owns it); a *coverage copy* materialises the source owner's identity (attribution travels).
    // A received source's owner default is always materialised.
    let copy_creator: Option<String> = match source.creator.as_deref() {
        Some(c) if !c.is_empty() => Some(c.to_string()),
        _ if source.is_owned() && same_owner => None,
        _ if source.is_owned() => Some(Picture::format_identity(
            source_owner_username,
            source_owner_domain,
        )),
        _ => match (
            source.owner_username.as_deref(),
            source.owner_instance_domain.as_deref(),
        ) {
            (Some(u), Some(d)) if !u.is_empty() => Some(Picture::format_identity(u, d)),
            _ => None,
        },
    };

    // ── New owned row, seeded from the source's effective EXIF ────────────────
    let eff = source.full_exif();
    let camera_json = serde_json::to_value(&eff.camera).unwrap_or_else(|_| serde_json::json!({}));

    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;
    let copy = PictureRepository::create_copy(
        &mut *tx,
        new_id,
        user_id,
        source.filename.as_deref(),
        source.mime_type.as_deref(),
        source.file_size,
        source.width,
        source.height,
        camera_json,
        eff.captured_at,
        eff.gps_lat,
        eff.gps_lng,
        eff.gps_alt,
        eff.orientation,
        cs_user.as_deref(),
        cs_instance.as_deref(),
        cs_pic.as_deref(),
        copy_creator.as_deref(),
        // Identical bytes: the source's verdict and its file snapshot carry over unread (33 §4.1).
        crate::services::jobs::copy_exif_status(source.exif_sync_status),
        source
            .file_exif
            .as_ref()
            .and_then(|f| serde_json::to_value(&f.0).ok()),
    )
    .await?;
    // `is_initial = false`: keep the seeded effective EXIF (don't re-extract the owner's embedded
    // EXIF), but still compute file_size/hash, content_hash, dimensions and thumbnails.
    crate::services::jobs::enqueue_thumbnail_job(&mut *tx, user_id, new_id, false, None).await?;
    tx.commit().await.map_err(map_sqlx_error)?;

    // New owned bytes committed → drop the cached committed mirror (feature 22 §5.2).
    crate::services::storage::invalidate_committed(cache, user_id).await;

    // Wake the pipeline so the new owned picture is tagged; the dedup reconcile runs again once
    // `gen_thumbnail` lands its `content_hash` (that completion wakes the pipeline too).
    waker.trigger_debounced(user_id);

    Ok(copy)
}

