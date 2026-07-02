//! Hierarchy data model, validation, and the internal `TagPredicate`.
//!
//! A hierarchy is an ordered tree of nodes (`config` JSONB) mapping the user's tag graph to a
//! filesystem-like directory tree. It stores **no pictures** — every directory resolves to a
//! [`TagPredicate`] over a picture's stored tag set, and membership is derived live.
//!
//! See `doc/features/05_hierarchies.md` for the full specification. This module owns the pure
//! types and their validation; the read resolver lives in `services::hierarchy` and the SQL
//! rendering of [`TagPredicate`] lives in `repository::picture`.

use super::tag::TagPath;
use serde::{Deserialize, Serialize};

// ─── Config enums ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SafeDeleteMode {
    #[default]
    #[serde(rename = "singleBranch")]
    SingleBranch,
    #[serde(rename = "fullDelete")]
    FullDelete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NamingStrategy {
    #[default]
    Original,
    Date,
    Id,
}

/// Combinator over a query node's flat `include` list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchMode {
    #[default]
    All,
    Any,
}

/// What a `mirror` does with pictures whose deepest tag sits below its `maxDepth` cut
/// (feature 18 §7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeeperMode {
    /// Roll the picture up to the deepest allowed directory (the existing collapse roll-up).
    #[default]
    Collapse,
    /// Drop the picture (and any directory below the cut) from the mirror.
    Exclude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TagOpKind {
    Assign,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagOp {
    pub op: TagOpKind,
    pub path: String,
}

/// Tag operations applied when a picture is added to / removed from a writable directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteBack {
    #[serde(default)]
    pub on_add: Vec<TagOp>,
    #[serde(default)]
    pub on_remove: Vec<TagOp>,
}

// ─── Node tree ──────────────────────────────────────────────────────────────────

/// A single node in a hierarchy's `config`. Renders to one (or, for `mirror`, a subtree of)
/// directory. Common fields live here; kind-specific fields are flattened from [`NodeKind`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    /// Stable, unique-within-hierarchy id (sidebar keys, write-path resolution, reorder).
    pub id: String,
    /// Directory label. Required for `query`/`static`; optional override for `mirror`
    /// (defaults to the `tagRoot`'s last label).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub naming: Option<NamingStrategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safe_delete_mode: Option<SafeDeleteMode>,
    /// Per-node write-back tri-state (feature 18 §5). `None` = inherit the nearest explicit
    /// ancestor (root seed = the hierarchy master switch); `Some(true/false)` overrides for this
    /// node and its subtree. Inert (ceiling wins) when the master switch is off; ignored on
    /// `drop` nodes (always writable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_back_enabled: Option<bool>,
    #[serde(flatten)]
    pub kind: NodeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum NodeKind {
    /// Mirrors the live tag subtree under `tag_root`. A leaf in the authored JSON — its
    /// descendant directories come from the tag set at resolve time.
    Mirror {
        tag_root: String,
        #[serde(default)]
        keep_dir: bool,
        #[serde(default)]
        collapsed: Vec<String>,
        #[serde(default)]
        exclude: Vec<String>,
        /// Cap directory generation to N tag levels below `tag_root`. `0`/absent = unrestricted
        /// (feature 18 §7.1).
        #[serde(default)]
        max_depth: u32,
        /// How pictures below the `max_depth` cut are handled (feature 18 §7.2).
        #[serde(default)]
        deeper_mode: DeeperMode,
    },
    /// Explicit predicate node; may nest. Effective read predicate = own ∧ all ancestors.
    Query {
        #[serde(rename = "match", default)]
        match_mode: MatchMode,
        #[serde(default)]
        include: Vec<String>,
        #[serde(default)]
        exclude: Vec<String>,
        #[serde(default)]
        match_untagged: bool,
        /// `None` ⇒ read-only directory (subject to the effective write-back gate).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        write_back: Option<WriteBack>,
        #[serde(default)]
        children: Vec<Node>,
    },
    /// Pure container — no predicate, no direct pictures, read-only.
    Static {
        #[serde(default)]
        children: Vec<Node>,
    },
    /// Write-only inbox (feature 18 §4): always shown, lists nothing, and applies a fixed
    /// `on_add` op-list to every upload. Always writable, even when the master switch is off.
    Drop {
        #[serde(default)]
        on_add: Vec<TagOp>,
    },
}

