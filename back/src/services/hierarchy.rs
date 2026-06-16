//! Hierarchy CRUD orchestration and the read resolver.
//!
//! The resolver ([`resolve`]) turns a `HierarchyConfig` plus the user's distinct tag paths into a
//! [`ResolvedDir`] tree (the synthetic root and its descendants). Each directory carries a
//! [`TagPredicate`] for its direct files (`browse`) and one for its subtree (counts / empty-dir
//! hiding). It is the single source of truth for both the webapp navigation and (later) WebDAV.
//!
//! See `doc/features/05_hierarchies.md` §5–6.

use crate::clients::federation::FederationClient;
use crate::domain::hierarchy::{
    HierarchyConfig, MatchMode, NamingStrategy, NodeKind, SafeDeleteMode, TagPredicate,
};
use crate::domain::tag::TagPath;
use crate::infra::config::Config;
use crate::infra::error::AppError;
use crate::infra::redis::Cache;
use crate::infra::s3::Storage;
use crate::repository::hierarchy::{HierarchyRepository, HierarchyRow};
use crate::repository::picture::{PictureListFilter, PictureSortField, SortOrder};
use crate::repository::tag::TagRepository;
use crate::services::pictures::{PictureListResult, ThumbnailSize};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

// ─── Resolved directory tree ────────────────────────────────────────────────────

/// A resolved directory: a pure function of the config and the tag set. Picture membership is
/// derived live via the [`TagPredicate`]s.
#[derive(Debug, Clone)]
pub struct ResolvedDir {
    pub name: String,
    pub writable: bool,
    // Carried for the write-ready model; consumed by the WebDAV write layer (§13, future).
    #[allow(dead_code)]
    pub safe_delete_mode: SafeDeleteMode,
    #[allow(dead_code)]
    pub naming: NamingStrategy,
    /// Direct-files predicate (`P(D) ∧ ⋀¬own(childᵢ)`). `None` for `static` (no direct files).
    pub direct: Option<TagPredicate>,
    /// Subtree predicate, for counts / empty-dir hiding. `None` for `static` (recurse children).
    pub subtree: Option<TagPredicate>,
    /// The membership term the parent subtracts as `own(child)`. `None` for `static`.
    own_for_parent: Option<TagPredicate>,
    pub children: Vec<ResolvedDir>,
}

fn depth_of(ltree: &str) -> usize {
    ltree.split('.').count()
}

/// `a` is an ancestor of `b` or equal to it.
fn under_or_eq(a: &str, b: &str) -> bool {
    a == b || TagPath::from_ltree(a).is_ancestor_of(&TagPath::from_ltree(b))
}

/// Build the synthetic root directory for `config` against the user's `distinct_paths`.
pub fn resolve(config: &HierarchyConfig, distinct_paths: &[String]) -> ResolvedDir {
    let roots = build_nodes(
        &config.nodes,
        distinct_paths,
        &[],
        config.write_back,
        config.safe_delete_mode,
        config.naming,
    );
    // The synthetic root is a pure container: it is not an authored node and carries no
    // predicate of its own, so it surfaces no direct files. Pictures appear only in the
    // configured directories — browsing "" returns an empty page, like a `static` node. (A
    // vacuously-true root predicate would otherwise dump every uncovered picture — all of them,
    // for an empty hierarchy — into the root listing.)
    ResolvedDir {
        name: String::new(),
        writable: false,
        safe_delete_mode: config.safe_delete_mode,
        naming: config.naming,
        direct: None,
        subtree: None,
        own_for_parent: None,
        children: roots,
    }
}

