//! Recipient-side registration of received pictures: creating/removing the local picture rows and
//! their `/SharedToMe/…` tags (with per-picture tokens) for an incoming share.
//! Either called from federation handler or from shares/delivery.rs module for local shares.

use crate::clients::federation::models::AnnouncedPicture;
use crate::domain::share::IncomingShare;
use crate::domain::tag::TagPath;
use crate::infra::error::{AppError, map_sqlx_error};
use crate::repository::picture::PictureRepository;
use crate::repository::pipeline::PipelineRepository;
use crate::repository::share::IncomingShareRepository;
use crate::repository::tag::TagRepository;
use sqlx::PgPool;
use uuid::Uuid;

/// Upsert received-picture rows and assign `/SharedToMe/…` tags (with their per-picture token)
/// for every picture in `pictures`, all inside a single DB transaction.
///
/// Both `create_received` (ON CONFLICT DO UPDATE) and `assign_incoming_share_tag`
/// (ON CONFLICT DO UPDATE SET picture_token) are idempotent, so replaying the same
/// announcement is safe and refreshes the token.
pub async fn register_received_pictures(
    db: &PgPool,
    recipient_id: Uuid,
    incoming_share_id: Uuid,
    shared_tag: &TagPath,
    pictures: &[AnnouncedPicture],
) -> Result<usize, AppError> {
    if pictures.is_empty() {
        return Ok(0);
    }

    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;

    for pic in pictures {
        // The announced owner snapshot (typed FullExif) is stored verbatim in remote_exif_data; the
        // merged exif_data + promoted columns are re-materialised below, preserving any existing
        // local overrides (09 §6/§8).
        let received = PictureRepository::create_received(
            &mut *tx,
            recipient_id,
            &pic.picture_id,
            &pic.owner_username,
            &pic.owner_instance_domain,
            pic.filename.as_deref(),
            pic.mime_type.as_deref(),
            pic.file_size,
            pic.width,
            pic.height,
            pic.blurhash.as_ref(),
            pic.file_hash.as_deref(),
            pic.thumbnails_generated_at,
            &pic.exif,
            pic.owner_deleted_at,
            pic.owner_purge_at,
        )
        .await?;

        // Effective EXIF = owner snapshot merged with the recipient's preserved sticky overrides.
        let overrides = received
            .local_exif_overrides
            .as_ref()
            .map(|j| j.0.clone())
            .unwrap_or_default();
        let merged = pic.exif.merged_with(&overrides);
        PictureRepository::apply_received_materialization(
            &mut *tx,
            received.id,
            &merged.camera,
            merged.captured_at,
            merged.gps_lat,
            merged.gps_lng,
            merged.gps_alt,
            merged.orientation,
        )
        .await?;

        TagRepository::assign_incoming_share_tag(
            &mut *tx,
            received.id,
            shared_tag.as_ltree(),
            incoming_share_id,
            pic.picture_token,
        )
        .await?;
    }

    // Stamp the announcement and refresh the advisory shared-tag path (reflects a sender-side
    // tag rename / re-target on the next announcement).
    IncomingShareRepository::record_announcement(
        &mut *tx,
        incoming_share_id,
        shared_tag.as_ltree(),
    )
    .await?;

    tx.commit().await.map_err(map_sqlx_error)?;

    Ok(pictures.len())
}

/// Recipient-side per-picture unannounce: remove the share's `incoming_share` tag from the named
/// pictures, delete the picture rows that no longer have any incoming-share tag, and mark the
/// survivors dirty (token refresh). Used by both the cross-instance handler and the same-backend
/// task path. Returns the number of deleted picture rows.
pub async fn unregister_announced_pictures(
    db: &PgPool,
    incoming: &IncomingShare,
    remote_ids: &[String],
) -> Result<u64, AppError> {
    if remote_ids.is_empty() {
        return Ok(0);
    }
    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;

    let local_ids =
        PictureRepository::find_ids_by_remote_ids(&mut *tx, incoming.recipient_id, remote_ids)
            .await?;
    if local_ids.is_empty() {
        tx.commit()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        return Ok(0);
    }

    TagRepository::remove_incoming_share_tags_for_pictures(&mut *tx, incoming.id, &local_ids)
        .await?;
    let deleted =
        PictureRepository::delete_orphans_among(&mut *tx, incoming.recipient_id, &local_ids)
            .await?;
    let survivors: Vec<Uuid> = local_ids
        .into_iter()
        .filter(|id| !deleted.contains(id))
        .collect();
    PipelineRepository::invalidate(&mut *tx, &survivors).await?;

    tx.commit().await.map_err(map_sqlx_error)?;
    Ok(deleted.len() as u64)
}