impl NodeKind {
    /// The authored child nodes, for kinds that carry them (`query`/`static`).
    pub fn children(&self) -> Option<&[Node]> {
        match self {
            NodeKind::Query { children, .. } | NodeKind::Static { children } => Some(children),
            NodeKind::Mirror { .. } | NodeKind::Drop { .. } => None,
        }
    }
}

fn default_version() -> u32 {
    2
}
fn default_write_back() -> bool {
    true
}

/// Top-level hierarchy configuration stored in `hierarchies.config`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HierarchyConfig {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub safe_delete_mode: SafeDeleteMode,
    #[serde(default)]
    pub naming: NamingStrategy,
    /// Master switch; `false` ⇒ entire hierarchy read-only (fullDelete still allowed).
    #[serde(default = "default_write_back")]
    pub write_back: bool,
    #[serde(default)]
    pub nodes: Vec<Node>,
}

impl Default for HierarchyConfig {
    fn default() -> Self {
        Self {
            version: 2,
            safe_delete_mode: SafeDeleteMode::default(),
            naming: NamingStrategy::default(),
            write_back: true,
            nodes: vec![],
        }
    }
}

impl Node {
    /// The directory label this node renders to (`name`, or a mirror's `tagRoot` last label).
    /// Returns `None` only for a `query`/`static` node missing its required `name`.
    pub fn effective_name(&self) -> Option<String> {
        if let Some(ref n) = self.name {
            return Some(n.clone());
        }
        match &self.kind {
            NodeKind::Mirror { tag_root, .. } => tag_root.rsplit('.').next().map(|s| s.to_string()),
            _ => None,
        }
    }
}

// ─── Slug (WebDAV mount path) ─────────────────────────────────────────────────────

