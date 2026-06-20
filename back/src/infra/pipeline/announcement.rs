//! Pipeline announcement step — the single picture-announcement path.
//!
//! [`reconcile_share`] diffs one share's coverage against the `share_announcements` tracking table
//! and delivers **inline** with deliver-then-record ordering: the federation call (cross-instance)
//! or local registration (same-backend) happens first, and tracking rows / status flips are written
//! only on success. A failed delivery demotes an `active` share to `errored` and sets a retry
//! backoff; a fully-delivered `pending_first_announcement`/`errored` share flips to `active`.
//!
//! Two entry points select the coverage scope:
//! - [`reconcile_pending_and_errored`] — full coverage for `pending_first_announcement`/`errored`
//!   shares (the initial announce and the failure-recovery pass).
//! - [`reconcile_active_batch`] — the dirty-picture delta for `active` + `future = true` shares.
//!
//! See `doc/features/02_pipeline_announcement_robustness.md` §3/§4.

use super::PipelineRun;
use crate::clients::federation::models::{
    AnnouncedPicture, PicturesAnnouncementRequest, PicturesUnannouncementRequest,
};
use crate::domain::picture::Picture;
use crate::domain::share::{OutgoingShare, ShareStatus};
use crate::domain::tag::TagPath;
use crate::infra::error::{AppError, map_sqlx_error};
use crate::repository::picture::PictureRepository;
use crate::repository::share::{IncomingShareRepository, OutgoingShareRepository};
use crate::repository::share_announcement::ShareAnnouncementRepository;
use crate::repository::tag::TagRepository;
use crate::repository::user::UserRepository;
use crate::services::shares::registration::{
    register_received_pictures, unregister_announced_pictures,
};
use crate::services::users::find_local_user_id;
use chrono::{Duration as ChronoDuration, Utc};
use std::collections::{HashMap, HashSet};
use tracing::debug;
use uuid::Uuid;

/// Which pictures a reconcile pass considers.
enum CoverageScope<'a> {
    /// All pictures under the share's tag (PFA / Errored full reconcile).
    Full,
    /// Only these dirty pictures (the active incremental fast path).
    Dirty(&'a [Uuid]),
}

/// Reconcile every share of `user_id` needing a full pass: `pending_first_announcement` (initial
/// announce) or `errored` (failure recovery), with the backoff window already elapsed.
pub async fn reconcile_pending_and_errored(
    run: &PipelineRun<'_>,
    user_id: Uuid,
) -> Result<(), AppError> {
    let now = Utc::now().naive_utc();
    let shares = OutgoingShareRepository::list_announceable_by_owner(run.db, user_id, now).await?;
    if shares.is_empty() {
        return Ok(());
    }
    let sender = sender_username(run, user_id).await?;
    for share in shares {
        debug!(sender = sender, sender_id = ?user_id, share_id = ?share.id, tag = share.tag_path, recipient = share.recipient_username, recipient_global_domain = share.recipient_instance, "pipeline: reconciling share");
        reconcile_share(run, &share, CoverageScope::Full, &sender).await?;
    }
    Ok(())
}

/// Reconcile the user's `active` + `future = true` shares over a batch of reconciled dirty pictures.
pub async fn reconcile_active_batch(
    run: &PipelineRun<'_>,
    user_id: Uuid,
    dirty_ids: &[Uuid],
) -> Result<(), AppError> {
    if dirty_ids.is_empty() {
        return Ok(());
    }
    let shares = OutgoingShareRepository::list_active_future_by_owner(run.db, user_id).await?;
    if shares.is_empty() {
        return Ok(());
    }
    let sender = sender_username(run, user_id).await?;
    for share in shares {
        reconcile_share(run, &share, CoverageScope::Dirty(dirty_ids), &sender).await?;
    }
    Ok(())
}

async fn sender_username(run: &PipelineRun<'_>, user_id: Uuid) -> Result<String, AppError> {
    Ok(UserRepository::find_by_id(run.db, user_id)
        .await?
        .map(|u| u.username)
        .unwrap_or_default())
}

/// The id the recipient stores as `remote_picture_id` (the original owner's picture id).
fn announce_id(p: &Picture) -> String {
    p.remote_picture_id
        .clone()
        .unwrap_or_else(|| p.id.to_string())
}

fn needs_activation(share: &OutgoingShare) -> bool {
    matches!(
        share.status,
        ShareStatus::PendingFirstAnnouncement | ShareStatus::Errored
    )
}

