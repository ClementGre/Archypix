use crate::infra::error::{AppError, map_sqlx_error};
use crate::repository::tag::TagRepository;
use crate::repository::tagging::TaggingServiceRepository;
use sqlx::PgPool;
use uuid::Uuid;

/// Delete a tagging service, promoting every tag it assigned to `manual` so the user's
/// curation survives the deletion. Promotion and deletion share one transaction.
///
/// Returns `false` if the service does not exist or is not owned by `owner_id` (in which
/// case the transaction is rolled back and no tags are promoted).
#[tracing::instrument(skip(db), fields(user_id = %owner_id, service_id = %service_id))]
pub async fn delete_service(
    db: &PgPool,
    owner_id: Uuid,
    service_id: Uuid,
    promote_tags: bool,
) -> Result<bool, AppError> {
    let mut tx = db.begin().await.map_err(map_sqlx_error)?;
    if promote_tags {
        TagRepository::promote_service_tags_to_manual(&mut *tx, service_id).await?;
    } else {
        TagRepository::remove_service_tags(&mut *tx, service_id).await?;
    }
    let deleted = TaggingServiceRepository::delete(&mut *tx, owner_id, service_id).await?;
    if !deleted {
        // Not owned / not found — undo the promotion.
        tx.rollback().await.map_err(map_sqlx_error)?;
        return Ok(false);
    }
    tx.commit().await.map_err(map_sqlx_error)?;
    Ok(true)
}
