//! Share lifecycle: create / accept / revoke / reject outgoing & incoming shares, and the shared
//! `cleanup_incoming_share` teardown. Picture announcement itself is the pipeline's job — these
//! functions only manage share state and hand work to the pipeline (via the
//! `pending_first_announcement` status) and the task queue.

use crate::clients::federation::FederationClient;
use crate::clients::federation::models::ShareAnnouncementRequest;
use crate::domain::share::{IncomingShare, OutgoingShare, ShareStatus};
use crate::domain::tag::TagPath;
use crate::infra::redis::Cache;
use crate::infra::routine::RoutineHandle;
use crate::infra::routine::unannounce::UnannounceInput;
use crate::infra::settings::keys;
use crate::repository::picture::PictureRepository;
use crate::repository::pipeline::PipelineRepository;
use crate::repository::share::{IncomingShareRepository, OutgoingShareRepository};
use crate::repository::share_announcement::ShareAnnouncementRepository;
use crate::repository::tag::TagRepository;
use crate::repository::user::UserRepository;
use crate::services::shares::shareback::auto_accept_shareback_local;
use crate::services::users::find_local_user_id;
use archypix_common::error::{AppError, map_sqlx_error};
use archypix_common::settings::Settings;
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::hash::RandomState;
use std::sync::Arc;
use uuid::Uuid;

/// Remove tags, delete unreachable received pictures, set the share to `final_status` (which makes
/// any mapping referencing it derive as broken), cascade downstream unannounce / transitive
/// revocation, and wake the pipeline.
/// Used by both revocation (→ Revoked) and rejection (→ Tombstoned).
///
/// See doc/features/01_better_sharing_support.md §8 for the full sequence. Returns the number of
/// received pictures deleted.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, federation, settings, task_queue, pipeline_waker, share), fields(share_id = %share.id))]
pub async fn cleanup_incoming_share(
    db: &PgPool,
    cache: &dyn Cache,
    federation: &FederationClient,
    settings: &Settings,
    task_queue: &RoutineHandle<UnannounceInput>,
    pipeline_waker: &RoutineHandle<Uuid>,
    share: &IncomingShare,
    final_status: ShareStatus,
) -> Result<u64, AppError> {
    // Capture the SharedToMe tag paths before the tags are removed (needed for transitive
    // revocation scoping).
    let shared_paths = TagRepository::incoming_share_tag_paths(db, share.id).await?;

    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;

    let affected = TagRepository::remove_incoming_share_tags(&mut *tx, share.id).await?;
    let survivors = PictureRepository::find_with_any_incoming_share_tag(
        &mut *tx,
        share.recipient_id,
        &affected,
    )
    .await?;
    let survivors_set: HashSet<Uuid, RandomState> = HashSet::from_iter(survivors.iter().cloned());
    let deleted_ids: Vec<Uuid> = affected
        .iter()
        .filter(|id| !survivors_set.contains(id))
        .cloned()
        .collect();

    // Downstream recipients of the to-be-deleted pictures (pictures still exist here).
    let downstream =
        ShareAnnouncementRepository::find_downstream_for_pictures(&mut *tx, &deleted_ids).await?;

    let deleted = PictureRepository::delete_received_without_share_tags(
        &mut *tx,
        share.recipient_id,
        &share.sender_username,
        &share.sender_instance,
    )
    .await?;
    ShareAnnouncementRepository::delete_for_pictures(&mut *tx, &deleted_ids).await?;
    PipelineRepository::invalidate(&mut *tx, &survivors).await?;
    // Mapping brokenness is derived from the share status (feature 20 §10.1) — setting the share's
    // status below is all that's needed; no tagging-config write here.
    IncomingShareRepository::set_status(&mut *tx, share.id, final_status.clone()).await?;

    tx.commit().await.map_err(map_sqlx_error)?;

    // ── Side effects (after commit) ───────────────────────────────────────────
    // The relayer (this share's recipient) is the sender of any downstream unannounce.
    let relayer_username = UserRepository::find_by_id(db, share.recipient_id)
        .await?
        .map(|u| u.username)
        .unwrap_or_default();

    // Unannounce deleted pictures to downstream recipients, grouped per outgoing share.
    let mut by_share: HashMap<Uuid, (String, String, Vec<String>)> = HashMap::new();
    for d in downstream {
        let entry = by_share.entry(d.outgoing_share_id).or_insert_with(|| {
            (
                d.recipient_username.clone(),
                d.recipient_instance.clone(),
                vec![],
            )
        });
        entry.2.push(d.announce_id);
    }
    for (os_id, (recipient_username, recipient_instance, picture_ids)) in by_share {
        // Same-backend ⇔ the recipient user resolves locally — not merely the same global domain
        // (multiple backends can share a global domain). See doc/features/02 §5.
        let is_same_backend = find_local_user_id(
            cache,
            db,
            settings,
            &recipient_username,
            &recipient_instance,
        )
        .await?
        .is_some();
        task_queue.trigger(UnannounceInput {
            outgoing_share_id: os_id,
            sender_username: relayer_username.clone(),
            recipient_username,
            recipient_instance,
            picture_ids,
            is_same_backend,
        });
    }

    // Transitive revocation: only on a real revocation (not a rejection/tombstone), and only
    // for directly re-shared `SharedToMe.*` tags.
    if final_status == ShareStatus::Revoked {
        for path in &shared_paths {
            let downstream_shares =
                OutgoingShareRepository::find_by_tag_prefix(db, share.recipient_id, path).await?;
            for sh in downstream_shares {
                Box::pin(revoke_outgoing_share(
                    db,
                    cache,
                    federation,
                    settings,
                    task_queue,
                    pipeline_waker,
                    share.recipient_id,
                    "", // owner_username only used for cross-instance federation messages
                    sh.id,
                ))
                .await?;
            }
        }
    }

    pipeline_waker.trigger(share.recipient_id);
    Ok(deleted)
}