/// Diff one share against the tracking table, deliver inline, and record on success.
async fn reconcile_share(
    run: &PipelineRun<'_>,
    share: &OutgoingShare,
    scope: CoverageScope<'_>,
    sender_username: &str,
) -> Result<(), AppError> {
    let db = run.db;
    let scope_ids: Option<&[Uuid]> = match &scope {
        CoverageScope::Full => None,
        CoverageScope::Dirty(ids) => Some(ids),
    };

    // ── Read current coverage and existing tracking ───────────────────────────
    let covered: HashSet<Uuid> = ShareAnnouncementRepository::coverage_for_share(
        db,
        share.owner_id,
        &share.tag_path,
        &share.recipient_username,
        &share.recipient_instance,
        scope_ids,
    )
    .await?
    .into_iter()
    .collect();

    let tracking = ShareAnnouncementRepository::tracking_for_share(db, share.id, scope_ids).await?;
    // picture_id → (token, announced_updated_at) — the last value at which it was (re-)announced.
    let tracking_map: HashMap<Uuid, (Uuid, Option<chrono::NaiveDateTime>)> = tracking
        .iter()
        .map(|(pic, tok, at)| (*pic, (*tok, *at)))
        .collect();

    // Picture metadata + upstream tokens for everything involved.
    let mut involved: HashSet<Uuid> = covered.iter().copied().collect();
    for (pic, _, _) in &tracking {
        involved.insert(*pic);
    }
    let involved_ids: Vec<Uuid> = involved.into_iter().collect();
    let meta_by_id: HashMap<Uuid, Picture> = PictureRepository::list_by_ids(db, &involved_ids)
        .await?
        .into_iter()
        .map(|p| (p.id, p))
        .collect();
    let upstream = TagRepository::active_picture_tokens_for(db, &involved_ids).await?;

    let owner_retention_days =
        crate::repository::user_settings::UserSettingsRepository::trash_retention_days(
            db,
            share.owner_id,
        )
        .await?;

    // ── Compute the announce set (with the token + updated_at to record on success) ────────
    let mut announce_items: Vec<AnnouncedPicture> = Vec::new();
    let mut announce_tokens: Vec<(Uuid, Uuid, chrono::NaiveDateTime)> = Vec::new();
    for pic in &covered {
        let Some(meta) = meta_by_id.get(pic) else {
            continue;
        };
        let desired_upstream = upstream.get(pic).copied(); // Some => received (relayed) picture
        let token = match tracking_map.get(pic) {
            None => match desired_upstream {
                Some(up) => up,                            // received: forward its upstream token
                None if meta.is_owned() => Uuid::new_v4(), // owned: mint a fresh token
                None => continue, // received but no active upstream token yet
            },
            Some((existing, announced_at)) => {
                let token_moved = matches!(desired_upstream, Some(up) if up != *existing);
                // Metadata re-announce (§10.3): owner's EXIF/geo changed since the last announce.
                let metadata_changed = announced_at.map_or(true, |a| meta.updated_at > a);
                if token_moved {
                    desired_upstream.unwrap() // received whose upstream token moved → re-announce
                } else if metadata_changed {
                    *existing // same token, deliver refreshed metadata
                } else {
                    continue; // already announced, token + metadata stable → skip
                }
            }
        };
        // Owner-purge deadline: owned + trashed → derived `deleted_at + retention`; received → sender-supplied
        let owner_purge_at = if meta.is_owned() {
            meta.deleted_at
                .map(|d| d + ChronoDuration::days(owner_retention_days as i64))
        } else {
            meta.owner_purge_at
        };
        announce_items.push(AnnouncedPicture::from_picture(
            meta,
            token,
            sender_username,
            &run.config.global_domain,
            owner_purge_at,
        ));
        announce_tokens.push((*pic, token, meta.updated_at));
    }

    // ── Compute the unannounce set (tracked but no longer covered) ────────────
    let mut unannounce_ids: Vec<String> = Vec::new();
    let mut unannounce_pics: Vec<Uuid> = Vec::new();
    for (pic, _tok, _at) in &tracking {
        if covered.contains(pic) {
            continue;
        }
        unannounce_ids.push(
            meta_by_id
                .get(pic)
                .map(announce_id)
                .unwrap_or_else(|| pic.to_string()),
        );
        unannounce_pics.push(*pic);
    }

    // ── Nothing to do → activate a fully-consistent PFA/Errored share ─────────
    if announce_items.is_empty() && unannounce_ids.is_empty() {
        if needs_activation(share) {
            OutgoingShareRepository::mark_announce_success(db, share.id).await?;
        }
        return Ok(());
    }

    // ── Deliver-then-record ───────────────────────────────────────────────────
    let same_backend = find_local_user_id(
        run.cache,
        db,
        run.config,
        &share.recipient_username,
        &share.recipient_instance,
    )
    .await?
    .is_some();
    let mut ok = true;

    if !announce_items.is_empty() {
        match deliver_announce(run, share, sender_username, same_backend, announce_items).await {
            Ok(()) => {
                let mut tx = db.begin().await.map_err(map_sqlx_error)?;
                for (pic, token, updated_at) in &announce_tokens {
                    ShareAnnouncementRepository::insert_with_token(
                        &mut *tx,
                        share.id,
                        *pic,
                        *token,
                        Some(*updated_at),
                    )
                    .await?;
                }
                tx.commit().await.map_err(map_sqlx_error)?;
            }
            Err(e) => {
                tracing::error!(share_id = %share.id, error = ?e, "pipeline: announce delivery failed");
                ok = false;
            }
        }
    }

    if !unannounce_ids.is_empty() {
        match deliver_unannounce(run, share, sender_username, same_backend, &unannounce_ids).await {
            Ok(()) => {
                let mut tx = db.begin().await.map_err(map_sqlx_error)?;
                for pic in &unannounce_pics {
                    ShareAnnouncementRepository::delete(&mut *tx, share.id, *pic).await?;
                }
                tx.commit().await.map_err(map_sqlx_error)?;
            }
            Err(e) => {
                tracing::error!(share_id = %share.id, error = ?e, "pipeline: unannounce delivery failed");
                ok = false;
            }
        }
    }

    // ── Status transition ─────────────────────────────────────────────────────
    if ok {
        if needs_activation(share) {
            OutgoingShareRepository::mark_announce_success(db, share.id).await?;
        }
    } else {
        let next = (Utc::now() + ChronoDuration::seconds(run.config.pipeline_retry_backoff_secs))
            .naive_utc();
        // An `active` share whose incremental pass failed is demoted to `errored` so the next pass
        // is a full reconcile; PFA/Errored keep their status and just back off.
        let demote = share.status == ShareStatus::Active;
        OutgoingShareRepository::mark_announce_failure(db, share.id, demote, next).await?;
    }
    Ok(())
}

