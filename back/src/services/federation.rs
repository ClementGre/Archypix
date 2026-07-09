use crate::clients::federation::FederationClient;
use crate::clients::federation::models::AnnouncedPicture;
use crate::domain::share::ShareStatus;
use crate::domain::tag::TagPath;
use crate::infra::redis::Cache;
use crate::infra::routine;
use crate::infra::routine::RoutineHandle;
use crate::infra::s3::{self, Storage};
use crate::infra::settings::keys;
use crate::repository::picture::PictureRepository;
use crate::repository::share::{IncomingShareRepository, OutgoingShareRepository};
use crate::repository::share_announcement::ShareAnnouncementRepository;
use crate::repository::user::UserRepository;
use crate::services::pictures::PictureVariant;
use crate::services::shares::{register_received_pictures, unregister_announced_pictures};
use crate::services::users::find_local_user_id;
use archypix_common::error::AppError;
use archypix_common::settings::Settings;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::warn;
use uuid::Uuid;

pub struct PresignTokenItem {
    pub picture_token: Uuid,
    pub variant: Option<String>,
}

/// Validate and record an inbound share announcement from a remote instance.
/// Returns incoming share ID and a boolean indicating if the share was automatically accepted.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, settings, pipeline_waker), fields(outgoing_share_id = %outgoing_share_id))]
pub async fn receive_share_announcement(
    db: &PgPool,
    settings: &Settings,
    pipeline_waker: &RoutineHandle<Uuid>,
    authenticated_instance: &str,
    sender_username: &str,
    sender_instance: &str,
    recipient_username: &str,
    recipient_instance: &str,
    outgoing_share_id: Uuid,
    tag_path: &str,
    name: &str,
    message: Option<&str>,
    allow_share_back: bool,
    allow_exif_edit: bool,
    future: bool,
    shareback_of: Option<Uuid>,
) -> Result<(Uuid, bool), AppError> {
    if recipient_instance != settings.get(keys::GLOBAL_DOMAIN) {
        warn!(
            sender_instance,
            recipient_instance, "federation: announce_share rejected — invalid recipient instance"
        );
        return Err(AppError::BadRequest(
            "Invalid recipient instance".to_string(),
        ));
    }
    if sender_instance != authenticated_instance {
        warn!(
            authenticated_instance,
            sender_instance, "federation: announce_share rejected — sender instance mismatch"
        );
        return Err(AppError::Unauthorized(
            "Sender instance does not match authenticated instance".to_string(),
        ));
    }
    // Name/message come from a remote instance: validate before persisting.
    crate::domain::validation::validate_share_name(name).map_err(AppError::BadRequest)?;
    crate::domain::validation::validate_share_message(message).map_err(AppError::BadRequest)?;

    let recipient = UserRepository::find_by_username(db, recipient_username)
        .await?
        .ok_or(AppError::NotFound)?;

    // Cap the number of outstanding `pending` incoming shares per recipient
    let pending_incoming = IncomingShareRepository::list_by_recipient(db, recipient.id)
        .await?
        .into_iter()
        .filter(|s| s.status == ShareStatus::Pending)
        .count();
    if pending_incoming >= settings.get(keys::MAX_PENDING_INCOMING_SHARES) {
        warn!(
            recipient = recipient_username,
            pending_incoming,
            "federation: announce_share rejected: recipient pending-share cap reached"
        );
        return Err(AppError::TooManyRequests(
            "Recipient has too many pending shares".to_string(),
        ));
    }

    // Advisory local tag these pictures will land under, for the recipient's UI even before the
    // first picture announcement arrives.
    let shared_tag = TagPath::shared_to_me(
        sender_username,
        sender_instance,
        &TagPath::from_ltree(tag_path),
    );
    let incoming = IncomingShareRepository::create(
        db,
        recipient.id,
        sender_username,
        sender_instance,
        name,
        message,
        outgoing_share_id,
        allow_share_back,
        allow_exif_edit,
        future,
        Some(shared_tag.as_ltree()),
        shareback_of,
    )
    .await?;

    // ── ShareBack auto-accept ────────────────────────────────────
    // If this announcement references one of the recipient's own outgoing shares (the one the
    // sender is sharing back) and that share permits it, auto-accept locally and wire up the
    // mapping. The picture registration is driven by the sender's own follow-up announcement.
    let mut auto_accepted = false;
    if let Some(original_os_id) = shareback_of {
        if let Some(original) = OutgoingShareRepository::get_by_id(db, original_os_id).await? {
            let verified = original.owner_id == recipient.id
                && original.recipient_username == sender_username
                && original.recipient_instance == sender_instance
                && original.allow_share_back;
            if verified {
                crate::services::shares::auto_accept_shareback_local(
                    db,
                    pipeline_waker,
                    recipient.id,
                    &incoming,
                    &original,
                )
                .await?;
                auto_accepted = true;
            }
        }
    }

    Ok((incoming.id, auto_accepted))
}

