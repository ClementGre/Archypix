use crate::domain::hierarchy::TagPredicate;
use crate::repository::picture::PictureListFilter;
use archypix_common::error::AppError;
use sqlx::PgPool;
use uuid::Uuid;
use super::*;
use super::crud::{load_owned, parse_config};


/// One directory entry returned by the `tree` navigation endpoint.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TreeEntry {
    pub name: String,
    pub writable: bool,
    pub child_count: usize,
    pub picture_count: Option<i64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TreeEntry>,
}

fn predicate_filter(pred: &TagPredicate) -> PictureListFilter {
    PictureListFilter {
        page: 1,
        page_size: 1,
        predicate: Some(pred.clone()),
        ..Default::default()
    }
}

async fn count_pred(db: &PgPool, user_id: Uuid, pred: &TagPredicate) -> Result<i64, AppError> {
    crate::repository::picture::PictureRepository::count(db, user_id, &predicate_filter(pred)).await
}

async fn dir_nonempty(db: &PgPool, user_id: Uuid, dir: &ResolvedDir) -> Result<bool, AppError> {
    match &dir.subtree {
        Some(p) => Ok(count_pred(db, user_id, p).await? > 0),
        None => {
            // static: visible iff any child has pictures.
            for c in &dir.children {
                if Box::pin(dir_nonempty(db, user_id, c)).await? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }
}

async fn build_entries(
    db: &PgPool,
    user_id: Uuid,
    dirs: &[ResolvedDir],
    depth: u32,
    counts: bool,
) -> Result<Vec<TreeEntry>, AppError> {
    let mut out = Vec::new();
    for dir in dirs {
        // Empty-directory hiding only when counts are computed (§5.2). Drop inboxes are always
        // shown even though they surface no pictures (feature 18 §4).
        if counts && !dir.always_visible && !dir_nonempty(db, user_id, dir).await? {
            continue;
        }
        let picture_count = if counts {
            Some(match &dir.direct {
                Some(p) => count_pred(db, user_id, p).await?,
                None => 0,
            })
        } else {
            None
        };
        let children = if depth > 1 {
            Box::pin(build_entries(db, user_id, &dir.children, depth - 1, counts)).await?
        } else {
            Vec::new()
        };
        out.push(TreeEntry {
            name: dir.name.clone(),
            writable: dir.writable,
            child_count: dir.children.len(),
            picture_count,
            children,
        });
    }
    Ok(out)
}

pub struct TreeResult {
    pub path: String,
    pub directories: Vec<TreeEntry>,
}

#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db), fields(user_id = %user_id, hierarchy_id = %hierarchy_id))]
pub async fn resolve_tree(
    db: &PgPool,
    user_id: Uuid,
    hierarchy_id: Uuid,
    path: &str,
    depth: u32,
    counts: bool,
) -> Result<TreeResult, AppError> {
    let row = load_owned(db, user_id, hierarchy_id).await?;
    let config = parse_config(&row.config)?;
    let root = resolve_for_user(db, user_id, &config).await?;

    let segments = split_path(path);
    let target = find_dir(&root, &segments).ok_or(AppError::NotFound)?;
    let depth = depth.max(1);
    let directories = build_entries(db, user_id, &target.children, depth, counts).await?;
    Ok(TreeResult {
        path: segments.join("/"),
        directories,
    })
}

