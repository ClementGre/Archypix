use crate::infra::error::{AppError, map_sqlx_error};
use crate::infra::routine::RoutineHandle;
use crate::repository::picture::{PictureRepository, ResolvedSelection};
use crate::repository::pipeline::PipelineRepository;
use crate::repository::tag::TagRepository;
use crate::services::aggregate::DryRun;
use sqlx::PgPool;
use uuid::Uuid;

/// Result of a batch tag edit: either the dry-run breakdown or the applied count.
pub enum TagBatchOutcome {
    DryRun(DryRun),
    Applied { affected: i64 },
}

/// Add/remove tags across a [`ResolvedSelection`] (feature 14 §6.4). Removal only affects `manual`
/// rows (so the removable count reflects `manual_count`, not `count`). With `dry_run` the call
/// computes the §6.1 breakdown without mutating; otherwise it resolves the set inside the
/// transaction, applies remove-then-add atomically, invalidates the pipeline, and wakes it.
#[tracing::instrument(skip(db, waker, sel, add_tags, remove_tags), fields(user_id = %user_id, dry_run))]
pub async fn batch_edit_tags(
    db: &PgPool,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    sel: &ResolvedSelection,
    add_tags: &[String],
    remove_tags: &[String],
    dry_run: bool,
) -> Result<TagBatchOutcome, AppError> {
    if add_tags.is_empty() && remove_tags.is_empty() {
        return Err(AppError::BadRequest(
            "at least one of add_tags or remove_tags must be non-empty".to_string(),
        ));
    }
    // A selection that names neither a query nor an explicit picture can never match anything; reject
    // it like the legacy empty-`picture_ids` guard. (A query that *resolves* to zero rows is still a
    // valid no-op — its `filter` is `Some`, so `is_empty()` is false.)
    if sel.is_empty() {
        return Err(AppError::BadRequest(
            "selection is empty: provide a query or at least one picture".to_string(),
        ));
    }

    if dry_run {
        let affected = PictureRepository::count_selection(db, user_id, sel).await?;
        let removed = if remove_tags.is_empty() {
            None
        } else {
            Some(
                TagRepository::count_selection_with_manual_under(db, user_id, sel, remove_tags)
                    .await?,
            )
        };
        return Ok(TagBatchOutcome::DryRun(DryRun {
            affected,
            added: (!add_tags.is_empty()).then_some(affected),
            removed,
            ..Default::default()
        }));
    }

    let mut tx = db.begin().await.map_err(map_sqlx_error)?;
    let ids = PictureRepository::resolve_selection_ids(&mut *tx, user_id, sel).await?;
    if ids.is_empty() {
        tx.commit().await.map_err(map_sqlx_error)?;
        return Ok(TagBatchOutcome::Applied { affected: 0 });
    }
    TagRepository::batch_remove(&mut *tx, user_id, &ids, remove_tags).await?;
    TagRepository::batch_assign(&mut *tx, user_id, &ids, add_tags).await?;
    // Manual tag changes re-dirty the pictures so the pipeline re-evaluates requires/excludes gates.
    PipelineRepository::invalidate(&mut *tx, &ids).await?;
    tx.commit().await.map_err(map_sqlx_error)?;
    waker.trigger(user_id);
    Ok(TagBatchOutcome::Applied {
        affected: ids.len() as i64,
    })
}