fn build_nodes(
    nodes: &[crate::domain::hierarchy::Node],
    distinct_paths: &[String],
    and_terms: &[TagPredicate],
    cfg_write_back: bool,
    def_sdm: SafeDeleteMode,
    def_naming: NamingStrategy,
) -> Vec<ResolvedDir> {
    let mut out = Vec::new();
    for node in nodes {
        let sdm = node.safe_delete_mode.unwrap_or(def_sdm);
        let naming = node.naming.unwrap_or(def_naming);
        match &node.kind {
            NodeKind::Mirror { .. } => out.extend(expand_mirror(
                node,
                distinct_paths,
                and_terms,
                cfg_write_back,
                sdm,
                naming,
            )),
            NodeKind::Query {
                match_mode,
                include,
                exclude,
                match_untagged,
                write_back,
                children,
            } => {
                let own_base = TagPredicate {
                    include: include
                        .iter()
                        .map(|s| TagPath::from_ltree(s.clone()))
                        .collect(),
                    match_all: matches!(match_mode, MatchMode::All),
                    exclude: exclude
                        .iter()
                        .map(|s| TagPath::from_ltree(s.clone()))
                        .collect(),
                    untagged: *match_untagged,
                    ..TagPredicate::all()
                };
                // Children inherit ancestors + this node's own term.
                let mut child_and = and_terms.to_vec();
                child_and.push(own_base.clone());
                let child_dirs = build_nodes(
                    children,
                    distinct_paths,
                    &child_and,
                    cfg_write_back,
                    def_sdm,
                    def_naming,
                );
                let membership = TagPredicate {
                    and_terms: and_terms.to_vec(),
                    ..own_base.clone()
                };
                let direct = TagPredicate {
                    minus_children: child_dirs
                        .iter()
                        .filter_map(|d| d.own_for_parent.clone())
                        .collect(),
                    ..membership.clone()
                };
                out.push(ResolvedDir {
                    name: node.effective_name().unwrap_or_default(),
                    writable: cfg_write_back && write_back.is_some() && !*match_untagged,
                    safe_delete_mode: sdm,
                    naming,
                    direct: Some(direct),
                    subtree: Some(membership),
                    own_for_parent: Some(own_base),
                    children: child_dirs,
                });
            }
            NodeKind::Static { children } => {
                let child_dirs = build_nodes(
                    children,
                    distinct_paths,
                    and_terms,
                    cfg_write_back,
                    def_sdm,
                    def_naming,
                );
                out.push(ResolvedDir {
                    name: node.effective_name().unwrap_or_default(),
                    writable: false,
                    safe_delete_mode: sdm,
                    naming,
                    direct: None,
                    subtree: None,
                    own_for_parent: None,
                    children: child_dirs,
                });
            }
        }
    }
    out
}

/// Expand a `mirror` node into its directory subtree from the distinct tag paths.
fn expand_mirror(
    node: &crate::domain::hierarchy::Node,
    distinct_paths: &[String],
    and_terms: &[TagPredicate],
    cfg_write_back: bool,
    sdm: SafeDeleteMode,
    naming: NamingStrategy,
) -> Vec<ResolvedDir> {
    let (tag_root, keep_dir, collapsed, exclude) = match &node.kind {
        NodeKind::Mirror {
            tag_root,
            keep_dir,
            collapsed,
            exclude,
        } => (
            tag_root.clone(),
            *keep_dir,
            collapsed.clone(),
            exclude.clone(),
        ),
        _ => return vec![],
    };
    let root_depth = depth_of(&tag_root);

    // Paths under tagRoot (inclusive), minus excluded subtrees.
    let relevant: Vec<&String> = distinct_paths
        .iter()
        .filter(|p| under_or_eq(&tag_root, p) && !exclude.iter().any(|er| under_or_eq(er, p)))
        .collect();

    // Directory paths: every prefix at or below tagRoot depth, not inside a collapsed subtree.
    let mut dir_paths: HashSet<String> = HashSet::new();
    for p in &relevant {
        let labels: Vec<&str> = p.split('.').collect();
        for i in root_depth..=labels.len() {
            let pre = labels[..i].join(".");
            if collapsed.iter().any(|cr| under_or_eq(cr, &pre)) {
                continue;
            }
            dir_paths.insert(pre);
        }
    }

    // Collapsed roll-up arms: each collapsed root's pictures bubble to its nearest enabled ancestor.
    let mut collapsed_arms: HashMap<String, Vec<String>> = HashMap::new();
    for cr in &collapsed {
        if !under_or_eq(&tag_root, cr) || exclude.iter().any(|er| under_or_eq(er, cr)) {
            continue;
        }
        if !relevant.iter().any(|p| under_or_eq(cr, p)) {
            continue; // no pictures under this collapsed subtree
        }
        // Deepest ancestor of `cr` present in dir_paths.
        let anc = TagPath::from_ltree(cr.clone())
            .ancestors()
            .into_iter()
            .rev()
            .map(|a| a.as_ltree().to_string())
            .find(|a| dir_paths.contains(a));
        if let Some(a) = anc {
            collapsed_arms.entry(a).or_default().push(cr.clone());
        }
    }

    let ctx = MirrorCtx {
        dir_paths,
        collapsed_arms,
        exclude_roots: exclude,
        and_terms: and_terms.to_vec(),
        cfg_write_back,
        sdm,
        naming,
    };

    if keep_dir && ctx.dir_paths.contains(&tag_root) {
        let name = node.effective_name().unwrap_or_default();
        vec![build_mirror_dir(&tag_root, Some(name), &ctx)]
    } else {
        // keepDir = false (or tagRoot collapsed): the root label is stripped — its children sit
        // at the node's level.
        immediate_children(&tag_root, &ctx.dir_paths)
            .into_iter()
            .map(|c| build_mirror_dir(&c, None, &ctx))
            .collect()
    }
}

