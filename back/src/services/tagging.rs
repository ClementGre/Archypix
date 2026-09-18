use crate::domain::tagging::TaggingService;
use crate::infra::redis::Cache;
use crate::repository::tag::TagRepository;
use crate::repository::tagging::TaggingServiceRepository;
use archypix_common::error::{AppError, map_sqlx_error};
use sqlx::PgPool;
use uuid::Uuid;

/// Update a service's name / enabled flag / gates. Disabling one makes its tags no longer live, so
/// they are dropped here; re-enabling and every other config change re-derive on the next pipeline
/// run (via `touch_invalidated`). Returns `None` if the service is not the caller's.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache), fields(user_id = %owner_id, service_id = %service_id))]
pub async fn update_service(
    db: &PgPool,
    cache: &dyn Cache,
    owner_id: Uuid,
    service_id: Uuid,
    name: Option<&str>,
    enabled: Option<bool>,
    requires: Option<&[String]>,
    excludes: Option<&[String]>,
) -> Result<Option<TaggingService>, AppError> {
    let Some(service) =
        TaggingServiceRepository::update(db, owner_id, service_id, name, enabled, requires, excludes)
            .await?
    else {
        return Ok(None);
    };
    if enabled == Some(false) {
        TagRepository::remove_service_tags(db, service_id).await?;
        crate::services::tag_metadata::bust_cache(cache, owner_id).await;
    }
    TaggingServiceRepository::touch_invalidated(db, service_id).await?;
    Ok(Some(service))
}

/// Delete a tagging service, promoting every tag it assigned to `manual` so the user's
/// curation survives the deletion. Promotion and deletion share one transaction.
///
/// Returns `false` if the service does not exist or is not owned by `owner_id` (in which
/// case the transaction is rolled back and no tags are promoted).
#[tracing::instrument(skip(db, cache), fields(user_id = %owner_id, service_id = %service_id))]
pub async fn delete_service(
    db: &PgPool,
    cache: &dyn Cache,
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
    // The service's tags were promoted or removed synchronously (feature 34 §4).
    crate::services::tag_metadata::bust_cache(cache, owner_id).await;
    Ok(true)
}
