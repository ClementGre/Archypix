use crate::domain::hierarchy::{
    DeeperMode, HierarchyConfig, MatchMode, NamingStrategy, Node, NodeKind, SafeDeleteMode, TagOp,
    TagOpKind, TagPredicate, TrashedDuplicate, WriteBack,
};
use crate::domain::tag::TagPath;
use std::collections::{HashMap, HashSet};

/// A resolved directory: a pure function of the config and the tag set. Picture membership is
/// derived live via the [`TagPredicate`]s.
#[derive(Debug, Clone)]
pub struct ResolvedDir {
    pub name: String,
    pub writable: bool,
    pub safe_delete_mode: SafeDeleteMode,
    // Carried for the write-ready model; consumed by the WebDAV write layer (§13, future).
    #[allow(dead_code)]
    pub naming: NamingStrategy,
    /// Direct-files predicate (`P(D) ∧ ⋀¬own(childᵢ)`). `None` for `static`/`drop` and the
    /// `tagRoot` container of a projected mirror (no direct files).
    pub direct: Option<TagPredicate>,
    /// Subtree predicate, for counts / empty-dir hiding. `None` for containers (recurse children)
    /// and `drop` (always shown).
    pub subtree: Option<TagPredicate>,
    /// A `drop` inbox (feature 18 §4): exempt from empty-directory hiding, lists only its recent
    /// uploads (feature 39 §7).
    pub is_drop: bool,
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
    /// For `mirror` directories: the mirror's foreign `include` arms (feature 39 §5.1).
    mirror_include: Vec<String>,
    /// For a **container** directory (root/`static`/`query`) hoisting a `keepDir=false` mirror:
    /// the mirror a brand-new child directory maps to (feature 18 §11, first hoisted wins).
    new_child_mirror: Option<MirrorBase>,
    /// The authored node this directory renders; `None` for the synthetic root.
    pub node_id: Option<String>,
    /// Resolved `trashedDuplicate` policy (feature 39 §6).
    pub trashed_duplicate: TrashedDuplicate,
    pub children: Vec<ResolvedDir>,
}

/// A mirror tag a new directory path can extend (06_webdav §9), with its node's write settings.
#[derive(Debug, Clone)]
pub struct MirrorBase {
    pub tag: String,
    pub node_id: String,
    /// Foreign include arms, assigned alongside the directory's own tag (feature 39 §5.1).
    pub include: Vec<String>,
    pub writable: bool,
    pub trashed_duplicate: TrashedDuplicate,
}

impl MirrorBase {
    /// The implicit op-list of the mirror directory mapping to `tag` (05 §7.1, 39 §5.1).
    pub fn write_back(&self, tag: &str) -> WriteBack {
        let assign = |path: &str| TagOp {
            op: TagOpKind::Assign,
            path: path.to_string(),
        };
        let mut on_add = vec![assign(tag)];
        on_add.extend(self.include.iter().map(|i| assign(i)));
        WriteBack {
            on_add,
            on_remove: vec![TagOp {
                op: TagOpKind::Remove,
                path: tag.to_string(),
            }],
        }
    }

    /// This mirror's step in a node chain for the directory mapping to `tag`.
    pub fn chain_node(&self, tag: &str) -> ChainNode {
        ChainNode {
            key: chain_key(&self.node_id, Some(tag)),
            write_back: self.writable.then(|| self.write_back(tag)),
        }
    }
}

/// Each mirror directory is its own chain step; other kinds are one step per node.
fn chain_key(node_id: &str, mirror_tag: Option<&str>) -> String {
    match mirror_tag {
        Some(tag) => format!("{node_id}:{tag}"),
        None => node_id.to_string(),
    }
}

/// One step of a write-back node chain (feature 39 §3.1).
#[derive(Debug, Clone)]
pub struct ChainNode {
    pub key: String,
    pub write_back: Option<WriteBack>,
}

impl ResolvedDir {
    fn chain_node(&self) -> Option<ChainNode> {
        Some(ChainNode {
            key: chain_key(self.node_id.as_deref()?, self.mirror_tag.as_deref()),
            write_back: self.write_back.clone(),
        })
    }

