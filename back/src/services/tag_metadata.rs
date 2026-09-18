//! Tag metadata (feature 34): the enriched tag-tree read path with its cache, and the partial
//! upsert / delete writes.
//!
//! The metadata is decorative — nothing in the engine reads it (§2). WebDAV is the one deliberate
//! exception ([`crate::services::vfs`], §8).

use crate::domain::tag::TagSource;
use crate::domain::tag_metadata::{
    SharedTagMeta, TagMetadata, TagMetadataPatch, validate_metadata_path,
};
use crate::infra::redis::{Cache, RedisKey, cache_get_json, cache_set_json_ex};
use crate::repository::picture::PictureRepository;
use crate::repository::tag::{TagCounts, TagRepository};
use crate::repository::tag_metadata::TagMetadataRepository;
use archypix_common::error::{AppError, map_sqlx_error};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

/// The tag tree is *already* eventually consistent — the pipeline is async, so users experience
/// convergence rather than immediacy. A row-level trigger on `tags` would fire per row during bulk
/// pipeline runs; a TTL plus the explicit busts in [`bust_cache`]'s callers is the cheaper trade.
const TREE_TTL_SECS: u64 = 60;

/// One entry of `GET /tags` (§11). The root (`path = ""`) carries only its `meta`; its counts are
/// zero and meaningless.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagListItem {
    pub path: String,
    #[serde(flatten)]
    pub live: TagCounts,
    /// Omitted when the tag has no trashed pictures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trashed: Option<TagCounts>,
    /// Only with `with_sources=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<SourceCount>>,
    pub meta: Option<TagMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceCount {
    pub source: TagSource,
    pub count: i64,
}

// ── Read path ─────────────────────────────────────────────────────────────────

/// The whole browse read path (§4): every path the user has, ancestor-expanded, with live and
/// trashed counts, derived date ranges and the metadata row. Served from `tags:tree:{user_id}`.
///
/// `with_sources` adds the path×source provenance — too heavy for the app-start payload, so it is
/// computed outside the cache and never stored in it.
#[tracing::instrument(skip(db, cache), fields(user_id = %user_id, with_sources))]
pub async fn list_tree(
    db: &PgPool,
    cache: &dyn Cache,
    user_id: Uuid,
    with_sources: bool,
) -> Result<Vec<TagListItem>, AppError> {
    let key = RedisKey::TagTree(user_id);
    let mut items = match cache_get_json::<Vec<TagListItem>>(cache, key).await {
        Ok(Some(hit)) => hit,
        // A cache failure must not fail a browse — fall through to the query.
        Ok(None) | Err(_) => {
            let fresh = build_tree(db, user_id).await?;
            let _ = cache_set_json_ex(cache, key, &fresh, TREE_TTL_SECS).await;
            fresh
        }
    };

    if with_sources {
        let mut by_path: HashMap<String, Vec<SourceCount>> = HashMap::new();
        for (path, source, count) in TagRepository::list_sources_by_user(db, user_id).await? {
            by_path
                .entry(path)
                .or_default()
                .push(SourceCount { source, count });
        }
        for item in &mut items {
            item.sources = Some(by_path.remove(&item.path).unwrap_or_default());
        }
    }
    Ok(items)
}