/// Slugify a hierarchy name into the `/webdav/{slug}` path segment (06_webdav.md §4).
/// Lowercases, maps any run of non-`[a-z0-9]` characters to a single `-`, and trims
/// leading/trailing `-`. Empty result (e.g. a name of only symbols) falls back to
/// `hierarchy`. The slug is human-readable only — the token is the authority; the slug is
/// verified against the resolved hierarchy's name.
pub fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut prev_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "hierarchy".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Fold a candidate ltree tag path onto existing tags that differ only by case (06_webdav.md
/// §10c). A case-insensitive WebDAV client (macOS/Windows) may write `Photos/travel` against an
/// existing `Photos.Travel` tag; without folding this would mint a distinct case-only-duplicate
/// the client cannot tell apart. For each path component, top-down, if some existing tag shares
/// the already-normalized ancestor prefix and a component matching case-insensitively (but not
/// exactly), adopt the existing casing — reusing the existing tag instead of minting a sibling.
///
/// `existing` is the user's distinct stored tag paths (dot-separated ltree form).
pub fn reuse_existing_case(candidate: &str, existing: &[String]) -> String {
    let comps: Vec<&str> = candidate.split('.').collect();
    let mut result: Vec<String> = Vec::with_capacity(comps.len());
    for (i, comp) in comps.iter().enumerate() {
        let mut chosen = comp.to_string();
        for ex in existing {
            let ex_comps: Vec<&str> = ex.split('.').collect();
            if ex_comps.len() <= i {
                continue;
            }
            // The ancestor prefix (already normalized) must match exactly, and this component
            // must match case-insensitively but differ in casing.
            let prefix_matches = (0..i).all(|j| ex_comps[j] == result[j]);
            if prefix_matches && ex_comps[i] != *comp && ex_comps[i].eq_ignore_ascii_case(comp) {
                chosen = ex_comps[i].to_string();
                break;
            }
        }
        result.push(chosen);
    }
    result.join(".")
}

// ─── Validation ─────────────────────────────────────────────────────────────────

impl HierarchyConfig {
    /// Validate the full config (§11). Pure; called by the service on create/patch.
    pub fn validate(&self) -> Result<(), String> {
        let mut seen_ids = std::collections::HashSet::new();
        validate_nodes(&self.nodes, &mut seen_ids)
    }

    /// Resolve effective write-back for the node with `node_id` (feature 18 §5.1): the master
    /// switch is a hard ceiling; otherwise the nearest explicit `writeBackEnabled` ancestor wins
    /// (root seed = `true`). Returns `false` for an unknown id. Note: `drop` nodes are writable
    /// regardless of this — callers special-case them by kind.
    pub fn effective_enabled(&self, node_id: &str) -> bool {
        if !self.write_back {
            return false;
        }
        fn walk(nodes: &[Node], target: &str, inherited: bool) -> Option<bool> {
            for n in nodes {
                let here = n.write_back_enabled.unwrap_or(inherited);
                if n.id == target {
                    return Some(here);
                }
                if let Some(children) = n.kind.children() {
                    if let Some(v) = walk(children, target, here) {
                        return Some(v);
                    }
                }
            }
            None
        }
        walk(&self.nodes, node_id, true).unwrap_or(false)
    }
}

fn validate_nodes(
    nodes: &[Node],
    seen_ids: &mut std::collections::HashSet<String>,
) -> Result<(), String> {
    // Sibling name uniqueness over the resolved directory labels.
    // Sibling names must be unique case-insensitively — WebDAV runs over case-insensitive
    // filesystems (macOS/Windows), so `Fav` and `fav` would collide on a mounted drive
    // (06_webdav.md §10a). Reject the collision at save time.
    let mut seen_names = std::collections::HashSet::new();
    for node in nodes {
        let name = node
            .effective_name()
            .ok_or_else(|| format!("node {:?} of this kind requires a `name`", node.id))?;
        if !seen_names.insert(name.to_lowercase()) {
            return Err(format!(
                "duplicate sibling directory name {name:?} (names must be unique ignoring case)"
            ));
        }
        if !seen_ids.insert(node.id.clone()) {
            return Err(format!("duplicate node id {:?}", node.id));
        }
        validate_node(node, seen_ids)?;
    }
    Ok(())
}

fn validate_node(
    node: &Node,
    seen_ids: &mut std::collections::HashSet<String>,
) -> Result<(), String> {
    match &node.kind {
        NodeKind::Mirror {
            tag_root,
            collapsed,
            exclude,
            ..
        } => {
            let root = parse_path(tag_root)?;
            // `collapsed` must stay under tagRoot; `exclude` may be foreign now (feature 18 §7.3)
            // — a foreign exclude is a pure picture-membership cut, so it only needs to parse.
            for entry in collapsed {
                let p = parse_path(entry)?;
                if !(p == root || root.is_ancestor_of(&p)) {
                    return Err(format!(
                        "mirror collapsed entry {entry:?} must be under tagRoot {tag_root:?}"
                    ));
                }
            }
            for entry in exclude {
                parse_path(entry)?;
            }
        }
        NodeKind::Query {
            match_mode,
            include,
            exclude,
            match_untagged,
            write_back,
            children,
        } => {
            for p in include.iter().chain(exclude.iter()) {
                parse_path(p)?;
            }
            if *match_untagged && (!include.is_empty() || !exclude.is_empty()) {
                return Err(format!(
                    "node {:?}: matchUntagged requires empty include and exclude",
                    node.id
                ));
            }
            // Untagged write-back is now permitted (feature 18 §6); its op-list is free-form
            // ("no stored tag" is not an include/exclude predicate, so §7.2 compliance is skipped).
            if let (Some(wb), false) = (write_back, *match_untagged) {
                validate_write_back(wb, include, exclude, *match_mode)
                    .map_err(|e| format!("node {:?}: {e}", node.id))?;
            }
            validate_nodes(children, seen_ids)?;
        }
        NodeKind::Static { children } => {
            validate_nodes(children, seen_ids)?;
        }
        NodeKind::Drop { on_add } => {
            // A leaf inbox (feature 18 §4): the fixed op-list only needs to parse — no read
            // predicate to satisfy, so no compliance check.
            for op in on_add {
                parse_path(&op.path)?;
            }
        }
    }
    Ok(())
}

fn parse_path(raw: &str) -> Result<TagPath, String> {
    // SharedToMe is allowed in hierarchy config (received pictures are first-class — §11).
    TagPath::parse(raw, true)
}

/// §7.2 compliance: the op-list must be structurally capable of making a picture satisfy
/// (`onAdd`) or stop satisfying (`onRemove`) the directory's read predicate.
fn validate_write_back(
    wb: &WriteBack,
    include: &[String],
    exclude: &[String],
    match_mode: MatchMode,
) -> Result<(), String> {
    let add_assign: std::collections::HashSet<&str> = wb
        .on_add
        .iter()
        .filter(|o| o.op == TagOpKind::Assign)
        .map(|o| o.path.as_str())
        .collect();
    let add_remove: std::collections::HashSet<&str> = wb
        .on_add
        .iter()
        .filter(|o| o.op == TagOpKind::Remove)
        .map(|o| o.path.as_str())
        .collect();
    let rem_remove: std::collections::HashSet<&str> = wb
        .on_remove
        .iter()
        .filter(|o| o.op == TagOpKind::Remove)
        .map(|o| o.path.as_str())
        .collect();
    let rem_assign: std::collections::HashSet<&str> = wb
        .on_remove
        .iter()
        .filter(|o| o.op == TagOpKind::Assign)
        .map(|o| o.path.as_str())
        .collect();

    // onAdd: satisfy the include term...
    let include_ok = match match_mode {
        MatchMode::All => include.iter().all(|p| add_assign.contains(p.as_str())),
        MatchMode::Any => {
            include.is_empty() || include.iter().any(|p| add_assign.contains(p.as_str()))
        }
    };
    if !include_ok {
        return Err(
            "onAdd does not assign the include tags required to satisfy the predicate".into(),
        );
    }
    // ...and clear every excluded tag.
    if !exclude.iter().all(|p| add_remove.contains(p.as_str())) {
        return Err("onAdd does not remove every excluded tag".into());
    }

    // onRemove: at least one breaking op.
    let breaks_include = match match_mode {
        MatchMode::All => include.iter().any(|p| rem_remove.contains(p.as_str())),
        MatchMode::Any => {
            !include.is_empty() && include.iter().all(|p| rem_remove.contains(p.as_str()))
        }
    };
    let breaks_exclude = exclude.iter().any(|p| rem_assign.contains(p.as_str()));
    if !breaks_include && !breaks_exclude {
        return Err(
            "onRemove cannot break the predicate (no include-remove or exclude-assign op)".into(),
        );
    }
    Ok(())
}

// ─── TagPredicate ───────────────────────────────────────────────────────────────

/// A predicate over a picture's stored tag set, rendered to SQL by `repository::picture`.
///
/// Membership (`own`) is: `untagged` ⇒ no tag rows; otherwise the OR/AND (per `match_all`) of the
/// positive arms `include` (inclusive `<@`) and `exact` (non-inclusive `=`). An empty `include`
/// **and** `exact` with `untagged = false` matches **all** pictures.
///
/// The full match is `own ∧ (⋀ and_terms) ∧ (none of `exclude`) ∧ (none of `minus_children`)`.
/// `and_terms` carries inherited ancestor predicates (a query node's effective predicate is
/// `own ∧ all ancestors`). `minus_children` encodes "most-specific node wins": a picture is a
/// direct file of a directory only if it does not also belong to one of the directory's visible
/// children.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TagPredicate {
    /// Inclusive (`tag_path <@ p`) positive arms.
    pub include: Vec<TagPath>,
    /// `true` = AND the positive arms, `false` = OR them. Ignored when no positive arms.
    pub match_all: bool,
    /// Reject if the picture has any tag under one of these (inclusive).
    pub exclude: Vec<TagPath>,
    /// Strict "no stored tag of any source".
    pub untagged: bool,
    /// Exact (`tag_path = p`, non-inclusive) positive arms — mirror exact-T membership.
    pub exact: Vec<TagPath>,
    /// Inherited ancestor predicates, AND-combined into membership (query inheritance).
    pub and_terms: Vec<TagPredicate>,
    /// `own(childᵢ)` terms to subtract (most-specific-node-wins).
    pub minus_children: Vec<TagPredicate>,
}

