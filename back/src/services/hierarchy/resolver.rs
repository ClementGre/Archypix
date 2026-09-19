use crate::domain::hierarchy::{
    DeeperMode, HierarchyConfig, MatchMode, NamingStrategy, NodeKind, SafeDeleteMode, TagOp,
    TagOpKind, TagPredicate, WriteBack,
};
use crate::domain::tag::TagPath;
use std::collections::{HashMap, HashSet};


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
    /// Direct-files predicate (`P(D) ∧ ⋀¬own(childᵢ)`). `None` for `static`/`drop` (no direct
    /// files).
    pub direct: Option<TagPredicate>,
    /// Subtree predicate, for counts / empty-dir hiding. `None` for `static` (recurse children)
    /// and `drop` (always shown, see [`ResolvedDir::always_visible`]).
    pub subtree: Option<TagPredicate>,
    /// Exempt from empty-directory hiding (feature 18 §4) — a `drop` inbox is always listed even
    /// though it surfaces no pictures.
    pub always_visible: bool,
    /// The membership term the parent subtracts as `own(child)`. `None` for `static`.
    own_for_parent: Option<TagPredicate>,
    /// Effective write-back op-list for this directory (06_webdav.md §7). `None` ⇒ read-only.
    /// `mirror` dirs synthesize assign/remove of their own tag; writable `query` dirs carry the
    /// authored op-list. Consumed by the WebDAV write layer.
    pub write_back: Option<WriteBack>,
    /// For `mirror` directories: the ltree tag path this directory maps to (its own tag). `None`
    /// for `query`/`static`/root. The WebDAV write layer uses this to extend the mirror with a
    /// brand-new sub-path — appending the new segments as deeper tag labels (06_webdav.md §9).
    pub mirror_tag: Option<String>,
    /// For a **container** directory (root/`static`/`query`) that hoists a `keepDir=false`
    /// `mirror` child's expansion into its own level: the mirror's `(tagRoot, writable)`, so a
    /// brand-new child directory created here maps to that mirror (feature 18 §11 — the first
    /// hoisted mirror wins). `None` when the container has no hoisted mirror.
    pub new_child_mirror: Option<(String, bool)>,
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
        true, // root seed for the tri-state write-back inheritance (feature 18 §5.1)
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
        always_visible: false,
        own_for_parent: None,
        write_back: None,
        mirror_tag: None,
        new_child_mirror: first_hoisted_mirror(&config.nodes, config.write_back, true),
        children: roots,
    }
}

/// The first `keepDir=false` `mirror` among `nodes` (its expansion is hoisted into the container's
/// level, so a brand-new child directory of the container maps to it) with its effective
/// writability — feature 18 §11. `master`/`inherited` resolve the mirror's `writeBackEnabled`.
fn first_hoisted_mirror(
    nodes: &[crate::domain::hierarchy::Node],
    master: bool,
    inherited: bool,
) -> Option<(String, bool)> {
    nodes.iter().find_map(|node| match &node.kind {
        NodeKind::Mirror {
            tag_root,
            keep_dir: false,
            ..
        } => {
            let writable = master && node.write_back_enabled.unwrap_or(inherited);
            Some((tag_root.clone(), writable))
        }
        _ => None,
    })
}

