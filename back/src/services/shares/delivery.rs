//! Best-effort task delivery for the revocation-cascade unannounce: same-backend operations run
//! directly against the DB; cross-instance ones post to the recipient's federation endpoint. Called
//! by the task runner (`infra::tasks`) for `UnannounceSharedPictures`.
//!
//! Note: the *pipeline* announces/unannounces inline (see `infra::pipeline::announcement`). The only
//! task path left here is the best-effort downstream unannounce emitted by `cleanup_incoming_share`.

use crate::clients::federation::FederationClient;
use crate::clients::federation::models::PicturesUnannouncementRequest;
use crate::infra::config::Config;
use crate::infra::error::AppError;
use crate::infra::pipeline::PipelineWaker;
use crate::infra::tasks::InternalTask;
use crate::repository::share::IncomingShareRepository;
use crate::services::shares::registration::unregister_announced_pictures;

/// Deliver an `UnannounceSharedPictures` task: same-backend removes tags directly, cross-instance
/// posts to the recipient's `/pictures/unannounce`.
#[tracing::instrument(skip(db, federation, config, pipeline_waker, task))]
pub async fn deliver_unannounce_task(
    db: &sqlx::PgPool,
    federation: &FederationClient,
    config: &Config,
    pipeline_waker: &PipelineWaker,
    task: InternalTask,
) -> Result<(), AppError> {
    let InternalTask::UnannounceSharedPictures {
        outgoing_share_id,
        sender_username,
        recipient_username,
        recipient_instance,
        picture_ids,
        is_same_backend,
    } = task
    else {
        return Ok(());
    };

    if is_same_backend {
        let Some(incoming) = IncomingShareRepository::find_by_outgoing_share(
            db,
            outgoing_share_id,
            &config.global_domain,
        )
        .await?
        else {
            return Ok(());
        };
        unregister_announced_pictures(db, &incoming, &picture_ids).await?;
        pipeline_waker.wake(incoming.recipient_id);
    } else {
        federation
            .unannounce_pictures_to_backend(
                &sender_username,
                &recipient_username,
                &recipient_instance,
                &PicturesUnannouncementRequest {
                    outgoing_share_id,
                    sender_username: sender_username.clone(),
                    sender_instance: config.global_domain.clone(),
                    picture_ids,
                },
            )
            .await?;
    }
    Ok(())
}
