use crate::clients::federation::FederationClient;
use crate::clients::federation::models::AnnouncedPicture;
use crate::domain::share::ShareStatus;
use crate::domain::tag::TagPath;
use crate::infra::config::Config;
use crate::infra::error::AppError;
use crate::infra::pipeline::PipelineWaker;
use crate::infra::redis::Cache;
use crate::infra::s3::{self, Storage};
use crate::repository::picture::PictureRepository;
use crate::repository::share::{IncomingShareRepository, OutgoingShareRepository};
use crate::repository::share_announcement::ShareAnnouncementRepository;
use crate::repository::user::UserRepository;
use crate::services::pictures::PictureVariant;
use crate::services::shares::{register_received_pictures, unregister_announced_pictures};
use crate::services::users::find_local_user_id;
use sqlx::PgPool;
use tracing::warn;
use uuid::Uuid;

pub struct PresignTokenItem {
    pub picture_token: Uuid,
    pub variant: Option<String>,
}

/// Validate and record an inbound share announcement from a remote instance.
/// Returns incoming share ID and a boolean indicating if the share was automatically accepted.
pub async fn receive_share_announcement(
    db: &PgPool,
    config: &Config,
    pipeline_waker: &PipelineWaker,
    authenticated_instance: &str,
    sender_username: &str,
    sender_instance: &str,
    recipient_username: &str,
    recipient_instance: &str,
    outgoing_share_id: Uuid,
    allow_share_back: bool,
    shareback_of: Option<Uuid>,
) -> Result<(Uuid, bool), AppError> {
    if recipient_instance != config.global_domain {
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
    let recipient = UserRepository::find_by_username(db, recipient_username)
        .await?
        .ok_or(AppError::NotFound)?;

    // Cap the number of outstanding `pending` incoming shares per recipient
    let pending_incoming = IncomingShareRepository::list_by_recipient(db, recipient.id)
        .await?
        .into_iter()
        .filter(|s| s.status == ShareStatus::Pending)
        .count();
    if pending_incoming >= config.max_pending_incoming_shares {
        warn!(
            recipient = recipient_username,
            pending_incoming,
            "federation: announce_share rejected: recipient pending-share cap reached"
        );
        return Err(AppError::TooManyRequests(
            "Recipient has too many pending shares".to_string(),
        ));
    }

    let incoming = IncomingShareRepository::create(
        db,
        recipient.id,
        sender_username,
        sender_instance,
        outgoing_share_id,
        allow_share_back,
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
pub async fn receive_share_accept(
    db: &PgPool,
    pipeline_waker: &PipelineWaker,
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
    pipeline_waker.wake(share.owner_id);
    Ok(())
}

/// Received a share revocation from the sender; clean up the matching IncomingShare.
#[allow(clippy::too_many_arguments)]
pub async fn receive_share_revoke(
    db: &PgPool,
    cache: &dyn Cache,
    federation: &FederationClient,
    config: &Config,
    task_queue: &crate::infra::tasks::TaskQueue,
    pipeline_waker: &PipelineWaker,
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
        config,
        task_queue,
        pipeline_waker,
        &share,
        ShareStatus::Revoked,
    )
    .await
}

/// Received a share rejection from the recipient; tombstone the OutgoingShare.
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
        }
    }

    Ok(())
}

/// Received a batch of pictures from a sender; register them under the active IncomingShare.
/// Loop prevention: pictures whose owner is a local user (the relayed picture is our own) are
/// skipped.
#[allow(clippy::too_many_arguments)]
pub async fn receive_pictures_announcement(
    db: &PgPool,
    cache: &dyn Cache,
    config: &Config,
    pipeline_waker: &PipelineWaker,
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
            config,
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
        pipeline_waker.wake(incoming.recipient_id);
    }
    Ok(registered)
}

/// Received a per-picture unannounce from a sender; remove the share's tags from the named
/// pictures and delete now-orphaned received-picture rows.
pub async fn receive_pictures_unannouncement(
    db: &PgPool,
    pipeline_waker: &PipelineWaker,
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
    pipeline_waker.wake(incoming.recipient_id);
    Ok(deleted)
}

/// Resolve per-picture tokens to owned pictures and presign each. The token *is* the
/// authorization — no federation JWT is required. An unknown token yields 401.
pub async fn presign_by_picture_tokens(
    db: &PgPool,
    storage: &dyn Storage,
    config: &Config,
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
        let url = storage.presign_get(variant.bucket(config), &key).await?;
        results.push((item.picture_token, url));
    }
    Ok(results)
}