    /// The mirror a brand-new child path of this directory extends: its own tag for a mirror
    /// directory, the hoisted mirror's root for a container.
    pub fn extension_base(&self) -> Option<MirrorBase> {
        let Some(tag) = &self.mirror_tag else {
            return self.new_child_mirror.clone();
        };
        Some(MirrorBase {
            tag: tag.clone(),
            node_id: self.node_id.clone()?,
            include: self.mirror_include.clone(),
            writable: self.writable,
            trashed_duplicate: self.trashed_duplicate,
        })
    }
}

/// The node chain (ancestors then self) of the directory at `segments`. Intermediate levels of
/// one mirror expansion are not nodes of their own — only the deepest one counts.
pub fn node_chain(root: &ResolvedDir, segments: &[String]) -> Option<Vec<ChainNode>> {
    let mut dirs: Vec<&ResolvedDir> = Vec::with_capacity(segments.len());
    let mut cur = root;
    for seg in segments {
        cur = cur.children.iter().find(|c| &c.name == seg)?;
        dirs.push(cur);
    }
    Some(
        dirs.iter()
            .enumerate()
            .filter(|(i, d)| dirs.get(i + 1).is_none_or(|next| next.node_id != d.node_id))
            .filter_map(|(_, d)| d.chain_node())
            .collect(),
    )
}

/// The difference rule (feature 39 §3.1): `onRemove` of every node being left (`src \ dst`), then
/// `onAdd` of every node being entered (`dst \ src`). A node in both fires nothing.
pub fn transition(src: &[ChainNode], dst: &[ChainNode]) -> (Vec<TagOp>, Vec<TagOp>) {
    let ops = |from: &[ChainNode], other: &[ChainNode], pick: fn(&WriteBack) -> &Vec<TagOp>| {
        from.iter()
            .filter(|n| !other.iter().any(|o| o.key == n.key))
            .filter_map(|n| n.write_back.as_ref())
            .flat_map(|wb| pick(wb).iter().cloned())
            .collect::<Vec<_>>()
    };
    (
        ops(src, dst, |wb| &wb.on_remove),
        ops(dst, src, |wb| &wb.on_add),
    )
}

fn depth_of(ltree: &str) -> usize {
    ltree.split('.').count()
}

/// `a` is an ancestor of `b` or equal to it.
fn under_or_eq(a: &str, b: &str) -> bool {
    TagPath::from_ltree(a).is_ancestor_or_self(&TagPath::from_ltree(b))
}

fn paths(raw: &[String]) -> Vec<TagPath> {
    raw.iter().map(|s| TagPath::from_ltree(s.clone())).collect()
}

/// A query node's own term (`match`-combined `include`, minus `exclude`, or `matchUntagged`).
fn query_own(
    match_mode: MatchMode,
    include: &[String],
    exclude: &[String],
    match_untagged: bool,
) -> TagPredicate {
    TagPredicate {
        include: paths(include),
        match_all: matches!(match_mode, MatchMode::All),
        exclude: paths(exclude),
        untagged: match_untagged,
        ..TagPredicate::all()
    }
}

/// A mirror whose pictures are filtered (a foreign `include` or `exclude`), so its directories
/// must be pruned against picture membership rather than path presence (feature 39 §5.3).
#[derive(Debug, Clone)]
pub struct MirrorFilter {
    pub node_id: String,
    pub tag_root: String,
    /// The picture filter every directory of the mirror applies (ancestors included).
    pub predicate: TagPredicate,
}