#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, federation, settings, task_queue, pipeline_waker), fields(share_id = %share_id, user_id = %rejector_id))]
pub async fn reject_incoming_share(
    db: &PgPool,
    cache: &dyn Cache,
    federation: &FederationClient,
    settings: &Settings,
    task_queue: &RoutineHandle<UnannounceInput>,
    pipeline_waker: &RoutineHandle<Uuid>,
    rejector_id: Uuid,
    rejector_username: &str,
    share_id: Uuid,
) -> Result<(), AppError> {
    let incoming = IncomingShareRepository::get_by_id(db, share_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if incoming.recipient_id != rejector_id {
        return Err(AppError::NotFound);
    }

    match incoming.status {
        ShareStatus::Tombstoned => return Ok(()),
        ShareStatus::Revoked => return Err(AppError::NotFound),
        ShareStatus::Pending | ShareStatus::PendingFirstAnnouncement => {
            IncomingShareRepository::set_status(db, share_id, ShareStatus::Tombstoned).await?;
        }
        // `Errored` is outgoing-only, but the shared enum requires a branch; treat like Active.
        ShareStatus::Active | ShareStatus::Errored => {
            cleanup_incoming_share(
                db,
                cache,
                federation,
                settings,
                task_queue,
                pipeline_waker,
                &incoming,
                ShareStatus::Tombstoned,
            )
            .await?;
        }
    }

    // Notify the sender that their share was rejected.
    if find_local_user_id(
        cache,
        db,
        settings,
        &incoming.sender_username,
        &incoming.sender_instance,
    )
    .await?
    .is_some()
    {
        // Same-backend: directly tombstone the sender's OutgoingShare and drop its tracking rows
        // (invalidating its presign tokens), mirroring revocation.
        OutgoingShareRepository::set_status(
            db,
            incoming.outgoing_share_id,
            ShareStatus::Tombstoned,
        )
        .await?;
        ShareAnnouncementRepository::delete_all_for_share(db, incoming.outgoing_share_id).await?;
    } else {
        // Cross-instance: send rejection to the sender's backend.
        federation
            .send_share_reject(
                rejector_username,
                &incoming.sender_username,
                &incoming.sender_instance,
                incoming.outgoing_share_id,
            )
            .await?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, federation, settings, pipeline_waker), fields(user_id = %owner_id))]
