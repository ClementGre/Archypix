use crate::domain::hierarchy::{
    HierarchyConfig, TagPredicate,
};
use crate::repository::hierarchy::{HierarchyRepository, HierarchyRow};
use crate::repository::picture::PictureListFilter;
use crate::repository::tag::TagRepository;
use crate::repository::tag_metadata::TagMetadataRepository;
use archypix_common::error::AppError;
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;
use super::*;


pub(super) fn parse_config(value: &serde_json::Value) -> Result<HierarchyConfig, AppError> {
    let config: HierarchyConfig = serde_json::from_value(value.clone())
        .map_err(|e| AppError::BadRequest(format!("invalid hierarchy config: {e}")))?;
    config.validate().map_err(AppError::BadRequest)?;
    Ok(config)
}

pub(super) async fn load_owned(
    db: &PgPool,
    user_id: Uuid,
    hierarchy_id: Uuid,
) -> Result<HierarchyRow, AppError> {
    HierarchyRepository::get_by_owner_and_id(db, user_id, hierarchy_id)
        .await?
        .ok_or(AppError::NotFound)
}

#[tracing::instrument(skip(db), fields(user_id = %user_id))]
pub async fn list_hierarchies(db: &PgPool, user_id: Uuid) -> Result<Vec<HierarchyRow>, AppError> {
    HierarchyRepository::list_by_owner(db, user_id).await
}

/// Load an owned hierarchy, parse + validate its config, and resolve the directory tree
/// against the user's current tags. The single entry point the WebDAV `VirtualFs` uses.
#[tracing::instrument(skip(db), fields(user_id = %user_id, hierarchy_id = %hierarchy_id))]
pub async fn load_resolved(
    db: &PgPool,
    user_id: Uuid,
    hierarchy_id: Uuid,
) -> Result<(HierarchyRow, HierarchyConfig, ResolvedDir), AppError> {
    let row = load_owned(db, user_id, hierarchy_id).await?;
    let config = parse_config(&row.config)?;
    let root = resolve_for_user(db, user_id, &config).await?;
    Ok((row, config, root))
}

/// Resolve a config against the user's live tag set **plus** the feature-34 naming inputs: the
/// deliberately-empty tags that must still render as directories (§8.1) and the custom
/// `webdav_dir_name` overrides (§8). The one place both the WebDAV VFS and the webapp directory
/// tree go through, so they never disagree about a directory's name.
pub async fn resolve_for_user(
    db: &PgPool,
    user_id: Uuid,
    config: &HierarchyConfig,
) -> Result<ResolvedDir, AppError> {
    let mut distinct = TagRepository::list_paths_by_user(db, user_id).await?;
    let meta = TagMetadataRepository::list_for_user(db, user_id).await?;
    let mut custom: HashMap<String, String> = HashMap::new();
    for m in meta {
        if m.tag_path.is_empty() {
            continue; // the root view is not a directory
        }
        if m.show_when_empty && !distinct.contains(&m.tag_path) {
            distinct.push(m.tag_path.clone());
        }
        if let Some(name) = m.webdav_dir_name {
            custom.insert(m.tag_path, name);
        }
    }
    let mut root = resolve(config, &distinct);
    apply_custom_dir_names(&mut root, &custom);
    Ok(root)
}

/// Build a [`PictureListFilter`] that returns up to `page_size` pictures matching `pred`.
/// Used by the WebDAV VFS to list a directory's direct files with full picture rows.
pub fn list_filter_for(pred: &TagPredicate, page_size: i64) -> PictureListFilter {
    PictureListFilter {
        page: 1,
        page_size,
        predicate: Some(pred.clone()),
        ..Default::default()
    }
}

#[tracing::instrument(skip(db), fields(user_id = %user_id, hierarchy_id = %hierarchy_id))]
pub async fn get_hierarchy(
    db: &PgPool,
    user_id: Uuid,
    hierarchy_id: Uuid,
) -> Result<HierarchyRow, AppError> {
    load_owned(db, user_id, hierarchy_id).await
}

#[tracing::instrument(skip(db, config_value), fields(user_id = %user_id))]
pub async fn create_hierarchy(
    db: &PgPool,
    user_id: Uuid,
    name: &str,
    config_value: &serde_json::Value,
) -> Result<HierarchyRow, AppError> {
    if name.trim().is_empty() {
        return Err(AppError::BadRequest("name must not be empty".to_string()));
    }
    let config = parse_config(config_value)?;
    // Store the normalized config (defaults filled, fields ordered) so reads are canonical.
    let normalized =
        serde_json::to_value(&config).map_err(|e| AppError::InternalServerError(e.to_string()))?;
    HierarchyRepository::create(db, user_id, name.trim(), &normalized).await
}

#[tracing::instrument(skip(db, config_value), fields(user_id = %user_id, hierarchy_id = %hierarchy_id))]
pub async fn update_hierarchy(
    db: &PgPool,
    user_id: Uuid,
    hierarchy_id: Uuid,
    name: Option<&str>,
    enabled: Option<bool>,
    config_value: Option<&serde_json::Value>,
) -> Result<HierarchyRow, AppError> {
    if let Some(n) = name {
        if n.trim().is_empty() {
            return Err(AppError::BadRequest("name must not be empty".to_string()));
        }
    }
    let normalized = match config_value {
        Some(v) => {
            let config = parse_config(v)?;
            Some(
                serde_json::to_value(&config)
                    .map_err(|e| AppError::InternalServerError(e.to_string()))?,
            )
        }
        None => None,
    };
    HierarchyRepository::update(
        db,
        user_id,
        hierarchy_id,
        name.map(str::trim),
        enabled,
        normalized.as_ref(),
    )
    .await?
    .ok_or(AppError::NotFound)
}

#[tracing::instrument(skip(db), fields(user_id = %user_id, hierarchy_id = %hierarchy_id))]
pub async fn delete_hierarchy(
    db: &PgPool,
    user_id: Uuid,
    hierarchy_id: Uuid,
) -> Result<bool, AppError> {
    HierarchyRepository::delete(db, user_id, hierarchy_id).await
}