/// Join the enriched aggregates to the metadata rows, adding the paths that exist only because of
/// metadata (`show_when_empty`, §2) and the root row (§3.3).
async fn build_tree(db: &PgPool, user_id: Uuid) -> Result<Vec<TagListItem>, AppError> {
    let aggregates = TagRepository::list_tags_enriched(db, user_id).await?;
    let mut meta: HashMap<String, TagMetadata> = TagMetadataRepository::list_for_user(db, user_id)
        .await?
        .into_iter()
        .map(|m| (m.tag_path.clone(), m))
        .collect();

    let mut seen: HashSet<String> = aggregates.iter().map(|a| a.path.clone()).collect();
    let mut items: Vec<TagListItem> = aggregates
        .into_iter()
        .map(|a| TagListItem {
            path: a.path.clone(),
            live: a.live,
            trashed: (!a.trashed.is_zero()).then_some(a.trashed),
            sources: None,
            meta: meta.remove(&a.path),
        })
        .collect();

    // A deliberately-created empty tag has no `tags` row at all, so it (and any ancestor that is
    // itself empty) must be synthesized or the node would be unreachable in the tree.
    let empty_paths: Vec<String> = meta
        .values()
        .filter(|m| m.show_when_empty && !m.tag_path.is_empty())
        .map(|m| m.tag_path.clone())
        .collect();
    for path in empty_paths {
        for node in crate::domain::tag::TagPath::from_ltree(path.clone())
            .ancestors()
            .into_iter()
            .map(|a| a.as_ltree().to_string())
            .chain(std::iter::once(path))
        {
            if seen.insert(node.clone()) {
                let meta_row = meta.remove(&node);
                items.push(TagListItem {
                    path: node,
                    live: TagCounts::default(),
                    trashed: None,
                    sources: None,
                    meta: meta_row,
                });
            }
        }
    }

    // The root is always returned so the client never has to invent it (§13.10 — nothing depends
    // on the row existing).
    items.push(TagListItem {
        path: String::new(),
        live: TagCounts::default(),
        trashed: None,
        sources: None,
        meta: meta.remove(""),
    });
    items.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(items)
}

/// Drop the cached tree. Called at the end of a pipeline run and by every synchronous writer that
/// changes the tag set, a capture date or a metadata row (§4). Best-effort: a cache failure is not
/// worth failing the write over, the TTL covers it.
pub async fn bust_cache(cache: &dyn Cache, user_id: Uuid) {
    let _ = cache.del(RedisKey::TagTree(user_id)).await;
}

// ── Write path ────────────────────────────────────────────────────────────────

/// Apply a coalesced batch of partial upserts (§4.1). Each item is merged onto the stored row (or a
/// fresh default one) and validated as a whole; a merged row that is all-default is **deleted**
/// instead of written, so browsing with default view settings never litters the table (§2).
///
/// Returns the rows that survived the prune, so the caller can echo the resulting state.
#[tracing::instrument(skip(db, cache, patches), fields(user_id = %user_id, count = patches.len()))]
pub async fn upsert(
    db: &PgPool,
    cache: &dyn Cache,
    user_id: Uuid,
    patches: Vec<TagMetadataPatch>,
) -> Result<Vec<TagMetadata>, AppError> {
    if patches.is_empty() {
        return Ok(vec![]);
    }
    let mut normalized = Vec::with_capacity(patches.len());
    for mut p in patches {
        p.tag_path = validate_metadata_path(&p.tag_path).map_err(AppError::BadRequest)?;
        normalized.push(p);
    }
    let mut paths: Vec<String> = normalized.iter().map(|p| p.tag_path.clone()).collect();
    paths.sort();
    paths.dedup();

    let covers: Vec<Uuid> = normalized
        .iter()
        .filter_map(|p| p.cover_picture_id.flatten())
        .collect();
    let owned = PictureRepository::filter_owned_ids(db, user_id, &covers).await?;
    if let Some(bad) = covers.iter().find(|c| !owned.contains(c)) {
        return Err(AppError::BadRequest(format!(
            "cover_picture_id {bad} does not belong to you"
        )));
    }

    let mut tx = db.begin().await.map_err(map_sqlx_error)?;
    let mut stored: HashMap<String, TagMetadata> =
        TagMetadataRepository::find_many(&mut *tx, user_id, &paths)
            .await?
            .into_iter()
            .map(|m| (m.tag_path.clone(), m))
            .collect();

    // A coalesced flush should not carry the same tag twice, but a client bug must not turn into a
    // duplicate-key upsert: fold repeats onto each other, last value wins per field.
    let mut merged: BTreeMap<String, TagMetadata> = BTreeMap::new();
    for patch in normalized {
        let path = patch.tag_path.clone();
        let base = merged
            .remove(&path)
            .or_else(|| stored.remove(&path))
            .unwrap_or_else(|| TagMetadata::new(path.clone()));
        merged.insert(path, patch.apply(base).map_err(AppError::BadRequest)?);
    }
    let (writes, pruned): (Vec<_>, Vec<_>) = merged
        .into_values()
        .partition(|m| !m.is_all_default());
    let prunes: Vec<String> = pruned.into_iter().map(|m| m.tag_path).collect();

    TagMetadataRepository::delete_many(&mut *tx, user_id, &prunes).await?;
    TagMetadataRepository::upsert_many(&mut *tx, user_id, &writes).await?;
    tx.commit().await.map_err(map_sqlx_error)?;

    bust_cache(cache, user_id).await;
    Ok(writes)
}