pub async fn create_outgoing_share(
    db: &PgPool,
    cache: &dyn Cache,
    federation: &FederationClient,
    settings: &Settings,
    pipeline_waker: &RoutineHandle<Uuid>,
    owner_id: Uuid,
    sender_username: &str,
    tag_path: &str,
    name: &str,
    message: Option<&str>,
    recipient_username: &str,
    recipient_instance: &str,
    allow_share_back: bool,
    allow_exif_edit: bool,
    future: bool,
    shareback_of: Option<Uuid>,
) -> Result<OutgoingShare, AppError> {
    // Validate the user-supplied recipient instance before it drives any WebFinger / federation
    // HTTP call — blocks the blind-SSRF / request-amplification vector (07_security_audit.md §2.4).
    crate::domain::validation::validate_federation_domain(recipient_instance)
        .map_err(AppError::BadRequest)?;
    crate::domain::validation::validate_share_name(name).map_err(AppError::BadRequest)?;
    crate::domain::validation::validate_share_message(message).map_err(AppError::BadRequest)?;

    // Cap the number of outstanding `pending` outgoing shares per user to curb share spam
    let pending_outgoing = OutgoingShareRepository::list_by_owner(db, owner_id)
        .await?
        .into_iter()
        .filter(|s| s.status == ShareStatus::Pending)
        .count();
    if pending_outgoing >= settings.get(keys::MAX_PENDING_OUTGOING_SHARES) {
        return Err(AppError::TooManyRequests(format!(
            "You have too many pending shares ({} max). Wait for some to be accepted or revoke them.",
            settings.get(keys::MAX_PENDING_OUTGOING_SHARES)
        )));
    }

    let recipient_local_id =
        find_local_user_id(cache, db, settings, recipient_username, recipient_instance).await?;

    let mut tx = db
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;

    let share = OutgoingShareRepository::create(
        &mut *tx,
        owner_id,
        tag_path,
        name,
        message,
        recipient_username,
        recipient_instance,
        allow_share_back,
        allow_exif_edit,
        future,
        shareback_of,
    )
    .await?;

    let mut same_backend_incoming: Option<(Uuid, IncomingShare)> = None;
    let mut cross_instance_auto_accepted = false;
    if let Some(recipient_id) = recipient_local_id {
        // Same-backend: create IncomingShare in the same transaction. Stamp the advisory shared-tag
        // path so the recipient sees the target tag even before the first announcement.
        let shared_tag = TagPath::shared_to_me(
            sender_username,
            &settings.get(keys::GLOBAL_DOMAIN),
            &TagPath::from_ltree(tag_path),
        );
        let incoming = IncomingShareRepository::create(
            &mut *tx,
            recipient_id,
            sender_username,
            &settings.get(keys::GLOBAL_DOMAIN),
            name,
            message,
            share.id,
            allow_share_back,
            allow_exif_edit,
            future,
            Some(shared_tag.as_ltree()),
            shareback_of,
        )
        .await?;
        same_backend_incoming = Some((recipient_id, incoming));
    } else {
        // Cross-instance share: announce via federation protocol inside the transaction, so a
        // delivery failure rolls back the OutgoingShare insert.
        let token = federation
            .get_or_wait_federation_token(sender_username, recipient_username, recipient_instance)
            .await?;
        let auto_accepted = federation
            .announce_share(
                recipient_username,
                recipient_instance,
                &token,
                &ShareAnnouncementRequest {
                    sender_username: sender_username.to_string(),
                    sender_instance: settings.get(keys::GLOBAL_DOMAIN).clone(),
                    recipient_username: recipient_username.to_string(),
                    recipient_instance: recipient_instance.to_string(),
                    outgoing_share_id: share.id,
                    tag_path: tag_path.to_string(),
                    name: name.to_string(),
                    message: message.map(str::to_string),
                    allow_share_back,
                    allow_exif_edit,
                    future,
                    shareback_of,
                },
            )
            .await?;

        // ShareBack auto-accepted by the recipient (no callback into this still-open transaction;
        // it returned `auto_accepted`). Hand our OutgoingShare to the pipeline — set
        // `pending_first_announcement` so it announces our pictures and flips to Active.
        if auto_accepted {
            OutgoingShareRepository::set_status(
                &mut *tx,
                share.id,
                ShareStatus::PendingFirstAnnouncement,
            )
            .await?;
        }
        cross_instance_auto_accepted = auto_accepted;
    }

    tx.commit().await.map_err(map_sqlx_error)?;

    if cross_instance_auto_accepted {
        // Wake the pipeline to announce the just-created ShareBack's pictures (owner is the sender).
        pipeline_waker.trigger(owner_id);
    }

    // Same-backend ShareBack auto-accept (no federation involved). Runs *after* commit and is
    // non-fatal: on failure the OutgoingShare is still created and the recipient can
    // accept the ShareBack manually. The recipient's IncomingShare is activated + mapped here;
    // the sender's pictures are announced by the pipeline once the OutgoingShare is moved to `pending_first_announcement`.
    if let (Some((recipient_id, incoming)), Some(original_os_id)) =
        (same_backend_incoming, shareback_of)
    {
        if let Some(original) = OutgoingShareRepository::get_by_id(db, original_os_id).await? {
            let verified = original.owner_id == recipient_id
                && original.recipient_username == sender_username
                && original.recipient_instance == settings.get(keys::GLOBAL_DOMAIN)
                && original.allow_share_back;
            if verified {
                match auto_accept_shareback_local(
                    db,
                    pipeline_waker,
                    recipient_id,
                    &incoming,
                    &original,
                )
                .await
                {
                    Ok(()) => {
                        // Announce the initiator's pictures to the recipient via the pipeline.
                        let _ = OutgoingShareRepository::set_status(
                            db,
                            share.id,
                            ShareStatus::PendingFirstAnnouncement,
                        )
                        .await;
                        pipeline_waker.trigger(owner_id);
                    }
                    Err(e) => tracing::error!(
                        share_id = %share.id,
                        error = ?e,
                        "shares: same-backend ShareBack auto-accept failed (share created; recipient may accept manually)"
                    ),
                }
            }
        }
    }

    Ok(share)
}