struct MirrorCtx {
    dir_paths: HashSet<String>,
    collapsed_arms: HashMap<String, Vec<String>>,
    exclude_roots: Vec<String>,
    and_terms: Vec<TagPredicate>,
    cfg_write_back: bool,
    sdm: SafeDeleteMode,
    naming: NamingStrategy,
}

/// Immediate child directory paths of `path` present in `dir_paths` (one label deeper).
fn immediate_children(path: &str, dir_paths: &HashSet<String>) -> Vec<String> {
    let want_depth = depth_of(path) + 1;
    let mut kids: Vec<String> = dir_paths
        .iter()
        .filter(|c| depth_of(c) == want_depth && under_or_eq(path, c))
        .cloned()
        .collect();
    kids.sort();
    kids
}

fn build_mirror_dir(path: &str, name_override: Option<String>, ctx: &MirrorCtx) -> ResolvedDir {
    let children: Vec<ResolvedDir> = immediate_children(path, &ctx.dir_paths)
        .into_iter()
        .map(|c| build_mirror_dir(&c, None, ctx))
        .collect();

    // Membership for direct files: exact T plus any collapsed subtrees rolled into this dir.
    let mut include: Vec<TagPath> = Vec::new();
    if let Some(arms) = ctx.collapsed_arms.get(path) {
        include.extend(arms.iter().map(|s| TagPath::from_ltree(s.clone())));
    }
    let own = TagPredicate {
        exact: vec![TagPath::from_ltree(path.to_string())],
        include,
        match_all: false, // exact T OR collapsed arms
        and_terms: ctx.and_terms.clone(),
        ..TagPredicate::all()
    };
    let direct = TagPredicate {
        minus_children: children
            .iter()
            .filter_map(|d| d.own_for_parent.clone())
            .collect(),
        ..own.clone()
    };
    // Subtree: everything under T (inclusive), minus excluded subtrees.
    let subtree = TagPredicate {
        include: vec![TagPath::from_ltree(path.to_string())],
        match_all: true,
        exclude: ctx
            .exclude_roots
            .iter()
            .map(|s| TagPath::from_ltree(s.clone()))
            .collect(),
        and_terms: ctx.and_terms.clone(),
        ..TagPredicate::all()
    };
    let label = path.rsplit('.').next().unwrap_or(path).to_string();
    ResolvedDir {
        name: name_override.unwrap_or(label),
        writable: ctx.cfg_write_back,
        safe_delete_mode: ctx.sdm,
        naming: ctx.naming,
        direct: Some(direct),
        subtree: Some(subtree),
        // Parent subtracts membership under this dir's tag (inclusive).
        own_for_parent: Some(TagPredicate {
            include: vec![TagPath::from_ltree(path.to_string())],
            match_all: true,
            ..TagPredicate::all()
        }),
        children,
    }
}

/// Navigate from `root` to the directory addressed by `segments` (directory names).
pub fn find_dir<'a>(root: &'a ResolvedDir, segments: &[String]) -> Option<&'a ResolvedDir> {
    let mut cur = root;
    for seg in segments {
        cur = cur.children.iter().find(|c| &c.name == seg)?;
    }
    Some(cur)
}

