//! Content-dedup operations outside the pipeline reconciler. Group *reconciliation* is routine work
//! (`routines::pipeline::dedup`); arrival classification is a plain repository-driven step callers
//! run inline. Model: doc/features/11 §5.

use crate::domain::picture::DeletedReason;
use crate::repository::dedup::{DedupRepository, DedupRow};
use archypix_common::error::AppError;
use sqlx::PgPool;
use uuid::Uuid;

/// Boomerang guard (§5.4): a copy arriving into a Rejected group (a `manual`/`boomerang` sibling, no
/// live) is itself trashed as `boomerang` before it is shown; otherwise left as created (live) and
/// the reconciler collapses/promotes it. See doc/features/11.
#[tracing::instrument(skip(db), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn classify_arrival(
    db: &PgPool,
    user_id: Uuid,
    picture_id: Uuid,
) -> Result<(), AppError> {
    let Some(key) = DedupRepository::content_key_of(db, picture_id).await? else {
        return Ok(()); // no content/file hash yet → nothing to group on
    };
    let rows = DedupRepository::list_group_rows(db, user_id, &key).await?;
    let others: Vec<&DedupRow> = rows.iter().filter(|r| r.id != picture_id).collect();
    let deleted = others.iter().any(|r| {
        matches!(
            r.deleted_reason,
            Some(DeletedReason::Manual | DeletedReason::Boomerang)
        )
    });
    if deleted {
        DedupRepository::set_boomerang(db, picture_id).await?;
    }
    Ok(())
}