/// *Reset metadata* (§6): drop the rows, keep the tags.
#[tracing::instrument(skip(db, cache, paths), fields(user_id = %user_id, count = paths.len()))]
pub async fn delete(
    db: &PgPool,
    cache: &dyn Cache,
    user_id: Uuid,
    paths: &[String],
) -> Result<u64, AppError> {
    let paths: Vec<String> = paths
        .iter()
        .map(|p| validate_metadata_path(p).map_err(AppError::BadRequest))
        .collect::<Result<_, _>>()?;
    let deleted = TagMetadataRepository::delete_many(db, user_id, &paths).await?;
    bust_cache(cache, user_id).await;
    Ok(deleted)
}

/// Read an owner's decoration for one tag so it can be announced or rendered on a landing page
/// (§10.1) — [`TagMetadata::shared`] picks the travelling subset.
#[tracing::instrument(skip(db), fields(user_id = %user_id, tag_path))]
pub async fn shared_meta_for(
    db: &PgPool,
    user_id: Uuid,
    tag_path: &str,
) -> Result<Option<SharedTagMeta>, AppError> {
    let meta = TagMetadataRepository::find_many(db, user_id, &[tag_path.to_string()])
        .await?
        .first()
        .map(TagMetadata::shared);
    Ok(meta.filter(|m| !m.is_empty()))
}

/// Seed a recipient's `SharedToMe.<sender>.<subpath>` row from the sender's announced decoration
/// (§10.1). **Seed once, never overwrite**: without this rule a sender tidying their own label
/// would silently clobber a recipient who had renamed it, and a recipient who reset their metadata
/// would get it back.
///
/// The cover travels as the owner's picture id and is resolved through `pictures.remote_picture_id`;
/// if the picture is not (or not yet) in the share it is dropped and the frontend's
/// first-loaded-photo fallback applies.
#[tracing::instrument(skip(db, cache, meta), fields(user_id = %user_id, tag_path))]
pub async fn seed_once(
    db: &PgPool,
    cache: &dyn Cache,
    user_id: Uuid,
    tag_path: &str,
    meta: &SharedTagMeta,
) -> Result<bool, AppError> {
    if meta.is_empty() || tag_path.is_empty() {
        return Ok(false);
    }
    if !TagMetadataRepository::find_many(db, user_id, &[tag_path.to_string()])
        .await?
        .is_empty()
    {
        return Ok(false);
    }
    let cover = match meta.cover_remote_picture_id {
        Some(remote) => PictureRepository::find_ids_by_remote_ids(db, user_id, &[remote.to_string()])
            .await?
            .into_iter()
            .next(),
        None => None,
    };
    let row = TagMetadata {
        display_name: meta.display_name.clone(),
        description: meta.description.clone(),
        color: meta.color.clone(),
        cover_picture_id: cover,
        ..TagMetadata::new(tag_path.to_string())
    };
    if row.is_all_default() {
        return Ok(false);
    }
    TagMetadataRepository::upsert_many(db, user_id, &[row]).await?;
    bust_cache(cache, user_id).await;
    Ok(true)
}