/// Every filtered mirror in `config`, with its effective picture filter.
pub fn mirror_filters(config: &HierarchyConfig) -> Vec<MirrorFilter> {
    fn walk(nodes: &[Node], and_terms: &[TagPredicate], out: &mut Vec<MirrorFilter>) {
        for node in nodes {
            match &node.kind {
                NodeKind::Mirror {
                    tag_root,
                    exclude,
                    include,
                    ..
                } => {
                    let foreign: Vec<String> = exclude
                        .iter()
                        .filter(|e| !under_or_eq(tag_root, e))
                        .cloned()
                        .collect();
                    if include.is_empty() && foreign.is_empty() {
                        continue;
                    }
                    out.push(MirrorFilter {
                        node_id: node.id.clone(),
                        tag_root: tag_root.clone(),
                        predicate: TagPredicate {
                            include: paths(include),
                            match_all: true,
                            exclude: paths(&foreign),
                            and_terms: and_terms.to_vec(),
                            ..TagPredicate::all()
                        },
                    });
                }
                NodeKind::Query {
                    match_mode,
                    include,
                    exclude,
                    match_untagged,
                    children,
                    ..
                } => {
                    let mut child_and = and_terms.to_vec();
                    child_and.push(query_own(*match_mode, include, exclude, *match_untagged));
                    walk(children, &child_and, out);
                }
                NodeKind::Static { children } => walk(children, and_terms, out),
                NodeKind::Drop { .. } => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&config.nodes, &[], &mut out);
    out
}

/// Inherited per-node settings threaded down the authored tree.
#[derive(Clone, Copy)]
struct Inherited {
    /// The hierarchy write-back switch (hard ceiling).
    master: bool,
    /// Effective write-back of the parent chain (feature 18 §5.1), seeded `true` at the root.
    enabled: bool,
    trashed_duplicate: TrashedDuplicate,
    sdm: SafeDeleteMode,
    naming: NamingStrategy,
}

impl Inherited {
    /// This node's resolved settings, which are also what its subtree inherits.
    fn at(self, node: &Node) -> Self {
        Self {
            master: self.master,
            enabled: self.master && node.write_back_enabled.unwrap_or(self.enabled),
            trashed_duplicate: node.trashed_duplicate.unwrap_or(self.trashed_duplicate),
            sdm: node.safe_delete_mode.unwrap_or(self.sdm),
            naming: node.naming.unwrap_or(self.naming),
        }
    }
}

/// Build the root directory of `config` from the user's `distinct_paths`; `filtered_paths` holds
/// each filtered mirror's own path set, by node id (see [`mirror_filters`]).
pub fn resolve(
    config: &HierarchyConfig,
    distinct_paths: &[String],
    filtered_paths: &HashMap<String, Vec<String>>,
) -> ResolvedDir {
    let root = Inherited {
        master: config.write_back,
        enabled: true,
        trashed_duplicate: config.trashed_duplicate,
        sdm: config.safe_delete_mode,
        naming: config.naming,
    };
    let paths = Paths {
        distinct: distinct_paths,
        filtered: filtered_paths,
    };
    let roots = build_nodes(&config.nodes, &paths, &[], root);
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
        is_drop: false,
        own_for_parent: None,
        write_back: None,
        mirror_tag: None,
        mirror_include: vec![],
        new_child_mirror: first_hoisted_mirror(&config.nodes, root),
        node_id: None,
        trashed_duplicate: root.trashed_duplicate,
        children: roots,
    }
}

struct Paths<'a> {
    distinct: &'a [String],
    filtered: &'a HashMap<String, Vec<String>>,
}

impl Paths<'_> {
    fn for_mirror(&self, node_id: &str) -> &[String] {
        self.filtered
            .get(node_id)
            .map(Vec::as_slice)
            .unwrap_or(self.distinct)
    }
}

/// The hoisted (`keepDir=false`) mirror a new child of the container maps to (feature 18 §11);
/// a projected one (`minDepth > 1`) has no tag for a new top-level directory, so it is skipped.
fn first_hoisted_mirror(nodes: &[Node], inherited: Inherited) -> Option<MirrorBase> {
    nodes.iter().find_map(|node| match &node.kind {
        NodeKind::Mirror {
            tag_root,
            keep_dir: false,
            include,
            min_depth,
            ..
        } if *min_depth <= 1 => {
            let at = inherited.at(node);
            Some(MirrorBase {
                tag: tag_root.clone(),
                node_id: node.id.clone(),
                include: include.clone(),
                writable: at.enabled,
                trashed_duplicate: at.trashed_duplicate,
            })
        }
        _ => None,
    })
}

