use crate::infra::error::{AppError, map_sqlx_error};
use crate::repository::tag::TagRepository;
use sqlx::PgPool;
use uuid::Uuid;

#[tracing::instrument(skip(db, picture_ids, add_tags, remove_tags), fields(user_id = %user_id))]
pub async fn edit_picture_tags(
    db: &PgPool,
    user_id: Uuid,
    picture_ids: &[Uuid],
    add_tags: &[String],
    remove_tags: &[String],
) -> Result<(), AppError> {
    if picture_ids.is_empty() {
        return Err(AppError::BadRequest(
            "picture_ids must not be empty".to_string(),
        ));
    }
    if add_tags.is_empty() && remove_tags.is_empty() {
        return Err(AppError::BadRequest(
            "at least one of add_tags or remove_tags must be non-empty".to_string(),
        ));
    }

    let mut tx = db.begin().await.map_err(map_sqlx_error)?;
    TagRepository::batch_remove(&mut *tx, user_id, picture_ids, remove_tags).await?;
    TagRepository::batch_assign(&mut *tx, user_id, picture_ids, add_tags).await?;
    tx.commit().await.map_err(map_sqlx_error)?;

    Ok(())
}
