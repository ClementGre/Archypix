//! The feature 14 **selection descriptor** and **homogenized picture filter** (§2–§3).
//!
//! Every batch endpoint (aggregate, tags, EXIF, trash/restore) speaks one selection language: a
//! `query` (a [`PictureFilter`]) plus explicit `include_ids` / `exclude_ids` deltas. The effective
//! set is `(resolve(query) ∪ include_ids) \ exclude_ids`, always scoped server-side to the caller's
//! own holdings. Both the flat gallery and a hierarchy directory resolve to the same
//! [`TagPredicate`]-backed [`PictureListFilter`], so the model works identically across views.
//!
//! Resolution happens at **apply time** (no point-in-time pinning, §2.1): `Ctrl+A` means "everything
//! this query matches now". The mandatory confirmation popup's dry-run re-resolves through the same
//! path, so the previewed count can never diverge from the apply.

use crate::domain::hierarchy::TagPredicate;
use crate::domain::tag::TagPath;
use crate::infra::error::AppError;
use crate::repository::picture::{
    PictureListFilter, PictureSortField, ResolvedSelection, SortOrder,
};
use crate::services::hierarchy;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

/// Scope/date params shared by both filter kinds (§3). Mirror the read-side `GET /pictures` scope.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScopeParams {
    #[serde(default)]
    pub owned_only: bool,
    #[serde(default)]
    pub shared_with_me: bool,
    #[serde(default)]
    pub include_deleted: bool,
    pub captured_after: Option<DateTime<Utc>>,
    pub captured_before: Option<DateTime<Utc>>,
}

/// The flat gallery filter — the `GET /pictures` tag-set params. Tag lists are arrays here (JSON
/// body) rather than the comma-strings the query-string list endpoint uses.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FlatFilter {
    #[serde(default)]
    pub include_tags: Vec<String>,
    #[serde(default)]
    pub exclude_tags: Vec<String>,
    /// Exactly-matched ltree paths
    #[serde(default)]
    pub exact: Vec<String>,
    /// `all` (AND) | `any` (OR) over `include_tags`. Default `all`.
    #[serde(rename = "match")]
    pub match_mode: Option<String>,
    #[serde(default)]
    pub untagged: bool,
    #[serde(flatten)]
    pub scope: ScopeParams,
}

/// A hierarchy directory filter — `hierarchy_id` + slash-delimited directory `path`, resolved
/// server-side to the directory's "most-specific node wins" direct predicate (§3), AND-ed with the
/// shared scope/date params.
#[derive(Debug, Clone, Deserialize)]
pub struct HierarchyFilter {
    pub hierarchy_id: Uuid,
    #[serde(default)]
    pub path: String,
    #[serde(flatten)]
    pub scope: ScopeParams,
}

/// The homogenized picture filter (§3): the flat gallery or a hierarchy directory.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PictureFilter {
    Flat(FlatFilter),
    Hierarchy(HierarchyFilter),
}

/// The selection descriptor (§2). `query == null` ⇒ pure explicit set (`include_ids`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PictureSelection {
    #[serde(default)]
    pub query: Option<PictureFilter>,
    #[serde(default)]
    pub include_ids: Vec<Uuid>,
    #[serde(default)]
    pub exclude_ids: Vec<Uuid>,
}

/// Parse a `match` mode string into the `match_all` flag.
fn match_all_of(mode: Option<&str>) -> Result<bool, AppError> {
    match mode {
        None | Some("all") => Ok(true),
        Some("any") => Ok(false),
        Some(other) => Err(AppError::BadRequest(format!(
            "invalid match mode {other:?} (expected \"all\" or \"any\")"
        ))),
    }
}

/// Parse a wire-form ltree tag path, allowing the protected `SharedToMe` prefix (filtering is
/// read-only).
fn parse_path(s: &str) -> Result<TagPath, AppError> {
    TagPath::parse(s, true).map_err(AppError::BadRequest)
}

