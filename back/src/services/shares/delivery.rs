//! Best-effort delivery for the revocation-cascade unannounce: same-backend operations run directly
//! against the DB; cross-instance ones post to the recipient's federation endpoint. Called by the
//! `Unannounce` routine (`infra::routine::unannounce`).
//!
//! Note: the *pipeline* announces/unannounces inline (see `infra::routine::pipeline::announcement`). The only
//! path left here is the best-effort downstream unannounce emitted by `cleanup_incoming_share`.

use crate::clients::federation::FederationClient;
use crate::clients::federation::models::PicturesUnannouncementRequest;
use crate::infra::config::Config;
use crate::infra::error::AppError;
use crate::infra::routine::RoutineHandle;
use crate::infra::routine::unannounce::UnannounceInput;
use crate::repository::share::IncomingShareRepository;
use crate::services::shares::registration::unregister_announced_pictures;
use uuid::Uuid;

/// Deliver an `UnannounceInput`: same-backend removes tags directly (and wakes the recipient's
/// pipeline), cross-instance posts to the recipient's `/pictures/unannounce`.
#[tracing::instrument(skip(db, federation, config, pipeline, input))]
pub async fn deliver_unannounce(
    db: &sqlx::PgPool,
    federation: &FederationClient,
    config: &Config,
    pipeline: &RoutineHandle<Uuid>,
    input: UnannounceInput,
) -> Result<(), AppError> {
    let UnannounceInput {
        outgoing_share_id,
        sender_username,
        recipient_username,
        recipient_instance,
        picture_ids,
        is_same_backend,
    } = input;

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
        pipeline.trigger(incoming.recipient_id);
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