impl TagPredicate {
    /// Matches every picture (the vacuously-true `own`).
    pub fn all() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(json: serde_json::Value) -> Result<HierarchyConfig, String> {
        let c: HierarchyConfig = serde_json::from_value(json).map_err(|e| e.to_string())?;
        c.validate().map(|_| c)
    }

    #[test]
    fn default_config_roundtrips() {
        let json = serde_json::json!({
            "version": 1, "safeDeleteMode": "singleBranch", "naming": "original",
            "writeBack": true, "nodes": []
        });
        let c = cfg(json).unwrap();
        assert_eq!(c.version, 1);
        assert!(c.write_back);
        assert!(matches!(c.safe_delete_mode, SafeDeleteMode::SingleBranch));
    }

    #[test]
    fn parses_all_three_node_kinds() {
        let json = serde_json::json!({"nodes": [
            {"id": "n1", "kind": "mirror", "name": "Photos", "tagRoot": "Photos",
             "keepDir": false, "collapsed": ["Photos.Travel.Alps.Hiking"], "exclude": ["Photos.Outdoor"]},
            {"id": "n2", "kind": "query", "name": "Favorites", "match": "all",
             "include": ["Starred"], "exclude": [],
             "writeBack": {"onAdd": [{"op": "assign", "path": "Starred"}],
                           "onRemove": [{"op": "remove", "path": "Starred"}]}},
            {"id": "n3", "kind": "static", "name": "Albums", "children": []}
        ]});
        let c = cfg(json).unwrap();
        assert_eq!(c.nodes.len(), 3);
    }