/// Recursively build directories for `nodes` under the ancestor predicates `and_terms`.
fn build_nodes(
    nodes: &[Node],
    paths: &Paths,
    and_terms: &[TagPredicate],
    inherited: Inherited,
) -> Vec<ResolvedDir> {
    let mut out = Vec::new();
    for node in nodes {
        let at = inherited.at(node);
        let base = |writable: bool| ResolvedDir {
            name: node.effective_name().unwrap_or_default(),
            writable,
            safe_delete_mode: at.sdm,
            naming: at.naming,
            direct: None,
            subtree: None,
            is_drop: false,
            own_for_parent: None,
            write_back: None,
            mirror_tag: None,
            mirror_include: vec![],
            new_child_mirror: None,
            node_id: Some(node.id.clone()),
            trashed_duplicate: at.trashed_duplicate,
            children: vec![],
        };
        match &node.kind {
            NodeKind::Mirror { .. } => out.extend(expand_mirror(
                node,
                paths.for_mirror(&node.id),
                and_terms,
                at,
            )),
            NodeKind::Query {
                match_mode,
                include,
                exclude,
                match_untagged,
                write_back,
                children,
            } => {
                let own_base = query_own(*match_mode, include, exclude, *match_untagged);
                // Children inherit ancestors + this node's own term.
                let mut child_and = and_terms.to_vec();
                child_and.push(own_base.clone());
                let child_dirs = build_nodes(children, paths, &child_and, at);
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
                let writable = at.enabled && write_back.is_some();
                out.push(ResolvedDir {
                    direct: Some(direct),
                    subtree: Some(membership),
                    own_for_parent: Some(own_base),
                    write_back: if writable { write_back.clone() } else { None },
                    new_child_mirror: first_hoisted_mirror(children, at),
                    children: child_dirs,
                    ..base(writable)
                });
            }
            NodeKind::Static { children } => {
                // A static node is never writable itself, but its toggle sets the inherited
                // default for descendants (feature 18 §5).
                out.push(ResolvedDir {
                    new_child_mirror: first_hoisted_mirror(children, at),
                    children: build_nodes(children, paths, and_terms, at),
                    ..base(false)
                });
            }
            NodeKind::Drop { on_add } => {
                // Write-only inbox (feature 18 §4): always shown, lists only its recent uploads,
                // always writable (ignores master + writeBackEnabled), applies the fixed on_add.
                out.push(ResolvedDir {
                    is_drop: true,
                    write_back: Some(WriteBack {
                        on_add: on_add.clone(),
                        on_remove: vec![],
                    }),
                    ..base(true)
                });
            }
        }
    }
    out
}

