use crate::domain::tag::TagPath;
use crate::domain::tagging::ServiceType;
use crate::infra::routine::RoutineHandle;
use crate::repository::hierarchy::HierarchyRepository;
use crate::repository::picture::{PictureRepository, ResolvedSelection};
use crate::repository::pipeline::PipelineRepository;
use crate::repository::share::OutgoingShareRepository;
use crate::repository::tag::TagRepository;
use crate::repository::tagging::TaggingServiceRepository;
use crate::services::aggregate::DryRun;
use archypix_common::error::{map_sqlx_error, AppError};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// Result of a batch tag edit: either the dry-run breakdown or the applied count.
pub enum TagBatchOutcome {
    DryRun(DryRun),
    Applied { affected: i64 },
}

/// Add/remove tags across a [`ResolvedSelection`] (feature 14 §6.4). Removal only affects `manual`
/// rows (so the removable count reflects `manual_count`, not `count`). With `dry_run` the call
/// computes the §6.1 breakdown without mutating; otherwise it resolves the set inside the
/// transaction, applies remove-then-add atomically, invalidates the pipeline, and wakes it.
#[tracing::instrument(skip(db, waker, sel, add_tags, remove_tags), fields(user_id = %user_id, dry_run))]
pub async fn batch_edit_tags(
    db: &PgPool,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    sel: &ResolvedSelection,
    add_tags: &[String],
    remove_tags: &[String],
    dry_run: bool,
) -> Result<TagBatchOutcome, AppError> {
    if add_tags.is_empty() && remove_tags.is_empty() {
        return Err(AppError::BadRequest(
            "at least one of add_tags or remove_tags must be non-empty".to_string(),
        ));
    }
    // A selection that names neither a query nor an explicit picture can never match anything; reject
    // it like the legacy empty-`picture_ids` guard. (A query that *resolves* to zero rows is still a
    // valid no-op — its `filter` is `Some`, so `is_empty()` is false.)
    if sel.is_empty() {
        return Err(AppError::BadRequest(
            "selection is empty: provide a query or at least one picture".to_string(),
        ));
    }

    if dry_run {
        let affected = PictureRepository::count_selection(db, user_id, sel).await?;
        let removed = if remove_tags.is_empty() {
            None
        } else {
            Some(
                TagRepository::count_selection_with_manual_under(db, user_id, sel, remove_tags)
                    .await?,
            )
        };
        return Ok(TagBatchOutcome::DryRun(DryRun {
            affected,
            added: (!add_tags.is_empty()).then_some(affected),
            removed,
            ..Default::default()
        }));
    }

    let mut tx = db.begin().await.map_err(map_sqlx_error)?;
    let ids = PictureRepository::resolve_selection_ids(&mut *tx, user_id, sel).await?;
    if ids.is_empty() {
        tx.commit().await.map_err(map_sqlx_error)?;
        return Ok(TagBatchOutcome::Applied { affected: 0 });
    }
    // batch_remove/batch_assign re-dirty the affected pictures intrinsically
    TagRepository::batch_remove(&mut *tx, user_id, &ids, remove_tags).await?;
    TagRepository::batch_assign(&mut *tx, user_id, &ids, add_tags).await?;
    tx.commit().await.map_err(map_sqlx_error)?;
    waker.trigger(user_id);
    Ok(TagBatchOutcome::Applied {
        affected: ids.len() as i64,
    })
}

/// What a tag-rename cascade touched. `needs_pipeline_wake` gates the pipeline trigger.
#[derive(Debug, Default, Clone, Copy)]
pub struct RenameOutcome {
    pub tags_renamed: u64,
    pub services_changed: usize,
    pub shares_renamed: u64,
    pub hierarchies_changed: usize,
    pub pictures_invalidated: u64,
}

impl RenameOutcome {
    /// Wake the pipeline when anything the pipeline acts on changed: dirty pictures (re-tag +
    /// re-announce), a changed service (re-derive), or a renamed share (re-announce).
    pub fn needs_pipeline_wake(&self) -> bool {
        self.pictures_invalidated > 0 || self.services_changed > 0 || self.shares_renamed > 0
    }
}