/// Recursively build directories for `nodes`. `master` is the hierarchy write-back switch (hard
/// ceiling); `inherited_enabled` is the effective write-back of the parent chain (feature 18
/// §5.1) — the nearest explicit ancestor `writeBackEnabled`, seeded `true` at the root.
#[allow(clippy::too_many_arguments)]
fn build_nodes(
    nodes: &[crate::domain::hierarchy::Node],
    distinct_paths: &[String],
    and_terms: &[TagPredicate],
    master: bool,
    inherited_enabled: bool,
    def_sdm: SafeDeleteMode,
    def_naming: NamingStrategy,
) -> Vec<ResolvedDir> {
    let mut out = Vec::new();
    for node in nodes {
        let sdm = node.safe_delete_mode.unwrap_or(def_sdm);
        let naming = node.naming.unwrap_or(def_naming);
        // Effective write-back for this node + the value its subtree inherits.
        let node_enabled = if master {
            node.write_back_enabled.unwrap_or(inherited_enabled)
        } else {
            false
        };
        match &node.kind {
            NodeKind::Mirror { .. } => out.extend(expand_mirror(
                node,
                distinct_paths,
                and_terms,
                node_enabled,
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
                    master,
                    node_enabled,
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
                // Untagged nodes may now be writable (feature 18 §6) — free-form op-list.
                let writable = node_enabled && write_back.is_some();
                out.push(ResolvedDir {
                    name: node.effective_name().unwrap_or_default(),
                    writable,
                    safe_delete_mode: sdm,
                    naming,
                    direct: Some(direct),
                    subtree: Some(membership),
                    always_visible: false,
                    own_for_parent: Some(own_base),
                    write_back: if writable { write_back.clone() } else { None },
                    mirror_tag: None,
                    new_child_mirror: first_hoisted_mirror(children, master, node_enabled),
                    children: child_dirs,
                });
            }
            NodeKind::Static { children } => {
                // A static node is never writable itself, but its toggle sets the inherited
                // default for descendants (feature 18 §5).
                let child_dirs = build_nodes(
                    children,
                    distinct_paths,
                    and_terms,
                    master,
                    node_enabled,
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
                    always_visible: false,
                    own_for_parent: None,
                    write_back: None,
                    mirror_tag: None,
                    new_child_mirror: first_hoisted_mirror(children, master, node_enabled),
                    children: child_dirs,
                });
            }
            NodeKind::Drop { on_add } => {
                // Write-only inbox (feature 18 §4): always shown, lists nothing, always writable
                // (ignores master + writeBackEnabled), applies the fixed on_add op-list.
                out.push(ResolvedDir {
                    name: node.effective_name().unwrap_or_default(),
                    writable: true,
                    safe_delete_mode: sdm,
                    naming,
                    direct: None,
                    subtree: None,
                    always_visible: true,
                    own_for_parent: None,
                    write_back: Some(WriteBack {
                        on_add: on_add.clone(),
                        on_remove: vec![],
                    }),
                    mirror_tag: None,
                    new_child_mirror: None,
                    children: vec![],
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
    writable: bool,
    sdm: SafeDeleteMode,
    naming: NamingStrategy,
) -> Vec<ResolvedDir> {
    let (tag_root, keep_dir, collapsed, exclude, max_depth, deeper_mode) = match &node.kind {
        NodeKind::Mirror {
            tag_root,
            keep_dir,
            collapsed,
            exclude,
            max_depth,
            deeper_mode,
        } => (
            tag_root.clone(),
            *keep_dir,
            collapsed.clone(),
            exclude.clone(),
            *max_depth,
            *deeper_mode,
        ),
        _ => return vec![],
    };
    let root_depth = depth_of(&tag_root);

    // Split excludes (feature 18 §7.3): sub-tag excludes (`<@ tagRoot`) prune directories AND
    // pictures; foreign excludes are a pure picture-membership cut applied to every directory.
    let (mut sub_excludes, foreign_excludes): (Vec<String>, Vec<String>) = exclude
        .into_iter()
        .partition(|er| under_or_eq(&tag_root, er));
    let mut collapsed = collapsed;

    // maxDepth (§7.1–7.2): every tag path deeper than the cut folds at its level-(maxDepth+1)
    // ancestor, which we inject as a synthetic collapsed (roll-up) or excluded (drop) root —
    // reusing the existing machinery so directory generation naturally stops at the cut.
    if max_depth >= 1 {
        let cut_depth = root_depth + max_depth as usize;
        let cut_roots: HashSet<String> = distinct_paths
            .iter()
            .filter(|p| {
                under_or_eq(&tag_root, p)
                    && !sub_excludes.iter().any(|er| under_or_eq(er, p))
                    && depth_of(p) > cut_depth
            })
            .map(|p| {
                p.split('.')
                    .take(cut_depth + 1)
                    .collect::<Vec<_>>()
                    .join(".")
            })
            .collect();
        match deeper_mode {
            DeeperMode::Collapse => collapsed.extend(cut_roots),
            DeeperMode::Exclude => sub_excludes.extend(cut_roots),
        }
    }

    // Paths under tagRoot (inclusive), minus sub-tag-excluded subtrees.
    let relevant: Vec<&String> = distinct_paths
        .iter()
        .filter(|p| under_or_eq(&tag_root, p) && !sub_excludes.iter().any(|er| under_or_eq(er, p)))
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
        if !under_or_eq(&tag_root, cr) || sub_excludes.iter().any(|er| under_or_eq(er, cr)) {
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
        sub_excludes,
        foreign_excludes,
        and_terms: and_terms.to_vec(),
        writable,
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
    /// Sub-tag excludes (`<@ tagRoot`) — prune the subtree; applied to the `subtree` predicate.
    sub_excludes: Vec<String>,
    /// Foreign excludes (not under tagRoot) — a picture-membership cut on every directory.
    foreign_excludes: Vec<String>,
    and_terms: Vec<TagPredicate>,
    writable: bool,
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

    // Membership-cut excludes for this directory:
    //   - foreign excludes (§7.3): reject any picture carrying one, on every mirror directory;
    //   - sub-tag excludes that fall *within this directory's subtree*: otherwise a picture that
    //     independently carries the exact directory tag (e.g. a `rule`/`segment` `Photos` row) would
    //     leak into the ancestor directory even though it also carries the excluded `Photos.Test`.
    //     Sibling-branch excludes are not added here (a picture keeps showing under its other branch).
    let mut cut: Vec<TagPath> = ctx
        .foreign_excludes
        .iter()
        .map(|s| TagPath::from_ltree(s.clone()))
        .collect();
    for e in &ctx.sub_excludes {
        if under_or_eq(path, e) {
            cut.push(TagPath::from_ltree(e.clone()));
        }
    }

    // Membership for direct files: exact T plus any collapsed subtrees rolled into this dir.
    let mut include: Vec<TagPath> = Vec::new();
    if let Some(arms) = ctx.collapsed_arms.get(path) {
        include.extend(arms.iter().map(|s| TagPath::from_ltree(s.clone())));
    }
    let own = TagPredicate {
        exact: vec![TagPath::from_ltree(path.to_string())],
        include,
        match_all: false, // exact T OR collapsed arms
        exclude: cut,
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
    // Subtree: everything under T (inclusive), minus excluded subtrees (sub-tag + foreign).
    let mut subtree_exclude: Vec<TagPath> = ctx
        .sub_excludes
        .iter()
        .map(|s| TagPath::from_ltree(s.clone()))
        .collect();
    subtree_exclude.extend(
        ctx.foreign_excludes
            .iter()
            .map(|s| TagPath::from_ltree(s.clone())),
    );
    let subtree = TagPredicate {
        include: vec![TagPath::from_ltree(path.to_string())],
        match_all: true,
        exclude: subtree_exclude,
        and_terms: ctx.and_terms.clone(),
        ..TagPredicate::all()
    };
    let label = path.rsplit('.').next().unwrap_or(path).to_string();
    // Mirror write-back is implicit: assign/remove the directory's own tag (§7.1).
    let write_back = if ctx.writable {
        Some(WriteBack {
            on_add: vec![TagOp {
                op: TagOpKind::Assign,
                path: path.to_string(),
            }],
            on_remove: vec![TagOp {
                op: TagOpKind::Remove,
                path: path.to_string(),
            }],
        })
    } else {
        None
    };
    ResolvedDir {
        name: name_override.unwrap_or(label),
        writable: ctx.writable,
        safe_delete_mode: ctx.sdm,
        naming: ctx.naming,
        direct: Some(direct),
        subtree: Some(subtree),
        always_visible: false,
        // Parent subtracts membership under this dir's tag (inclusive).
        own_for_parent: Some(TagPredicate {
            include: vec![TagPath::from_ltree(path.to_string())],
            match_all: true,
            ..TagPredicate::all()
        }),
        write_back,
        mirror_tag: Some(path.to_string()),
        new_child_mirror: None,
        children,
    }
}

/// Apply custom WebDAV directory names over a resolved tree (feature 34 §8).
///
/// Resolution is **custom `webdav_dir_name` → ltree label**, never `display_name`: a mounted client
/// sees a directory rename as delete + create, so the folder name must not churn when a label is
/// tidied. Only a directory still named after its bare ltree label is renamed — an authored
/// `static`/`query`/`mirror` node name always wins. Every label in a sibling set is reserved before
/// any custom name is applied, and matching is case-insensitive (06_webdav.md §10c), so a
/// case-folding client can never end up with two directories it cannot tell apart.
pub fn apply_custom_dir_names(dir: &mut ResolvedDir, custom: &HashMap<String, String>) {
    if !custom.is_empty() {
        let mut taken: HashSet<String> = dir
            .children
            .iter()
            .map(|c| c.name.to_lowercase())
            .collect();
        // Path order, so a contested name resolves the same way on every host.
        let mut order: Vec<usize> = (0..dir.children.len()).collect();
        order.sort_by_key(|&i| dir.children[i].mirror_tag.clone().unwrap_or_default());
        for i in order {
            let child = &mut dir.children[i];
            let Some(tag) = child.mirror_tag.clone() else {
                continue;
            };
            if child.name != leaf_label(&tag) {
                continue; // authored name — it wins over a custom one
            }
            let Some(name) = custom.get(&tag) else { continue };
            if taken.insert(name.to_lowercase()) {
                child.name = name.clone();
            }
        }
    }
    for child in &mut dir.children {
        apply_custom_dir_names(child, custom);
    }
}

fn leaf_label(ltree: &str) -> String {
    ltree.rsplit('.').next().unwrap_or(ltree).to_string()
}

/// Navigate from `root` to the directory addressed by `segments` (directory names).
pub fn find_dir<'a>(root: &'a ResolvedDir, segments: &[String]) -> Option<&'a ResolvedDir> {
    let mut cur = root;
    for seg in segments {
        cur = cur.children.iter().find(|c| &c.name == seg)?;
    }
    Some(cur)
}

pub fn split_path(path: &str) -> Vec<String> {
    path.split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