/// Accept an incoming share on behalf of `acceptor_username`.
///
/// Both paths only flip share status and wake the pipeline; the pictures are announced
/// asynchronously by the pipeline (via the `pending_first_announcement` status — the single
/// announce path):
/// - Same-backend: move the sender's OutgoingShare to `pending_first_announcement`.
/// - Cross-instance: notify the sender, who moves *its* OutgoingShare and announces back.
#[tracing::instrument(skip(db, cache, federation, settings, pipeline_waker), fields(share_id = %share_id, user_id = %acceptor_id))]
pub async fn accept_incoming_share(
    db: &PgPool,
    cache: &dyn Cache,
    federation: &FederationClient,
    settings: &Settings,
    pipeline_waker: &RoutineHandle<Uuid>,
    acceptor_id: Uuid,
    acceptor_username: &str,
    share_id: Uuid,
) -> Result<(), AppError> {
    let incoming = IncomingShareRepository::get_by_id(db, share_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if incoming.recipient_id != acceptor_id {
        return Err(AppError::NotFound);
    }

    match incoming.status {
        ShareStatus::Pending => {} // normal path
        // already accepted / outgoing-only states — idempotent no-op on the incoming side
        ShareStatus::Active | ShareStatus::PendingFirstAnnouncement | ShareStatus::Errored => {
            return Ok(());
        }
        ShareStatus::Revoked | ShareStatus::Tombstoned => return Err(AppError::NotFound),
    }

    // Transition to Active immediately — this is the acceptor's consent.
    IncomingShareRepository::set_status(db, incoming.id, ShareStatus::Active).await?;

    let sender_local_id = find_local_user_id(
        cache,
        db,
        settings,
        &incoming.sender_username,
        &incoming.sender_instance,
    )
    .await?;
    if let Some(sender_id) = sender_local_id {
        // ── Same-backend path ─────────────────────────────────────────────────
        // Hand the sender's OutgoingShare to the pipeline: it announces the current coverage and
        // flips the share to Active. No pictures are registered synchronously here.
        OutgoingShareRepository::set_status(
            db,
            incoming.outgoing_share_id,
            ShareStatus::PendingFirstAnnouncement,
        )
        .await?;
        pipeline_waker.trigger(sender_id);
        Ok(())
    } else {
        // ── Cross-instance path ───────────────────────────────────────────────
        // The IncomingShare is set Active *before* notifying the sender, because the sender then
        // moves its OutgoingShare to `pending_first_announcement` and its pipeline announces the
        // pictures back to us — which requires our `IncomingShare = Active` to be committed. If
        // the accept notification cannot be delivered, revert to Pending so the share isn't left
        // stuck Active with no pictures (keeping the requester unchanged on failure — the Rule).
        if let Err(e) = federation
            .send_share_accept(
                acceptor_username,
                &incoming.sender_username,
                &incoming.sender_instance,
                incoming.outgoing_share_id,
            )
            .await
        {
            let _ =
                IncomingShareRepository::set_status(db, incoming.id, ShareStatus::Pending).await;
            return Err(e);
        }
        Ok(())
    }
}

/// Revoke an outgoing share owned by `owner_id`.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, federation, settings, task_queue, pipeline_waker), fields(share_id = %share_id, user_id = %owner_id))]
pub async fn revoke_outgoing_share(
    db: &PgPool,
    cache: &dyn Cache,
    federation: &FederationClient,
    settings: &Settings,
    task_queue: &RoutineHandle<UnannounceInput>,
    pipeline_waker: &RoutineHandle<Uuid>,
    owner_id: Uuid,
    owner_username: &str,
    share_id: Uuid,
) -> Result<(), AppError> {
    let share = OutgoingShareRepository::get_by_id(db, share_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if share.owner_id != owner_id {
        return Err(AppError::NotFound);
    }
    if share.status == ShareStatus::Revoked {
        return Ok(()); // idempotent
    }

    // Mark the outgoing share as revoked first so no new picture announcements go out, then drop
    // all of its per-picture tokens (immediately invalidating presign for this share).
    OutgoingShareRepository::set_status(db, share_id, ShareStatus::Revoked).await?;
    ShareAnnouncementRepository::delete_all_for_share(db, share_id).await?;

    if find_local_user_id(
        cache,
        db,
        settings,
        &share.recipient_username,
        &share.recipient_instance,
    )
    .await?
    .is_some()
    {
        // ── Same-backend path ─────────────────────────────────────────────────
        // The IncomingShare may not exist yet (e.g. share created and immediately revoked).
        if let Some(incoming) = IncomingShareRepository::find_by_outgoing_share(
            db,
            share_id,
            &settings.get(keys::GLOBAL_DOMAIN),
        )
        .await?
        {
            if incoming.status != ShareStatus::Revoked && incoming.status != ShareStatus::Tombstoned
            {
                cleanup_incoming_share(
                    db,
                    cache,
                    federation,
                    settings,
                    task_queue,
                    pipeline_waker,
                    &incoming,
                    ShareStatus::Revoked,
                )
                .await?;
            }
        }
    } else {
        // ── Cross-instance path ───────────────────────────────────────────────
        federation
            .send_revocation(
                owner_username,
                &share.recipient_username,
                &share.recipient_instance,
                share.id,
            )
            .await?;
    }

    Ok(())
}