    #[test]
    fn mirror_name_defaults_to_tag_root_label() {
        let n: Node = serde_json::from_value(serde_json::json!(
            {"id": "n1", "kind": "mirror", "tagRoot": "Photos.Travel"}
        ))
        .unwrap();
        assert_eq!(n.effective_name().as_deref(), Some("Travel"));
    }

    #[test]
    fn rejects_duplicate_sibling_names() {
        let json = serde_json::json!({"nodes": [
            {"id": "a", "kind": "static", "name": "Dup", "children": []},
            {"id": "b", "kind": "static", "name": "Dup", "children": []}
        ]});
        assert!(cfg(json).is_err());
    }

    #[test]
    fn rejects_case_only_duplicate_sibling_names() {
        let json = serde_json::json!({"nodes": [
            {"id": "a", "kind": "static", "name": "Fav", "children": []},
            {"id": "b", "kind": "static", "name": "fav", "children": []}
        ]});
        assert!(cfg(json).is_err());
    }

    #[test]
    fn slugify_basics() {
        assert_eq!(slugify("My Photos"), "my-photos");
        assert_eq!(slugify("Photos/Travel 2024!"), "photos-travel-2024");
        assert_eq!(slugify("  spaced  "), "spaced");
        assert_eq!(slugify("***"), "hierarchy");
        assert_eq!(slugify("Déjà"), "d-j"); // non-ascii dropped, runs collapsed
    }

    #[test]
    fn reuse_existing_case_folds_leaf() {
        let existing = vec!["Photos.Travel".to_string()];
        // A client wrote `Photos/travel`; fold onto the existing-cased `Photos.Travel`.
        assert_eq!(
            reuse_existing_case("Photos.travel", &existing),
            "Photos.Travel"
        );
    }

    #[test]
    fn reuse_existing_case_folds_ancestor_and_leaf() {
        let existing = vec!["Photos.Travel".to_string()];
        // Both the ancestor (`photos`→`Photos`) and a brand-new deeper segment are handled:
        // the ancestor reuses existing casing, the new leaf keeps its given casing.
        assert_eq!(
            reuse_existing_case("photos.travel.Alps", &existing),
            "Photos.Travel.Alps"
        );
    }

    #[test]
    fn reuse_existing_case_no_match_is_identity() {
        let existing = vec!["Photos.Travel".to_string()];
        assert_eq!(
            reuse_existing_case("Images.Icons", &existing),
            "Images.Icons"
        );
        // Exact match is unchanged (no spurious rewrite).
        assert_eq!(
            reuse_existing_case("Photos.Travel", &existing),
            "Photos.Travel"
        );
    }