/// Rename a tag subtree across everything a user owns (edge case §7, "Tag rename cascade").
///
/// A real search-and-replace: manual tags, outgoing-share tags, tagging-service gates + config
/// (including SharedTagMapping), and hierarchy configs all have their `old` prefix swapped for
/// `new`. Changed services are invalidated; every picture carrying a tag under `old`/`new` is marked
/// dirty so the pipeline re-derives service tags and re-announces shares under the renamed tag. The
/// share tracking table is left untouched — re-announcement rides the picture `updated_at` bump, so
/// any pending announce/unannounce delta survives. Runs in one transaction.
#[tracing::instrument(skip(db), fields(user_id = %user_id, old = %old, new = %new))]
pub async fn cascade_rename(
    db: &PgPool,
    user_id: Uuid,
    old: &TagPath,
    new: &TagPath,
) -> Result<RenameOutcome, AppError> {
    let old_ltree = old.as_ltree();
    let new_ltree = new.as_ltree();
    if old == new {
        return Ok(RenameOutcome::default());
    }
    // A rename into the subtree being renamed (or vice-versa) would fold a path onto itself — reject.
    if old.is_ancestor_of(new) || new.is_ancestor_of(old) {
        return Err(AppError::BadRequest(
            "cannot rename a tag into its own ancestor or descendant".to_string(),
        ));
    }

    let mut tx = db.begin().await.map_err(map_sqlx_error)?;
    let mut outcome = RenameOutcome::default();

    // ── Manual picture tags ────────────────────────────────────────────────────
    outcome.tags_renamed =
        TagRepository::rename_manual_subtree(&mut *tx, user_id, old_ltree, new_ltree).await?;

    // ── Outgoing shares ────────────────────────────────────────────────────────
    outcome.shares_renamed =
        OutgoingShareRepository::rename_tag_subtree(&mut *tx, user_id, old_ltree, new_ltree)
            .await?;

    // ── Tagging services: gates (requires/excludes) + type-specific config ──────
    for svc in TaggingServiceRepository::list_by_owner(&mut *tx, user_id).await? {
        let (requires, req_changed) = rename_paths(&svc.requires, old, new);
        let (excludes, exc_changed) = rename_paths(&svc.excludes, old, new);
        let cfg_changed = rename_service_config(svc.service_type, &svc.config, old, new);
        if !req_changed && !exc_changed && cfg_changed.is_none() {
            continue;
        }
        let config = cfg_changed.unwrap_or(svc.config);
        TaggingServiceRepository::replace_gating_and_config(
            &mut *tx, svc.id, &requires, &excludes, &config,
        )
        .await?;
        outcome.services_changed += 1;
    }

    // ── Hierarchy configs (read-time WebDAV views; no pipeline work) ────────────
    for h in HierarchyRepository::list_by_owner(&mut *tx, user_id).await? {
        let mut config = h.config.clone();
        if rename_hierarchy_config(&mut config, old, new) {
            HierarchyRepository::update(&mut *tx, user_id, h.id, None, None, Some(&config)).await?;
            outcome.hierarchies_changed += 1;
        }
    }

    // ── Re-derive + re-announce: dirty every picture under the old or new subtree ─
    // `old` catches not-yet-renamed service-derived tags; `new` catches the renamed manual tags.
    // Marking dirty bumps `updated_at` (trigger), which re-announces covered shared pictures.
    outcome.pictures_invalidated = PipelineRepository::invalidate_under_tags(
        &mut *tx,
        user_id,
        &[old_ltree.to_string(), new_ltree.to_string()],
    )
    .await?;

    tx.commit().await.map_err(map_sqlx_error)?;
    Ok(outcome)
}

/// Rename each ltree path in `paths` that sits under `old`; returns the new list and whether any
/// element changed. Order is preserved.
fn rename_paths(paths: &[String], old: &TagPath, new: &TagPath) -> (Vec<String>, bool) {
    let mut changed = false;
    let out = paths
        .iter()
        .map(
            |p| match TagPath::from_ltree(p.clone()).rename_under(old, new) {
                Some(renamed) => {
                    changed = true;
                    renamed.as_ltree().to_string()
                }
                None => p.clone(),
            },
        )
        .collect();
    (out, changed)
}