/// Alice received Bob's accept notification: move the OutgoingShare to
/// `pending_first_announcement` and wake the pipeline, which announces the current coverage and
/// flips the share to `active`. (The actual picture announcement is the pipeline's job — the
/// single announce path.)
#[tracing::instrument(skip(db, pipeline_waker), fields(outgoing_share_id = %outgoing_share_id))]
pub async fn receive_share_accept(
    db: &PgPool,
    pipeline_waker: &RoutineHandle<Uuid>,
    authenticated_instance: &str,
    outgoing_share_id: Uuid,
) -> Result<(), AppError> {
    let share = OutgoingShareRepository::get_by_id(db, outgoing_share_id)
        .await?
        .ok_or(AppError::NotFound)?;

    match share.status {
        ShareStatus::Pending
        | ShareStatus::PendingFirstAnnouncement
        | ShareStatus::Active
        | ShareStatus::Errored => {}
        ShareStatus::Revoked | ShareStatus::Tombstoned => return Err(AppError::NotFound),
    }

    if share.recipient_instance != authenticated_instance {
        warn!(
            %outgoing_share_id,
            recipient_instance = %share.recipient_instance,
            authenticated = authenticated_instance,
            "federation: accept_share rejected — instance mismatch"
        );
        return Err(AppError::Unauthorized(
            "Authenticated instance is not the share recipient".to_string(),
        ));
    }

    // Already announced/active → idempotent no-op.
    if share.status == ShareStatus::Active {
        return Ok(());
    }

    OutgoingShareRepository::set_status(db, share.id, ShareStatus::PendingFirstAnnouncement)
        .await?;
    // The sender (this share's owner) must run its pipeline to announce the first coverage.
    pipeline_waker.trigger(share.owner_id);
    Ok(())
}

/// Received a share revocation from the sender; clean up the matching IncomingShare.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, federation, settings, task_queue, pipeline_waker), fields(outgoing_share_id = %outgoing_share_id))]
pub async fn receive_share_revoke(
    db: &PgPool,
    cache: &dyn Cache,
    federation: &FederationClient,
    settings: &Settings,
    task_queue: &RoutineHandle<routine::unannounce::UnannounceInput>,
    pipeline_waker: &RoutineHandle<Uuid>,
    authenticated_instance: &str,
    outgoing_share_id: Uuid,
) -> Result<u64, AppError> {
    let share = IncomingShareRepository::find_by_outgoing_share(
        db,
        outgoing_share_id,
        authenticated_instance,
    )
    .await?
    .ok_or(AppError::NotFound)?;
    crate::services::shares::cleanup_incoming_share(
        db,
        cache,
        federation,
        &settings,
        task_queue,
        pipeline_waker,
        &share,
        ShareStatus::Revoked,
    )
    .await
}