    #[test]
    fn reuse_existing_case_respects_distinct_parents() {
        // `a.X` exists but under parent `a`, not `b`; writing `b.x` must NOT borrow `a`'s casing.
        let existing = vec!["a.X".to_string()];
        assert_eq!(reuse_existing_case("b.x", &existing), "b.x");
    }

    #[test]
    fn rejects_duplicate_ids() {
        let json = serde_json::json!({"nodes": [
            {"id": "dup", "kind": "static", "name": "A", "children": []},
            {"id": "dup", "kind": "static", "name": "B", "children": []}
        ]});
        assert!(cfg(json).is_err());
    }

    #[test]
    fn rejects_collapsed_not_under_tag_root() {
        let json = serde_json::json!({"nodes": [
            {"id": "n1", "kind": "mirror", "tagRoot": "Photos", "collapsed": ["Images.Icons"]}
        ]});
        assert!(cfg(json).is_err());
    }

    #[test]
    fn accepts_collapsed_equal_to_tag_root() {
        let json = serde_json::json!({"nodes": [
            {"id": "n1", "kind": "mirror", "tagRoot": "Photos", "collapsed": ["Photos"]}
        ]});
        assert!(cfg(json).is_ok());
    }

    #[test]
    fn rejects_match_untagged_with_include() {
        let json = serde_json::json!({"nodes": [
            {"id": "n1", "kind": "query", "name": "U", "matchUntagged": true, "include": ["Photos"]}
        ]});
        assert!(cfg(json).is_err());
    }

    #[test]
    fn write_back_compliance_match_all_requires_all_includes() {
        // include [A, B] with match:all but onAdd only assigns A → invalid.
        let json = serde_json::json!({"nodes": [
            {"id": "n1", "kind": "query", "name": "Q", "match": "all", "include": ["A", "B"],
             "writeBack": {"onAdd": [{"op": "assign", "path": "A"}],
                           "onRemove": [{"op": "remove", "path": "A"}]}}
        ]});
        assert!(cfg(json).is_err());
    }

    #[test]
    fn write_back_compliance_match_any_one_include_suffices() {
        let json = serde_json::json!({"nodes": [
            {"id": "n1", "kind": "query", "name": "Q", "match": "any", "include": ["A", "B"],
             "writeBack": {"onAdd": [{"op": "assign", "path": "A"}],
                           "onRemove": [{"op": "remove", "path": "A"}, {"op": "remove", "path": "B"}]}}
        ]});
        assert!(cfg(json).is_ok());
    }

    #[test]
    fn write_back_compliance_must_clear_excludes_on_add() {
        // exclude [X] but onAdd does not remove X → invalid.
        let json = serde_json::json!({"nodes": [
            {"id": "n1", "kind": "query", "name": "Q", "match": "all", "include": ["A"], "exclude": ["X"],
             "writeBack": {"onAdd": [{"op": "assign", "path": "A"}],
                           "onRemove": [{"op": "remove", "path": "A"}]}}
        ]});
        assert!(cfg(json).is_err());
    }

    // ── feature 18: drop, tri-state write-back, foreign excludes, mirror depth ──────

    #[test]
    fn default_version_is_2() {
        let c: HierarchyConfig = serde_json::from_value(serde_json::json!({"nodes": []})).unwrap();
        assert_eq!(c.version, 2);
    }

    #[test]
    fn parses_drop_node() {
        let json = serde_json::json!({"nodes": [
            {"id": "d", "kind": "drop", "name": "Inbox",
             "onAdd": [{"op": "assign", "path": "Inbox"}]}
        ]});
        let c = cfg(json).unwrap();
        assert!(matches!(c.nodes[0].kind, NodeKind::Drop { .. }));
    }

    #[test]
    fn drop_requires_name_and_valid_on_add() {
        // Missing name → error (leaf still needs a directory label).
        let no_name = serde_json::json!({"nodes": [
            {"id": "d", "kind": "drop", "onAdd": [{"op": "assign", "path": "Inbox"}]}
        ]});
        assert!(cfg(no_name).is_err());
        // Invalid tag path in onAdd → error.
        let bad = serde_json::json!({"nodes": [
            {"id": "d", "kind": "drop", "name": "Inbox", "onAdd": [{"op": "assign", "path": "a b"}]}
        ]});
        assert!(cfg(bad).is_err());
    }