fn split_path(path: &str) -> Vec<String> {
    path.split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

// ─── Tree endpoint ──────────────────────────────────────────────────────────────

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
        sort: PictureSortField::default(),
        order: SortOrder::default(),
        tag: None,
        predicate: Some(pred.clone()),
        owned_only: false,
        shared_with_me: false,
        include_deleted: false,
        captured_after: None,
        captured_before: None,
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
        // Empty-directory hiding only when counts are computed (§5.2).
        if counts && !dir_nonempty(db, user_id, dir).await? {
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
    let distinct = TagRepository::list_paths_by_user(db, user_id).await?;
    let root = resolve(&config, &distinct);

    let segments = split_path(path);
    let target = find_dir(&root, &segments).ok_or(AppError::NotFound)?;
    let depth = depth.max(1);
    let directories = build_entries(db, user_id, &target.children, depth, counts).await?;
    Ok(TreeResult {
        path: segments.join("/"),
        directories,
    })
}

// ─── Browse endpoint ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BrowseParams {
    pub page: u32,
    pub page_size: u32,
    pub sort: PictureSortField,
    pub order: SortOrder,
    pub include_deleted: bool,
    pub owned_only: bool,
    pub shared_with_me: bool,
    pub captured_after: Option<DateTime<Utc>>,
    pub captured_before: Option<DateTime<Utc>>,
    pub thumbnail: Option<ThumbnailSize>,
}

#[allow(clippy::too_many_arguments)]
pub async fn browse(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    config: &Config,
    federation: &FederationClient,
    user_id: Uuid,
    hierarchy_id: Uuid,
    path: &str,
    params: BrowseParams,
) -> Result<PictureListResult, AppError> {
    if params.page_size > 200 {
        return Err(AppError::BadRequest(
            "page_size cannot exceed 200".to_string(),
        ));
    }
    let row = load_owned(db, user_id, hierarchy_id).await?;
    let hierarchy_config = parse_config(&row.config)?;
    let distinct = TagRepository::list_paths_by_user(db, user_id).await?;
    let root = resolve(&hierarchy_config, &distinct);

    let segments = split_path(path);
    let target = find_dir(&root, &segments).ok_or(AppError::NotFound)?;

    // static directories (and any None-direct node) have no direct files.
    let Some(predicate) = target.direct.clone() else {
        return Ok(PictureListResult {
            total: 0,
            page: params.page,
            page_size: params.page_size,
            items: vec![],
        });
    };

    let filter = PictureListFilter {
        page: params.page as i64,
        page_size: params.page_size as i64,
        sort: params.sort,
        order: params.order,
        tag: None,
        predicate: Some(predicate),
        owned_only: params.owned_only,
        shared_with_me: params.shared_with_me,
        include_deleted: params.include_deleted,
        captured_after: params.captured_after.map(|dt| dt.naive_utc()),
        captured_before: params.captured_before.map(|dt| dt.naive_utc()),
    };

    crate::services::pictures::list_with_filter(
        db,
        cache,
        storage,
        config,
        federation,
        user_id,
        filter,
        params.thumbnail,
    )
    .await
}

// ─── CRUD ───────────────────────────────────────────────────────────────────────

fn parse_config(value: &serde_json::Value) -> Result<HierarchyConfig, AppError> {
    let config: HierarchyConfig = serde_json::from_value(value.clone())
        .map_err(|e| AppError::BadRequest(format!("invalid hierarchy config: {e}")))?;
    config.validate().map_err(AppError::BadRequest)?;
    Ok(config)
}

async fn load_owned(
    db: &PgPool,
    user_id: Uuid,
    hierarchy_id: Uuid,
) -> Result<HierarchyRow, AppError> {
    HierarchyRepository::get_by_owner_and_id(db, user_id, hierarchy_id)
        .await?
        .ok_or(AppError::NotFound)
}

pub async fn list_hierarchies(db: &PgPool, user_id: Uuid) -> Result<Vec<HierarchyRow>, AppError> {
    HierarchyRepository::list_by_owner(db, user_id).await
}

pub async fn get_hierarchy(
    db: &PgPool,
    user_id: Uuid,
    hierarchy_id: Uuid,
) -> Result<HierarchyRow, AppError> {
    load_owned(db, user_id, hierarchy_id).await
}

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

pub async fn delete_hierarchy(
    db: &PgPool,
    user_id: Uuid,
    hierarchy_id: Uuid,
) -> Result<bool, AppError> {
    HierarchyRepository::delete(db, user_id, hierarchy_id).await
}