/// Build the [`PictureListFilter`] for a flat filter. Membership uses the predicate; pagination/sort
/// fields are irrelevant to a set and left at defaults.
fn flat_to_filter(f: &FlatFilter) -> Result<PictureListFilter, AppError> {
    let mut include: Vec<TagPath> = Vec::new();
    for t in &f.include_tags {
        if !t.trim().is_empty() {
            include.push(parse_path(t)?);
        }
    }
    let exclude: Vec<TagPath> = f
        .exclude_tags
        .iter()
        .filter(|s| !s.trim().is_empty())
        .map(|s| parse_path(s))
        .collect::<Result<_, _>>()?;
    let exact: Vec<TagPath> = f
        .exact
        .iter()
        .filter(|s| !s.trim().is_empty())
        .map(|s| parse_path(s))
        .collect::<Result<_, _>>()?;

    if f.untagged && (!include.is_empty() || !exclude.is_empty() || !exact.is_empty()) {
        return Err(AppError::BadRequest(
            "untagged is mutually exclusive with include_tags/exclude_tags/exact".to_string(),
        ));
    }

    // No tag arms and not untagged ⇒ a pure scope filter (still a real predicate so the scope
    // flags below are honoured). The predicate renders to TRUE; scope filtering happens via the
    // dedicated filter columns.
    let predicate = Some(TagPredicate {
        include,
        match_all: match_all_of(f.match_mode.as_deref())?,
        exclude,
        exact,
        untagged: f.untagged,
        ..TagPredicate::all()
    });

    Ok(filter_from(predicate, &f.scope))
}

/// Resolve a hierarchy directory to its direct-files predicate, returning `None` when the directory
/// has no direct files (`static`/root) — equivalent to a query that matches nothing.
async fn hierarchy_to_filter(
    db: &PgPool,
    user_id: Uuid,
    h: &HierarchyFilter,
) -> Result<Option<PictureListFilter>, AppError> {
    let (_, _config, root) = hierarchy::load_resolved(db, user_id, h.hierarchy_id).await?;
    let segments = hierarchy::split_path(&h.path);
    let target = hierarchy::find_dir(&root, &segments).ok_or(AppError::NotFound)?;
    Ok(target
        .direct
        .clone()
        .map(|pred| filter_from(Some(pred), &h.scope)))
}

/// Assemble a [`PictureListFilter`] from a predicate and the shared scope params.
fn filter_from(predicate: Option<TagPredicate>, scope: &ScopeParams) -> PictureListFilter {
    PictureListFilter {
        page: 1,
        page_size: 1,
        sort: PictureSortField::default(),
        order: SortOrder::default(),
        predicate,
        owned_only: scope.owned_only,
        shared_with_me: scope.shared_with_me,
        include_deleted: scope.include_deleted,
        captured_after: scope.captured_after.map(|dt| dt.naive_utc()),
        captured_before: scope.captured_before.map(|dt| dt.naive_utc()),
    }
}

/// Resolve a [`PictureSelection`] into a [`ResolvedSelection`] (§2). The query is lowered to a
/// predicate; the id deltas are carried through verbatim. Scoping to the caller is applied by the
/// repository membership term, never trusting the wire.
#[tracing::instrument(skip(db, selection), fields(user_id = %user_id))]
pub async fn resolve(
    db: &PgPool,
    user_id: Uuid,
    selection: &PictureSelection,
) -> Result<ResolvedSelection, AppError> {
    let filter = match &selection.query {
        None => None,
        Some(PictureFilter::Flat(f)) => Some(flat_to_filter(f)?),
        Some(PictureFilter::Hierarchy(h)) => hierarchy_to_filter(db, user_id, h).await?,
    };
    Ok(ResolvedSelection {
        filter,
        include_ids: selection.include_ids.clone(),
        exclude_ids: selection.exclude_ids.clone(),
    })
}

/// Resolve a request that carries either a [`PictureSelection`] or a legacy explicit `picture_ids`
/// list. When `selection` is present it wins; otherwise the ids form a pure explicit set. Lets every
/// batch endpoint accept the new descriptor while staying back-compatible with id-list callers.
#[tracing::instrument(skip(db, selection, picture_ids), fields(user_id = %user_id))]
pub async fn resolve_or_explicit(
    db: &PgPool,
    user_id: Uuid,
    selection: Option<&PictureSelection>,
    picture_ids: Vec<Uuid>,
) -> Result<ResolvedSelection, AppError> {
    match selection {
        Some(sel) => resolve(db, user_id, sel).await,
        None => Ok(ResolvedSelection::explicit(picture_ids)),
    }
}
