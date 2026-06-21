//! ShareBack auto-accept: when a user shares back to someone who allowed it, the recipient's
//! incoming share is auto-activated and an automatic `SharedTagMappingService` rule is created.

use crate::domain::share::{IncomingShare, OutgoingShare, ShareStatus};
use crate::domain::tagging::ServiceType;
use crate::infra::error::AppError;
use crate::infra::pipeline::PipelineWaker;
use crate::repository::share::IncomingShareRepository;
use crate::repository::tagging::{SharedTagMappingRuleRepository, TaggingServiceRepository};
use sqlx::PgPool;
use uuid::Uuid;

/// Find the user's first `shared_tag_mapping` service, creating an empty one if none exists.
/// Used by ShareBack auto-accept to attach the new mapping rule.
async fn find_or_create_shared_tag_mapping_service(
    db: &PgPool,
    owner_id: Uuid,
) -> Result<Uuid, AppError> {
    if let Some(id) =
        TaggingServiceRepository::first_mapping_service_for_owner(db, owner_id).await?
    {
        return Ok(id);
    }
    let svc =
        TaggingServiceRepository::create(db, owner_id, ServiceType::SharedTagMapping, &[], &[])
            .await?;
    Ok(svc.id)
}

/// Local part of a ShareBack auto-accept: transition the IncomingShare to Active and create +
/// link the automatic `SharedTagMappingService` rule pointing back at the original tag. No
/// pictures are registered here — the initiator's pictures are announced by its pipeline once its
/// OutgoingShare is moved to `pending_first_announcement` (cross-instance: the initiator does this
/// on the `auto_accepted` response; same-backend: `create_outgoing_share` does it).
#[tracing::instrument(skip(db, pipeline_waker, incoming, original_outgoing), fields(user_id = %recipient_id, share_id = %incoming.id))]
pub async fn auto_accept_shareback_local(
    db: &PgPool,
    pipeline_waker: &PipelineWaker,
    recipient_id: Uuid,
    incoming: &IncomingShare,
    original_outgoing: &OutgoingShare,
) -> Result<(), AppError> {
    IncomingShareRepository::set_status(db, incoming.id, ShareStatus::Active).await?;

    let service_id = find_or_create_shared_tag_mapping_service(db, recipient_id).await?;
    let mapping = SharedTagMappingRuleRepository::create(
        db,
        service_id,
        incoming.id,
        &original_outgoing.tag_path,
    )
    .await?;
    // `incoming_shares.local_mapping_service_id` FKs to the mapping-rule row (not the service).
    IncomingShareRepository::set_local_mapping_service(db, incoming.id, mapping.id).await?;
    TaggingServiceRepository::touch_invalidated(db, service_id).await?;

    pipeline_waker.wake(recipient_id);
    Ok(())
}