/// Expand a `mirror` node into its directory subtree from the distinct tag paths.
fn expand_mirror(
    node: &Node,
    distinct_paths: &[String],
    and_terms: &[TagPredicate],
    at: Inherited,
) -> Vec<ResolvedDir> {
    let NodeKind::Mirror {
        tag_root,
        keep_dir,
        collapsed,
        exclude,
        include,
        min_depth,
        max_depth,
        deeper_mode,
    } = &node.kind
    else {
        return vec![];
    };
    let root_depth = depth_of(tag_root);
    // Depth of the shallowest generated level (feature 39 §5.2); `root_depth + 1` = no projection.
    let top_depth = root_depth + (*min_depth).max(1) as usize;
    let projected = *min_depth > 1;

    // Split excludes (feature 18 §7.3): sub-tag excludes (`<@ tagRoot`) prune directories AND
    // pictures; foreign excludes are a pure picture-membership cut applied to every directory.
    let (mut sub_excludes, foreign_excludes): (Vec<String>, Vec<String>) = exclude
        .iter()
        .cloned()
        .partition(|er| under_or_eq(tag_root, er));
    // Collapsed entries above the projection cut are moot (feature 39 §12 edge 10).
    let mut collapsed: Vec<String> = collapsed
        .iter()
        .filter(|c| depth_of(c) >= top_depth || !projected)
        .cloned()
        .collect();

    // maxDepth (§7.1–7.2): every tag path deeper than the cut folds at its level-(maxDepth+1)
    // ancestor, which we inject as a synthetic collapsed (roll-up) or excluded (drop) root —
    // reusing the existing machinery so directory generation naturally stops at the cut.
    if *max_depth >= 1 {
        let cut_depth = root_depth + *max_depth as usize;
        let cut_roots: HashSet<String> = distinct_paths
            .iter()
            .filter(|p| {
                under_or_eq(tag_root, p)
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
        .filter(|p| under_or_eq(tag_root, p) && !sub_excludes.iter().any(|er| under_or_eq(er, p)))
        .collect();

    // Directory paths: every prefix at or below the top level (plus tagRoot itself when not
    // projected), not inside a collapsed subtree. A path above a projection cut has no directory.
    let first = if projected { top_depth } else { root_depth };
    let mut dir_paths: HashSet<String> = HashSet::new();
    for p in &relevant {
        let labels: Vec<&str> = p.split('.').collect();
        for i in first..=labels.len() {
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
        if !under_or_eq(tag_root, cr) || sub_excludes.iter().any(|er| under_or_eq(er, cr)) {
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

    // A foreign include is one more membership term on every directory (feature 39 §5.1).
    let mut and_terms = and_terms.to_vec();
    if !include.is_empty() {
        and_terms.push(TagPredicate {
            include: paths(include),
            match_all: true,
            ..TagPredicate::all()
        });
    }
    let ctx = MirrorCtx {
        dir_paths,
        collapsed_arms,
        sub_excludes,
        foreign_excludes,
        and_terms,
        mirror: MirrorBase {
            tag: tag_root.clone(),
            node_id: node.id.clone(),
            include: include.clone(),
            writable: at.enabled,
            trashed_duplicate: at.trashed_duplicate,
        },
        sdm: at.sdm,
        naming: at.naming,
    };
    let name = node.effective_name().unwrap_or_default();

    if !projected {
        return if *keep_dir && ctx.dir_paths.contains(tag_root) {
            vec![build_mirror_dir(tag_root, Some(name), &ctx)]
        } else {
            // keepDir = false (or tagRoot collapsed): the root label is stripped — its children
            // sit at the node's level.
            immediate_children(tag_root, &ctx.dir_paths)
                .into_iter()
                .map(|c| build_mirror_dir(&c, None, &ctx))
                .collect()
        };
    }

    // Projection: the top level is hoisted to the node's level (or under a tagRoot container).
    let mut tops: Vec<String> = ctx
        .dir_paths
        .iter()
        .filter(|p| depth_of(p) == top_depth)
        .cloned()
        .collect();
    tops.sort();
    let dirs: Vec<ResolvedDir> = projected_names(&tops, root_depth)
        .into_iter()
        .zip(&tops)
        .map(|(n, p)| build_mirror_dir(p, n, &ctx))
        .collect();
    if !*keep_dir || dirs.is_empty() {
        return dirs;
    }
    // The tagRoot container: no direct files, not writable (nothing to assign above the cut).
    vec![ResolvedDir {
        name,
        writable: false,
        safe_delete_mode: ctx.sdm,
        naming: ctx.naming,
        direct: None,
        subtree: None,
        is_drop: false,
        own_for_parent: Some(own_under(tag_root, &ctx.mirror.include)),
        write_back: None,
        mirror_tag: Some(tag_root.clone()),
        mirror_include: ctx.mirror.include.clone(),
        new_child_mirror: None,
        node_id: Some(node.id.clone()),
        trashed_duplicate: ctx.mirror.trashed_duplicate,
        children: dirs,
    }]
}

/// Names for hoisted top-level directories: the leaf label, qualified by the suppressed levels
/// when two leaves collide ignoring case (`Paris (2025)`, `Paris (2026)`).
fn projected_names(tops: &[String], root_depth: usize) -> Vec<Option<String>> {
    let leaf = |p: &str| leaf_label(p).to_lowercase();
    tops.iter()
        .map(|p| {
            if tops.iter().filter(|o| leaf(o) == leaf(p)).count() < 2 {
                return None;
            }
            let labels: Vec<&str> = p.split('.').collect();
            let hidden = labels[root_depth..labels.len() - 1].join(".");
            Some(format!("{} ({hidden})", leaf_label(p)))
        })
        .collect()
}

/// Membership under `tag` carrying every `include` arm: what a parent subtracts for a mirror dir.
fn own_under(tag: &str, include: &[String]) -> TagPredicate {
    let mut arms = vec![TagPath::from_ltree(tag.to_string())];
    arms.extend(paths(include));
    TagPredicate {
        include: arms,
        match_all: true,
        ..TagPredicate::all()
    }
}

struct MirrorCtx {
    dir_paths: HashSet<String>,
    collapsed_arms: HashMap<String, Vec<String>>,
    /// Sub-tag excludes (`<@ tagRoot`) — prune the subtree; applied to the `subtree` predicate.
    sub_excludes: Vec<String>,
    /// Foreign excludes (not under tagRoot) — a picture-membership cut on every directory.
    foreign_excludes: Vec<String>,
    /// Ancestor predicates plus the mirror's foreign `include` term.
    and_terms: Vec<TagPredicate>,
    /// The mirror node, its `tag` being `tagRoot`.
    mirror: MirrorBase,
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
    let mut cut: Vec<TagPath> = paths(&ctx.foreign_excludes);
    for e in &ctx.sub_excludes {
        if under_or_eq(path, e) {
            cut.push(TagPath::from_ltree(e.clone()));
        }
    }

    // Membership for direct files: exact T plus any collapsed subtrees rolled into this dir.
    let include = ctx
        .collapsed_arms
        .get(path)
        .map(|arms| paths(arms))
        .unwrap_or_default();
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
    let mut subtree_exclude = paths(&ctx.sub_excludes);
    subtree_exclude.extend(paths(&ctx.foreign_excludes));
    let subtree = TagPredicate {
        include: vec![TagPath::from_ltree(path.to_string())],
        match_all: true,
        exclude: subtree_exclude,
        and_terms: ctx.and_terms.clone(),
        ..TagPredicate::all()
    };
    ResolvedDir {
        name: name_override.unwrap_or_else(|| leaf_label(path)),
        writable: ctx.mirror.writable,
        safe_delete_mode: ctx.sdm,
        naming: ctx.naming,
        direct: Some(direct),
        subtree: Some(subtree),
        is_drop: false,
        // Parent subtracts membership under this dir's tag (inclusive).
        own_for_parent: Some(own_under(path, &ctx.mirror.include)),
        // Mirror write-back is implicit: assign/remove the directory's own tag (§7.1).
        write_back: ctx.mirror.writable.then(|| ctx.mirror.write_back(path)),
        mirror_tag: Some(path.to_string()),
        mirror_include: ctx.mirror.include.clone(),
        new_child_mirror: None,
        node_id: Some(ctx.mirror.node_id.clone()),
        trashed_duplicate: ctx.mirror.trashed_duplicate,
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


#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(json: serde_json::Value) -> HierarchyConfig {
        let c: HierarchyConfig = serde_json::from_value(json).unwrap();
        c.validate().unwrap();
        c
    }

    fn tree(config: &HierarchyConfig, paths: &[&str]) -> ResolvedDir {
        let paths: Vec<String> = paths.iter().map(|s| s.to_string()).collect();
        resolve(config, &paths, &HashMap::new())
    }

    fn names(dir: &ResolvedDir) -> Vec<&str> {
        dir.children.iter().map(|c| c.name.as_str()).collect()
    }

    fn seg(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    fn mirror(extra: serde_json::Value) -> HierarchyConfig {
        let mut node = serde_json::json!({"id": "m", "kind": "mirror", "tagRoot": "Era"});
        node.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        cfg(serde_json::json!({"nodes": [node]}))
    }

    const ERAS: &[&str] = &[
        "Era.2025.Japan",
        "Era.2026.Vietnam",
        "Era.2026.Laos.Hanoi",
        "Era.2027",
    ];

    #[test]
    fn min_depth_hoists_the_projected_level() {
        let root = tree(&mirror(serde_json::json!({"minDepth": 2})), ERAS);
        assert_eq!(names(&root), ["Japan", "Laos", "Vietnam"]);
        // Below the cut, directories nest as usual; the level above the cut (`Era.2027`) is gone.
        let laos = find_dir(&root, &seg(&["Laos"])).unwrap();
        assert_eq!(names(laos), ["Hanoi"]);
        assert_eq!(laos.mirror_tag.as_deref(), Some("Era.2026.Laos"));
    }

    #[test]
    fn min_depth_with_keep_dir_renders_a_read_only_container() {
        let root = tree(
            &mirror(serde_json::json!({"minDepth": 2, "keepDir": true})),
            ERAS,
        );
        assert_eq!(names(&root), ["Era"]);
        let era = &root.children[0];
        assert!(!era.writable && era.direct.is_none() && era.write_back.is_none());
        assert_eq!(names(era), ["Japan", "Laos", "Vietnam"]);
    }

    #[test]
    fn min_depth_equal_to_max_depth_projects_one_level() {
        let root = tree(
            &mirror(serde_json::json!({"minDepth": 2, "maxDepth": 2})),
            ERAS,
        );
        assert_eq!(names(&root), ["Japan", "Laos", "Vietnam"]);
        assert!(
            find_dir(&root, &seg(&["Laos"]))
                .unwrap()
                .children
                .is_empty()
        );
    }

    #[test]
    fn projected_name_collisions_are_qualified() {
        let root = tree(
            &mirror(serde_json::json!({"minDepth": 2})),
            &["Era.2025.Paris", "Era.2026.paris", "Era.2026.Rome"],
        );
        assert_eq!(names(&root), ["Paris (2025)", "Rome", "paris (2026)"]);
    }

    #[test]
    fn collapsed_above_the_cut_is_moot() {
        let root = tree(
            &mirror(serde_json::json!({"minDepth": 2, "collapsed": ["Era.2026"]})),
            ERAS,
        );
        assert_eq!(names(&root), ["Japan", "Laos", "Vietnam"]);
    }

    #[test]
    fn foreign_include_filters_and_joins_the_op_list() {
        let root = tree(&mirror(serde_json::json!({"include": ["Travel"]})), ERAS);
        let japan = find_dir(&root, &seg(&["2025", "Japan"])).unwrap();
        let direct = japan.direct.as_ref().unwrap();
        assert!(
            direct
                .and_terms
                .iter()
                .any(|t| t.include == [TagPath::from("Travel")])
        );
        let wb = japan.write_back.as_ref().unwrap();
        let assigned: Vec<&str> = wb.on_add.iter().map(|o| o.path.as_str()).collect();
        assert_eq!(assigned, ["Era.2025.Japan", "Travel"]);
        assert_eq!(
            wb.on_remove.len(),
            1,
            "onRemove only drops the directory's own tag"
        );
    }

    #[test]
    fn filtered_mirror_uses_its_own_path_set() {
        let config = mirror(serde_json::json!({"include": ["Travel"]}));
        let filters = mirror_filters(&config);
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].predicate.include, [TagPath::from("Travel")]);
        let all: Vec<String> = ERAS.iter().map(|s| s.to_string()).collect();
        let only = HashMap::from([("m".to_string(), vec!["Era.2025.Japan".to_string()])]);
        assert_eq!(names(&resolve(&config, &all, &only)), ["2025"]);
    }

    #[test]
    fn unfiltered_and_sub_tag_excluded_mirrors_are_not_filtered() {
        assert!(mirror_filters(&mirror(serde_json::json!({"exclude": ["Era.2027"]}))).is_empty());
        let foreign = mirror_filters(&mirror(serde_json::json!({"exclude": ["Private"]})));
        assert_eq!(foreign[0].predicate.exclude, [TagPath::from("Private")]);
    }

    fn voyage() -> HierarchyConfig {
        cfg(serde_json::json!({"nodes": [
            {"id": "q", "kind": "query", "name": "Voyage", "include": ["Travel"],
             "writeBack": {"onAdd": [{"op": "assign", "path": "Travel"}],
                           "onRemove": [{"op": "remove", "path": "Travel"}]},
             "children": [{"id": "m", "kind": "mirror", "tagRoot": "Era"}]},
            {"id": "o", "kind": "query", "name": "Other", "include": ["Other"],
             "writeBack": {"onAdd": [{"op": "assign", "path": "Other"}],
                           "onRemove": [{"op": "remove", "path": "Other"}]}}
        ]}))
    }

    fn paths_of(ops: &[TagOp]) -> Vec<String> {
        ops.iter()
            .map(|o| format!("{:?} {}", o.op, o.path))
            .collect()
    }

    #[test]
    fn sibling_move_under_a_shared_query_fires_only_the_mirror_steps() {
        let root = tree(&voyage(), ERAS);
        let src = node_chain(&root, &seg(&["Voyage", "2026", "Vietnam"])).unwrap();
        let dst = node_chain(&root, &seg(&["Voyage", "2026", "Laos"])).unwrap();
        // The intermediate `2026` level of the same mirror is not a node of its own.
        assert_eq!(src.len(), 2);
        let (leave, enter) = transition(&src, &dst);
        assert_eq!(paths_of(&leave), ["Remove Era.2026.Vietnam"]);
        assert_eq!(paths_of(&enter), ["Assign Era.2026.Laos"]);
    }

    #[test]
    fn move_out_of_the_subtree_fires_the_whole_chain() {
        let root = tree(&voyage(), ERAS);
        let src = node_chain(&root, &seg(&["Voyage", "2026", "Vietnam"])).unwrap();
        let dst = node_chain(&root, &seg(&["Other"])).unwrap();
        let (leave, enter) = transition(&src, &dst);
        assert_eq!(
            paths_of(&leave),
            ["Remove Travel", "Remove Era.2026.Vietnam"]
        );
        assert_eq!(paths_of(&enter), ["Assign Other"]);
    }

    #[test]
    fn put_enters_every_node_of_the_chain() {
        let root = tree(&voyage(), ERAS);
        let dst = node_chain(&root, &seg(&["Voyage", "2026", "Vietnam"])).unwrap();
        let (_, enter) = transition(&[], &dst);
        assert_eq!(
            paths_of(&enter),
            ["Assign Travel", "Assign Era.2026.Vietnam"]
        );
    }

    #[test]
    fn write_back_nearest_explicit_ancestor_wins_under_the_master_ceiling() {
        let nodes = serde_json::json!([
            {"id": "off", "kind": "static", "name": "Off", "writeBackEnabled": false, "children": [
                {"id": "on", "kind": "query", "name": "On", "include": ["X"], "writeBackEnabled": true,
                 "writeBack": {"onAdd": [{"op": "assign", "path": "X"}],
                               "onRemove": [{"op": "remove", "path": "X"}]}},
                {"id": "inh", "kind": "query", "name": "Inh", "include": ["Y"],
                 "writeBack": {"onAdd": [{"op": "assign", "path": "Y"}],
                               "onRemove": [{"op": "remove", "path": "Y"}]}}
            ]}
        ]);
        let writable = |master: bool, path: &[&str]| {
            let root = tree(
                &cfg(serde_json::json!({"writeBack": master, "nodes": nodes})),
                &[],
            );
            find_dir(&root, &seg(path)).unwrap().writable
        };
        assert!(writable(true, &["Off", "On"]), "explicit override");
        assert!(
            !writable(true, &["Off", "Inh"]),
            "inherits the off ancestor"
        );
        assert!(
            !writable(false, &["Off", "On"]),
            "master off is a hard ceiling"
        );
    }

    #[test]
    fn trashed_duplicate_inherits_like_write_back() {
        let root = tree(
            &cfg(serde_json::json!({"nodes": [
                {"id": "s", "kind": "static", "name": "S", "trashedDuplicate": "ignore", "children": [
                    {"id": "m", "kind": "mirror", "tagRoot": "Era"}
                ]},
                {"id": "d", "kind": "drop", "name": "Inbox", "onAdd": []}
            ]})),
            ERAS,
        );
        let s = find_dir(&root, &seg(&["S"])).unwrap();
        assert_eq!(s.trashed_duplicate, TrashedDuplicate::Ignore);
        assert_eq!(s.children[0].trashed_duplicate, TrashedDuplicate::Ignore);
        let inbox = find_dir(&root, &seg(&["Inbox"])).unwrap();
        assert_eq!(
            inbox.trashed_duplicate,
            TrashedDuplicate::Restore,
            "root seed"
        );
    }

    #[test]
    fn trashed_duplicate_root_seed_comes_from_the_config() {
        let root = tree(
            &cfg(serde_json::json!({"trashedDuplicate": "ignore", "nodes": [
                {"id": "d", "kind": "drop", "name": "Inbox", "onAdd": []}
            ]})),
            ERAS,
        );
        let inbox = find_dir(&root, &seg(&["Inbox"])).unwrap();
        assert_eq!(inbox.trashed_duplicate, TrashedDuplicate::Ignore);
    }
}