/// Rename the tag references inside a service's type-specific config. Returns the rewritten config
/// only when something changed. Rule → `rules[].assign_tag`; SharedTagMapping → `assign_tags[]`;
/// Segmentation → `root_tag`.
fn rename_service_config(
    service_type: ServiceType,
    config: &Value,
    old: &TagPath,
    new: &TagPath,
) -> Option<Value> {
    let mut cfg = config.clone();
    let changed = match service_type {
        ServiceType::Rule => {
            let mut any = false;
            if let Some(rules) = cfg.get_mut("rules").and_then(Value::as_array_mut) {
                for rule in rules {
                    if let Some(t) = rule.get_mut("assign_tag") {
                        any |= rename_json_str(t, old, new);
                    }
                }
            }
            any
        }
        ServiceType::SharedTagMapping => {
            let mut any = false;
            if let Some(tags) = cfg.get_mut("assign_tags").and_then(Value::as_array_mut) {
                for t in tags {
                    any |= rename_json_str(t, old, new);
                }
            }
            any
        }
        ServiceType::Segmentation => cfg
            .get_mut("root_tag")
            .map(|t| rename_json_str(t, old, new))
            .unwrap_or(false),
    };
    changed.then_some(cfg)
}

/// Rename every tag reference in a hierarchy config (feature 05/18). Walks the node tree and
/// rewrites the tag-bearing fields — `tagRoot`, `collapsed`/`exclude`/`include` lists, and the
/// `path` of each `on_add`/`on_remove` [`TagOp`]. Returns whether anything changed.
fn rename_hierarchy_config(config: &mut Value, old: &TagPath, new: &TagPath) -> bool {
    match config {
        Value::Object(map) => {
            let mut changed = false;
            if let Some(t) = map.get_mut("tagRoot") {
                changed |= rename_json_str(t, old, new);
            }
            for key in ["collapsed", "exclude", "include"] {
                if let Some(arr) = map.get_mut(key).and_then(Value::as_array_mut) {
                    for elem in arr {
                        changed |= rename_json_str(elem, old, new);
                    }
                }
            }
            // A `TagOp` is `{ op, path }`; only rename `path` when the sibling `op` marks it as one.
            if map.contains_key("op") {
                if let Some(t) = map.get_mut("path") {
                    changed |= rename_json_str(t, old, new);
                }
            }
            for child in map.values_mut() {
                changed |= rename_hierarchy_config(child, old, new);
            }
            changed
        }
        Value::Array(arr) => {
            let mut changed = false;
            for elem in arr {
                changed |= rename_hierarchy_config(elem, old, new);
            }
            changed
        }
        _ => false,
    }
}

