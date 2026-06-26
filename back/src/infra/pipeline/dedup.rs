//! Content-dedup reconciler (feature 11 §5) — runs serial per user in the pipeline, driving each
//! content group to its invariant: a **Live** group keeps one live survivor (rest `content_dedupe`);
//! a **Rejected** group (any `manual`/`boomerang` row) keeps one priority `manual` trash
//! representative (rest `boomerang`). Stable: a correct single-live group is never reshuffled. Full
//! model, lifecycle triggers and survivor priority: doc/features/11 §5.5 and doc/03 (feature 11).

use super::PipelineRun;
use crate::domain::picture::DeletedReason;
use crate::infra::error::{AppError, map_sqlx_error};
use crate::repository::dedup::{DedupRepository, DedupRow};
use sqlx::PgPool;
use uuid::Uuid;

/// Reconcile every content-dedup group of `user_id` that may need it. Idempotent and serial per user.
#[tracing::instrument(skip(run), fields(user_id = %user_id))]
pub async fn reconcile_for_user(run: &PipelineRun<'_>, user_id: Uuid) -> Result<(), AppError> {
    let keys = DedupRepository::find_candidate_keys(run.db, user_id).await?;
    for key in keys {
        reconcile_group(run.db, user_id, &key).await?;
    }
    Ok(())
}

/// Reconcile a single content group to the one-survivor invariant.
#[tracing::instrument(skip(db, user_id, key), fields(user_id = %user_id))]
pub async fn reconcile_group(db: &PgPool, user_id: Uuid, key: &str) -> Result<(), AppError> {
    let mut tx = db.begin().await.map_err(map_sqlx_error)?;
    let rows = DedupRepository::list_group_rows(&mut *tx, user_id, key).await?;

    let deleted = rows.iter().any(|r| {
        matches!(
            r.deleted_reason,
            Some(DeletedReason::Manual | DeletedReason::Boomerang)
        )
    });
    if deleted {
        // Rejected group: the priority copy (§5.1) is the single `manual` representative, the rest
        // `boomerang`. `best()` over the whole group — not just current `manual` rows — keeps it
        // stable (the delete trigger picks the same one). See doc/features/11 §5.5.
        let all: Vec<&DedupRow> = rows.iter().collect();
        let Some(rep) = best(&all) else {
            tx.commit().await.map_err(map_sqlx_error)?;
            return Ok(());
        };
        let rep_id = rep.id;
        for r in &rows {
            if r.id == rep_id {
                if r.deleted_reason != Some(DeletedReason::Manual) {
                    DedupRepository::set_manual(&mut *tx, r.id).await?;
                }
            } else if r.deleted_reason != Some(DeletedReason::Boomerang) {
                DedupRepository::set_boomerang(&mut *tx, r.id).await?;
            }
        }
    } else {
        // Exactly one non-deleted. Stable: group that already has exactly one live survivor is left untouched
        let live: Vec<&DedupRow> = rows.iter().filter(|r| r.deleted_at.is_none()).collect();
        match live.len() {
            1 => {} // already consistent — do nothing
            n if n > 1 => {
                // Transient (e.g. a fresh copy + its original both live) → keep one, hide the rest.
                let keep = best(&live).map(|r| r.id);
                for r in &live {
                    if Some(r.id) != keep {
                        DedupRepository::set_content_dedupe(&mut *tx, r.id).await?;
                    }
                }
            }
            _ => {
                // No live row → rescue-promote the best hidden copy.
                let dedupe: Vec<&DedupRow> = rows
                    .iter()
                    .filter(|r| r.deleted_reason == Some(DeletedReason::ContentDedupe))
                    .collect();
                if let Some(survivor) = best(&dedupe) {
                    DedupRepository::promote_to_live(&mut *tx, survivor.id).await?;
                }
            }
        }
    }

    tx.commit().await.map_err(map_sqlx_error)?;
    Ok(())
}

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

/// Highest-priority row (§5.1) by [`sort_key`]; `None` for an empty set.
fn best<'a>(candidates: &[&'a DedupRow]) -> Option<&'a DedupRow> {
    candidates
        .iter()
        .copied()
        .min_by(|a, b| sort_key(a).cmp(&sort_key(b)))
}

/// Ascending sort key — the minimum is the best survivor.
fn sort_key(r: &DedupRow) -> (bool, bool, bool, Uuid) {
    (
        r.owner_deleted_at.is_some(), // not-owner-deleted first
        !r.is_owned,                  // owned-by-me first
        r.is_copy,                    // original (not a copy) first
        r.id,                         // lowest id
    )
}
