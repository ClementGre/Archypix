//! ShareBack auto-accept: when a user shares back to someone who allowed it, the recipient's
//! incoming share is auto-activated and an automatic per-share `SharedTagMappingService` is created.

use crate::domain::share::{IncomingShare, OutgoingShare, ShareStatus};
use crate::domain::tagging::ServiceType;
use crate::infra::error::AppError;
use crate::infra::routine::RoutineHandle;
use crate::repository::share::IncomingShareRepository;
use crate::repository::tagging::TaggingServiceRepository;
use sqlx::PgPool;
use uuid::Uuid;

/// Local part of a ShareBack auto-accept: transition the IncomingShare to Active and create +
/// link the automatic per-share `SharedTagMappingService` pointing back at the original tag. No
/// pictures are registered here — the initiator's pictures are announced by its pipeline once its
/// OutgoingShare is moved to `pending_first_announcement` (cross-instance: the initiator does this
/// on the `auto_accepted` response; same-backend: `create_outgoing_share` does it).
#[tracing::instrument(skip(db, pipeline_waker, incoming, original_outgoing), fields(user_id = %recipient_id, share_id = %incoming.id))]
pub async fn auto_accept_shareback_local(
    db: &PgPool,
    pipeline_waker: &RoutineHandle<Uuid>,
    recipient_id: Uuid,
    incoming: &IncomingShare,
    original_outgoing: &OutgoingShare,
) -> Result<(), AppError> {
    IncomingShareRepository::set_status(db, incoming.id, ShareStatus::Active).await?;

    // One shared_tag_mapping service per incoming share (feature 20 §10.1).
    let config = serde_json::json!({
        "incoming_share_id": incoming.id,
        "assign_tags": [original_outgoing.tag_path],
    });
    let service = TaggingServiceRepository::create(
        db,
        recipient_id,
        ServiceType::SharedTagMapping,
        "Share-back",
        &[],
        &[],
        &config,
    )
    .await?;
    // `incoming_shares.local_mapping_service_id` now references the service itself.
    IncomingShareRepository::set_local_mapping_service(db, incoming.id, service.id).await?;
    TaggingServiceRepository::touch_invalidated(db, service.id).await?;

    pipeline_waker.trigger(recipient_id);
    Ok(())
}