    #[test]
    fn untagged_query_may_have_write_back() {
        // Previously rejected; feature 18 §6 allows it (compliance skipped, free-form ops).
        let json = serde_json::json!({"nodes": [
            {"id": "u", "kind": "query", "name": "Inbox", "matchUntagged": true,
             "writeBack": {"onAdd": [{"op": "remove", "path": "Inbox"}],
                           "onRemove": [{"op": "assign", "path": "Filed"}]}}
        ]});
        assert!(cfg(json).is_ok());
    }

    #[test]
    fn mirror_allows_foreign_exclude() {
        let json = serde_json::json!({"nodes": [
            {"id": "m", "kind": "mirror", "tagRoot": "Photos",
             "exclude": ["Private", "Photos.Outdoor"]}
        ]});
        assert!(cfg(json).is_ok());
    }

    #[test]
    fn mirror_still_rejects_foreign_collapsed() {
        let json = serde_json::json!({"nodes": [
            {"id": "m", "kind": "mirror", "tagRoot": "Photos", "collapsed": ["Private"]}
        ]});
        assert!(cfg(json).is_err());
    }

    #[test]
    fn mirror_parses_max_depth_and_deeper_mode() {
        let json = serde_json::json!({"nodes": [
            {"id": "m", "kind": "mirror", "tagRoot": "Photos", "maxDepth": 2, "deeperMode": "exclude"}
        ]});
        let c = cfg(json).unwrap();
        match &c.nodes[0].kind {
            NodeKind::Mirror {
                max_depth,
                deeper_mode,
                ..
            } => {
                assert_eq!(*max_depth, 2);
                assert_eq!(*deeper_mode, DeeperMode::Exclude);
            }
            _ => panic!("expected mirror"),
        }
    }

    #[test]
    fn effective_enabled_master_off_is_hard_ceiling() {
        let json = serde_json::json!({
            "writeBack": false,
            "nodes": [
                {"id": "a", "kind": "static", "name": "A", "writeBackEnabled": true, "children": [
                    {"id": "b", "kind": "query", "name": "B", "include": ["X"], "writeBackEnabled": true}
                ]}
            ]
        });
        let c = cfg(json).unwrap();
        assert!(!c.effective_enabled("a"));
        assert!(!c.effective_enabled("b"));
    }

    #[test]
    fn effective_enabled_nearest_ancestor_wins() {
        let json = serde_json::json!({
            "writeBack": true,
            "nodes": [
                {"id": "root_off", "kind": "static", "name": "Off", "writeBackEnabled": false,
                 "children": [
                    {"id": "mid", "kind": "static", "name": "Mid", "children": [
                        {"id": "leaf_on", "kind": "query", "name": "On", "include": ["X"],
                         "writeBackEnabled": true},
                        {"id": "leaf_inherit", "kind": "query", "name": "Inh", "include": ["Y"]}
                    ]}
                ]}
            ]
        });
        let c = cfg(json).unwrap();
        assert!(!c.effective_enabled("mid")); // inherits the off ancestor
        assert!(c.effective_enabled("leaf_on")); // explicit override
        assert!(!c.effective_enabled("leaf_inherit")); // inherits off
    }

    #[test]
    fn effective_enabled_root_seed_true() {
        let json = serde_json::json!({
            "writeBack": true,
            "nodes": [{"id": "q", "kind": "query", "name": "Q", "include": ["X"]}]
        });
        let c = cfg(json).unwrap();
        assert!(c.effective_enabled("q"));
    }

    #[test]
    fn write_back_compliance_exclude_assign_breaks_on_remove() {
        // No include; exclude [X]; onRemove assigns X (breaks membership) → valid.
        let json = serde_json::json!({"nodes": [
            {"id": "n1", "kind": "query", "name": "Q", "match": "all", "include": [], "exclude": ["X"],
             "writeBack": {"onAdd": [{"op": "remove", "path": "X"}],
                           "onRemove": [{"op": "assign", "path": "X"}]}}
        ]});
        assert!(cfg(json).is_ok());
    }
}