/// Deliver an announce inline. Same-backend registers against the local recipient (and wakes its
/// pipeline); cross-instance posts to the recipient's `/pictures/announce`. `items` is consumed.
async fn deliver_announce(
    run: &PipelineRun<'_>,
    share: &OutgoingShare,
    sender_username: &str,
    same_backend: bool,
    items: Vec<AnnouncedPicture>,
) -> Result<(), AppError> {
    if same_backend {
        let incoming = IncomingShareRepository::find_by_outgoing_share(
            run.db,
            share.id,
            &run.config.global_domain,
        )
        .await?
        .ok_or(AppError::NotFound)?;
        // The recipient must have accepted; otherwise this would record phantom tracking. Surface
        // it as an error so nothing is recorded and the share backs off and retries.
        if incoming.status != ShareStatus::Active {
            return Err(AppError::Conflict(
                "recipient incoming share is not active".to_string(),
            ));
        }
        let shared_tag = TagPath::shared_to_me(
            sender_username,
            &run.config.global_domain,
            &TagPath::from_ltree(&share.tag_path),
        );
        register_received_pictures(
            run.db,
            incoming.recipient_id,
            incoming.id,
            &shared_tag,
            &items,
        )
        .await?;
        run.waker.wake(incoming.recipient_id);
        Ok(())
    } else {
        run.federation
            .announce_pictures_to_backend(
                sender_username,
                &share.recipient_username,
                &share.recipient_instance,
                &PicturesAnnouncementRequest {
                    outgoing_share_id: share.id,
                    tag_path: share.tag_path.clone(),
                    sender_username: sender_username.to_string(),
                    sender_instance: run.config.global_domain.clone(),
                    pictures: items,
                },
            )
            .await
    }
}

/// Deliver an unannounce inline. Same-backend unregisters locally (and wakes the recipient);
/// cross-instance posts to the recipient's `/pictures/unannounce`.
async fn deliver_unannounce(
    run: &PipelineRun<'_>,
    share: &OutgoingShare,
    sender_username: &str,
    same_backend: bool,
    picture_ids: &[String],
) -> Result<(), AppError> {
    if same_backend {
        let incoming = IncomingShareRepository::find_by_outgoing_share(
            run.db,
            share.id,
            &run.config.global_domain,
        )
        .await?
        .ok_or(AppError::NotFound)?;
        unregister_announced_pictures(run.db, &incoming, picture_ids).await?;
        run.waker.wake(incoming.recipient_id);
        Ok(())
    } else {
        run.federation
            .unannounce_pictures_to_backend(
                sender_username,
                &share.recipient_username,
                &share.recipient_instance,
                &PicturesUnannouncementRequest {
                    outgoing_share_id: share.id,
                    sender_username: sender_username.to_string(),
                    sender_instance: run.config.global_domain.clone(),
                    picture_ids: picture_ids.to_vec(),
                },
            )
            .await
    }
}