/// Rename a JSON string value in place if it holds a tag path under `old`. Returns whether it changed.
fn rename_json_str(value: &mut Value, old: &TagPath, new: &TagPath) -> bool {
    let Some(s) = value.as_str() else {
        return false;
    };
    match TagPath::from_ltree(s.to_string()).rename_under(old, new) {
        Some(renamed) => {
            *value = Value::String(renamed.as_ltree().to_string());
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod rename_tests {
    use super::*;
    use serde_json::json;

    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

    async fn seed_user(db: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO users (id, username, email, display_name) VALUES ($1, $2, $3, $4)",
            id,
            format!("u_{}", &id.to_string()[..8]),
            format!("{id}@t.com"),
            "T",
        )
        .execute(db)
        .await
        .unwrap();
        id
    }

    async fn seed_picture(db: &PgPool, user_id: Uuid, manual_tag: &str) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO pictures (id, local_user_id, last_pipeline_run_at)
             VALUES ($1, $2, now() AT TIME ZONE 'utc')",
            id,
            user_id,
        )
        .execute(db)
        .await
        .unwrap();
        sqlx::query!(
            "INSERT INTO tags (picture_id, tag_path, source) VALUES ($1, $2::text::ltree, 'manual')",
            id,
            manual_tag,
        )
            .execute(db)
            .await
            .unwrap();
        id
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn cascade_rename_rewrites_everywhere(db: PgPool) {
        let user = seed_user(&db).await;
        let pic = seed_picture(&db, user, "Photos.Travel.Alps").await;
        // Unrelated tag on another picture must survive untouched.
        let other = seed_picture(&db, user, "Images.Icons").await;

        let share = OutgoingShareRepository::create(
            &db,
            user,
            "Photos.Travel",
            "S",
            None,
            "bob",
            "other.com",
            true,
            false,
            true,
            None,
        )
        .await
        .unwrap();

        let svc = TaggingServiceRepository::create(
            &db,
            user,
            ServiceType::Rule,
            "R",
            &["Photos.Travel".to_string()],
            &[],
            &json!({"rules": [{"id": Uuid::new_v4(), "predicate": {"and": []}, "assign_tag": "Photos.Travel.Auto"}]}),
        )
            .await
            .unwrap();

        let hier = HierarchyRepository::create(
            &db,
            user,
            "H",
            &json!({"version": 2, "nodes": [
                {"id": "m", "kind": "mirror", "tagRoot": "Photos.Travel", "exclude": ["Photos.Travel.Private"]}
            ]}),
        )
            .await
            .unwrap();

        let old = TagPath::from_ltree("Photos.Travel");
        let new = TagPath::from_ltree("Trips.2024");
        let outcome = cascade_rename(&db, user, &old, &new).await.unwrap();

        assert_eq!(outcome.tags_renamed, 1);
        assert_eq!(outcome.shares_renamed, 1);
        assert_eq!(outcome.services_changed, 1);
        assert_eq!(outcome.hierarchies_changed, 1);
        assert!(outcome.needs_pipeline_wake());

        // Manual tag renamed, unrelated tag untouched.
        let paths = TagRepository::list_manual_paths(&db, pic).await.unwrap();
        assert_eq!(paths, vec!["Trips.2024.Alps".to_string()]);
        assert_eq!(
            TagRepository::list_manual_paths(&db, other).await.unwrap(),
            vec!["Images.Icons".to_string()]
        );

        // Share tag renamed.
        let share = OutgoingShareRepository::get_by_id(&db, share.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(share.tag_path, "Trips.2024");

        // Service gate + config renamed and invalidated.
        let svc = TaggingServiceRepository::get_by_owner_and_id(&db, user, svc.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(svc.requires, vec!["Trips.2024".to_string()]);
        assert_eq!(svc.config["rules"][0]["assign_tag"], "Trips.2024.Auto");

        // Hierarchy config renamed (tagRoot + exclude).
        let hier = HierarchyRepository::get_by_owner_and_id(&db, user, hier.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(hier.config["nodes"][0]["tagRoot"], "Trips.2024");
        assert_eq!(hier.config["nodes"][0]["exclude"][0], "Trips.2024.Private");

        // Covered picture marked dirty for re-tag + re-announce.
        let dirty: bool = sqlx::query_scalar!(
            r#"SELECT (last_pipeline_run_at IS NULL) AS "d!" FROM pictures WHERE id = $1"#,
            pic,
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert!(dirty);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn cascade_rename_merges_colliding_manual_tag(db: PgPool) {
        let user = seed_user(&db).await;
        let pic = seed_picture(&db, user, "Photos.Travel").await;
        // Same picture already carries the rename target — the source row must be dropped, not error.
        sqlx::query!(
            "INSERT INTO tags (picture_id, tag_path, source) VALUES ($1, 'Photos.Vacation'::ltree, 'manual')",
            pic,
        )
            .execute(&db)
            .await
            .unwrap();

        let outcome = cascade_rename(
            &db,
            user,
            &TagPath::from_ltree("Photos.Travel"),
            &TagPath::from_ltree("Photos.Vacation"),
        )
        .await
        .unwrap();
        assert_eq!(outcome.tags_renamed, 0, "collision dropped the source row");

        let paths = TagRepository::list_manual_paths(&db, pic).await.unwrap();
        assert_eq!(paths, vec!["Photos.Vacation".to_string()]);
    }
}