/// Received a share rejection from the recipient; tombstone the OutgoingShare.
#[tracing::instrument(skip(db), fields(outgoing_share_id = %outgoing_share_id))]
pub async fn receive_share_reject(
    db: &PgPool,
    authenticated_instance: &str,
    outgoing_share_id: Uuid,
) -> Result<(), AppError> {
    let share = OutgoingShareRepository::get_by_id(db, outgoing_share_id)
        .await?
        .ok_or(AppError::NotFound)?;

    if share.recipient_instance != authenticated_instance {
        warn!(
            %outgoing_share_id,
            recipient_instance = %share.recipient_instance,
            authenticated = authenticated_instance,
            "federation: reject_share rejected — instance mismatch"
        );
        return Err(AppError::Unauthorized(
            "Authenticated instance is not the share recipient".to_string(),
        ));
    }

    match share.status {
        ShareStatus::Tombstoned => {}
        ShareStatus::Revoked => return Err(AppError::NotFound),
        ShareStatus::Pending
        | ShareStatus::PendingFirstAnnouncement
        | ShareStatus::Active
        | ShareStatus::Errored => {
            OutgoingShareRepository::set_status(db, share.id, ShareStatus::Tombstoned).await?;
            // Rejected by the recipient — invalidate every presign token this share held, exactly
            // like revocation (the recipient will never fetch these pictures again).
            ShareAnnouncementRepository::delete_all_for_share(db, share.id).await?;
        }
    }

    Ok(())
}

/// Received a batch of pictures from a sender; register them under the active IncomingShare.
/// Loop prevention: pictures whose owner is a local user (the relayed picture is our own) are
/// skipped.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, settings, pipeline_waker, pictures), fields(outgoing_share_id = %outgoing_share_id))]
pub async fn receive_pictures_announcement(
    db: &PgPool,
    cache: &dyn Cache,
    settings: &Settings,
    pipeline_waker: &RoutineHandle<Uuid>,
    authenticated_instance: &str,
    sender_username: &str,
    sender_instance: &str,
    outgoing_share_id: Uuid,
    tag_path: &str,
    pictures: Vec<AnnouncedPicture>,
) -> Result<usize, AppError> {
    if sender_instance != authenticated_instance {
        return Err(AppError::Unauthorized(
            "Sender instance does not match authenticated instance".to_string(),
        ));
    }

    let incoming =
        IncomingShareRepository::find_by_outgoing_share(db, outgoing_share_id, sender_instance)
            .await?
            .ok_or(AppError::NotFound)?;

    // Bind the announced sender to the share that created it: the `/SharedToMe/<sender>/…` tag is
    // built from `sender_username`, so a peer instance must not relabel pictures under a different
    // sender than the one recorded on the incoming share.
    if incoming.sender_username != sender_username {
        warn!(
            announced_sender = sender_username,
            recorded_sender = %incoming.sender_username,
            "federation: announce_pictures rejected: sender username mismatch"
        );
        return Err(AppError::Unauthorized(
            "Sender username does not match the incoming share".to_string(),
        ));
    }

    if incoming.status != ShareStatus::Active {
        return Err(AppError::NotFound);
    }

    // Loop prevention: drop any picture whose owner resolves to the local recipient.
    let mut kept: Vec<AnnouncedPicture> = Vec::with_capacity(pictures.len());
    for pic in pictures {
        if let Some(owner_id) = find_local_user_id(
            cache,
            db,
            settings,
            &pic.owner_username,
            &pic.owner_instance_domain,
        )
        .await?
        {
            if owner_id == incoming.recipient_id {
                continue;
            }
        }
        kept.push(pic);
    }

    let shared_tag = TagPath::shared_to_me(
        sender_username,
        sender_instance,
        &TagPath::from_ltree(tag_path),
    );

    let registered =
        register_received_pictures(db, incoming.recipient_id, incoming.id, &shared_tag, &kept)
            .await?;
    // Newly received pictures start with last_pipeline_run_at = NULL → wake the recipient's pipeline.
    if registered > 0 {
        pipeline_waker.trigger(incoming.recipient_id);
    }
    Ok(registered)
}

/// Received a per-picture unannounce from a sender; remove the share's tags from the named
/// pictures and delete now-orphaned received-picture rows.
#[tracing::instrument(skip(db, pipeline_waker, picture_ids), fields(outgoing_share_id = %outgoing_share_id))]
pub async fn receive_pictures_unannouncement(
    db: &PgPool,
    pipeline_waker: &RoutineHandle<Uuid>,
    authenticated_instance: &str,
    outgoing_share_id: Uuid,
    picture_ids: &[String],
) -> Result<u64, AppError> {
    let incoming = IncomingShareRepository::find_by_outgoing_share(
        db,
        outgoing_share_id,
        authenticated_instance,
    )
    .await?
    .ok_or(AppError::NotFound)?;

    let deleted = unregister_announced_pictures(db, &incoming, picture_ids).await?;
    pipeline_waker.trigger(incoming.recipient_id);
    Ok(deleted)
}

/// Owner-side handler for a recipient EXIF edit proposal (10 §4.2). Re-verifies the grant — an
/// **active** `OutgoingShare` to the requester with `allow_exif_edit` covering the picture (never
/// trusts the wire) — validates and applies the edit via the owner's existing `edit_picture`
/// write-through (`edit_pictures_exif`), which bumps `updated_at`, marks the picture dirty, and
/// wakes the pipeline so the metadata change re-announces to **all** recipients (incl. the
/// requester). Used by both the cross-instance federation handler and the same-backend short-circuit
/// in `services::pictures::propose_received_exif`.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, waker, set, clear), fields(picture_id))]
pub async fn receive_picture_edit_request(
    db: &PgPool,
    waker: &RoutineHandle<Uuid>,
    picture_id: &str,
    requester_username: &str,
    requester_instance: &str,
    set: crate::domain::job::FullExif,
    clear: Vec<crate::domain::job::ExifField>,
) -> Result<(), AppError> {
    let picture_id: Uuid = picture_id
        .parse()
        .map_err(|_| AppError::BadRequest("invalid picture_id".to_string()))?;

    // The picture must be owned (stored) on this backend. A relayer never applies a proposal — it is
    // addressed to the owner's backend (10 §5, transitive shares).
    let picture = PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if !picture.is_owned() {
        return Err(AppError::NotFound);
    }

    // Authorisation: an active grant to this requester covering the picture. Re-checked here so a
    // revoked-in-flight grant is rejected (10 §6.2).
    if OutgoingShareRepository::find_active_exif_editable_covering(
        db,
        picture_id,
        requester_username,
        requester_instance,
    )
    .await?
    .is_none()
    {
        warn!(
            %picture_id,
            requester = requester_username,
            requester_instance,
            "federation: picture edit request rejected — no active EXIF-edit grant covers it"
        );
        return Err(AppError::Forbidden(
            "no active share grants EXIF editing of this picture".to_string(),
        ));
    }

    // Apply through the owner's write-through. Reuses field validation (GPS/orientation/set∪clear),
    // the still-processing 409 guard, MIME preflight, the §5 in-flight rule, and the re-announce wake.
    crate::services::jobs::edit_pictures_exif(
        db,
        waker,
        picture.local_user_id,
        &[picture_id],
        set,
        clear,
    )
    .await?;
    Ok(())
}

/// Resolve per-picture tokens to owned pictures and presign each. The token *is* the
/// authorization — no federation JWT is required. An unknown token yields 401.
#[tracing::instrument(skip(db, storage, settings, items))]
pub async fn presign_by_picture_tokens(
    db: &PgPool,
    storage: &dyn Storage,
    settings: &Settings,
    items: &[PresignTokenItem],
) -> Result<Vec<(Uuid, String)>, AppError> {
    let mut results = Vec::with_capacity(items.len());
    for item in items {
        let picture_id = ShareAnnouncementRepository::find_picture_by_token(db, item.picture_token)
            .await?
            .ok_or_else(|| {
                AppError::Unauthorized("picture_token does not match any share".to_string())
            })?;
        let picture = PictureRepository::find_by_id(db, picture_id)
            .await?
            .ok_or(AppError::NotFound)?;
        if !picture.is_owned() {
            // A tracking token must point at a picture this backend actually stores.
            return Err(AppError::NotFound);
        }
        let variant: PictureVariant = item.variant.as_deref().unwrap_or("original").parse()?;
        let key = s3::picture_key(picture.local_user_id, picture.id);
        let url = storage.presign_get(&variant.bucket(settings), &key).await?;
        results.push((item.picture_token, url));
    }
    Ok(results)
}
