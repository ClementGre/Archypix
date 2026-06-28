use crate::domain::tag::{Tag, TagSource};
use crate::infra::error::{AppError, map_sqlx_error};
use crate::repository::picture::{PictureRepository, ResolvedSelection};
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;

/// One ancestor-expanded tag aggregate over a selection (feature 14 §4.2). `count` is
/// ancestor-inclusive (a picture stored as `A.B.C` contributes to `A`, `A.B`, `A.B.C`);
/// `count == total` ⇒ on every selected picture. `manual_count` counts pictures holding a *manual*
/// row under this path and drives the remove affordance. `sources` is populated only when
/// provenance is requested.
#[derive(Debug)]
pub struct TagAgg {
    pub path: String,
    pub count: i64,
    pub manual_count: i64,
    pub sources: Vec<(TagSource, i64)>,
}

pub struct TagRepository;

impl TagRepository {
    #[tracing::instrument(skip(ex), fields(user_id = %local_user_id))]
    pub async fn list_paths_by_user<'e, E>(
        ex: E,
        local_user_id: Uuid,
    ) -> Result<Vec<String>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_scalar!(
            r#"SELECT DISTINCT t.tag_path::text
               FROM tags t
               JOIN pictures p ON p.id = t.picture_id
               WHERE p.local_user_id = $1 AND p.deleted_at IS NULL"#,
            local_user_id
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
        .map(|rows| rows.into_iter().flatten().collect())
    }

    /// List tags for a specific picture owned by the given user.
    #[tracing::instrument(skip(ex), fields(user_id = %local_user_id, picture_id = %picture_id))]
    pub async fn list_for_picture<'e, E>(
        ex: E,
        local_user_id: Uuid,
        picture_id: Uuid,
    ) -> Result<Vec<Tag>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            Tag,
            r#"SELECT t.id, t.picture_id, t.tag_path::text as "tag_path!",
                      t.source as "source!: TagSource", t.source_id, t.assigned_at
               FROM tags t
               JOIN pictures p ON p.id = t.picture_id
               WHERE t.picture_id = $1 AND p.local_user_id = $2 AND p.deleted_at IS NULL"#,
            picture_id,
            local_user_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Load tags for a batch of pictures. Used by the pipeline loop to load current tags
    /// for all dirty pictures in one query rather than N per-picture queries.
    #[tracing::instrument(skip(ex, picture_ids))]
    pub async fn list_for_pictures<'e, E>(ex: E, picture_ids: &[Uuid]) -> Result<Vec<Tag>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if picture_ids.is_empty() {
            return Ok(vec![]);
        }
        sqlx::query_as!(
            Tag,
            r#"SELECT id, picture_id, tag_path::text as "tag_path!",
                      source as "source!: TagSource", source_id, assigned_at
               FROM tags
               WHERE picture_id = ANY($1::uuid[])"#,
            picture_ids as &[Uuid],
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Add tags to all pictures in the batch. All pictures must belong to `local_user_id`.
    ///
    /// Runs as a single data-modifying CTE:
    /// 1. Removes existing stored tags that are *proper ancestors* of any tag being added
    ///    (they become redundant once a deeper descendant is stored).
    /// 2. Inserts only the *deepest* tags from the input — any tag that is a proper ancestor of
    ///    another tag in the same input list is silently dropped.
    ///
    /// Must be called within a transaction together with `batch_remove` so that the overall
    /// remove-then-add is atomic.
    pub async fn batch_assign<'e, E>(
        ex: E,
        local_user_id: Uuid,
        picture_ids: &[Uuid],
        tags: &[String],
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        Self::assign_inner(ex, local_user_id, picture_ids, tags, false).await
    }

    /// Like [`batch_assign`](Self::batch_assign) but also tags soft-deleted (trashed) pictures.
    /// Used by the upload flow to tag already-existing-and-deleted duplicates (feature 15) so the
    /// user can find and restore them.
    #[tracing::instrument(skip(ex, picture_ids, tags), fields(user_id = %local_user_id))]
    pub async fn batch_assign_including_deleted<'e, E>(
        ex: E,
        local_user_id: Uuid,
        picture_ids: &[Uuid],
        tags: &[String],
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        Self::assign_inner(ex, local_user_id, picture_ids, tags, true).await
    }

    /// Shared implementation. Returns the number of tag rows actually inserted (0 ⇒ every tag was
    /// already present), which callers use to tell "newly tagged" from a no-op (e.g. WebDAV PUT).
    #[tracing::instrument(skip(ex, picture_ids, tags), fields(user_id = %local_user_id))]
    async fn assign_inner<'e, E>(
        ex: E,
        local_user_id: Uuid,
        picture_ids: &[Uuid],
        tags: &[String],
        include_deleted: bool,
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if tags.is_empty() || picture_ids.is_empty() {
            return Ok(0);
        }
        // Data-modifying CTE: cleanup (remove proper ancestors) + invalidate (re-dirty the
        // targeted pictures so the pipeline re-gates) + insert (only deepest).
        // `tag_path @> t AND tag_path <> t` = strict ancestor of t → remove it (redundant).
        // NOT EXISTS (deeper descendant) = this tag is the deepest in the input list.
        // `$4` lets the upload flow also tag soft-deleted pictures.
        let res = sqlx::query!(
            r#"WITH cleanup AS (
                 DELETE FROM tags
                 WHERE picture_id = ANY($1::uuid[])
                   AND tag_path @> ANY($3::ltree[])
                   AND NOT (tag_path = ANY($3::ltree[]))
                   AND source = 'manual'::tag_source
                   AND picture_id IN (
                     SELECT id FROM pictures
                     WHERE local_user_id = $2 AND ($4 OR deleted_at IS NULL)
                   )
               ),
               invalidate AS (
                 UPDATE pictures SET last_pipeline_run_at = NULL
                 WHERE id = ANY($1::uuid[]) AND local_user_id = $2 AND ($4 OR deleted_at IS NULL)
               )
               INSERT INTO tags (picture_id, tag_path, source)
               SELECT p.id, filtered.tag::ltree, 'manual'::tag_source
               FROM (
                 SELECT id FROM pictures
                 WHERE id = ANY($1::uuid[]) AND local_user_id = $2 AND ($4 OR deleted_at IS NULL)
               ) AS p
               CROSS JOIN (
                 SELECT t AS tag FROM unnest($3::text[]) AS t
                 WHERE NOT EXISTS (
                   SELECT 1 FROM unnest($3::text[]) AS deeper
                   WHERE deeper <> t AND deeper::ltree <@ t::ltree
                 )
               ) AS filtered
               WHERE NOT EXISTS (
                 SELECT 1 FROM tags existing
                 WHERE existing.picture_id = p.id
                   AND existing.tag_path <@ filtered.tag::ltree
                   AND existing.tag_path <> filtered.tag::ltree
                   AND existing.source = 'manual'::tag_source
               )
               ON CONFLICT (picture_id, tag_path) WHERE source = 'manual' DO NOTHING"#,
            picture_ids as &[Uuid],
            local_user_id,
            tags as &[String],
            include_deleted,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected())
    }

    /// Remove tags (and all their subtags) from all pictures in the batch.
    /// All pictures must belong to `local_user_id`.
    ///
    /// Only removes `source = 'manual'` tags — system-assigned tags (`incoming_share`, `rule`, etc.)
    /// are never touched by user operations.
    #[tracing::instrument(skip(ex, picture_ids, tags), fields(user_id = %local_user_id))]
    pub async fn batch_remove<'e, E>(
        ex: E,
        local_user_id: Uuid,
        picture_ids: &[Uuid],
        tags: &[String],
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if tags.is_empty() || picture_ids.is_empty() {
            return Ok(());
        }
        // Re-dirty only the pictures that actually lost a tag, so the pipeline re-gates.
        sqlx::query!(
            r#"WITH del AS (
                 DELETE FROM tags
                 WHERE picture_id = ANY($1::uuid[])
                   AND tag_path <@ ANY($2::ltree[])
                   AND source = 'manual'::tag_source
                   AND picture_id IN (
                     SELECT id FROM pictures WHERE local_user_id = $3 AND deleted_at IS NULL
                   )
                 RETURNING picture_id
               )
               UPDATE pictures SET last_pipeline_run_at = NULL
               WHERE id IN (SELECT picture_id FROM del)"#,
            picture_ids as &[Uuid],
            tags as &[String],
            local_user_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Distinct `manual` tag paths of a single picture. Used by the dedup "keep this copy" flow to
    /// mirror the previously-live picture's curated tag set onto the new survivor (feature 11 §5.5).
    #[tracing::instrument(skip(ex), fields(picture_id = %picture_id))]
    pub async fn list_manual_paths<'e, E>(ex: E, picture_id: Uuid) -> Result<Vec<String>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_scalar!(
            r#"SELECT tag_path::text as "tag_path!"
               FROM tags
               WHERE picture_id = $1 AND source = 'manual'::tag_source"#,
            picture_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Delete every `manual` tag of a picture the user holds (dedup survivor swap, feature 11 §5.5).
    /// Does not invalidate the pipeline itself — the caller (`set_survivor`) already re-dirties the
    /// whole group.
    #[tracing::instrument(skip(ex), fields(user_id = %local_user_id, picture_id = %picture_id))]
    pub async fn clear_manual_tags<'e, E>(
        ex: E,
        local_user_id: Uuid,
        picture_id: Uuid,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"DELETE FROM tags
               WHERE picture_id = $1
                 AND source = 'manual'::tag_source
                 AND picture_id IN (SELECT id FROM pictures WHERE local_user_id = $2)"#,
            picture_id,
            local_user_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Whether the picture carries a **non-manual** tag (rule/segment/share_mapping/
    /// incoming_share) under any of `paths` (inclusive). The WebDAV write-back layer uses this
    /// to detect that an `onRemove` cannot break membership — a live service still asserts the
    /// tag — and return `409 Conflict` instead (06_webdav.md §7.2).
    #[tracing::instrument(skip(ex, paths), fields(picture_id = %picture_id))]
    pub async fn has_non_manual_tag_under<'e, E>(
        ex: E,
        picture_id: Uuid,
        paths: &[String],
    ) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if paths.is_empty() {
            return Ok(false);
        }
        let row = sqlx::query!(
            r#"SELECT EXISTS(
                 SELECT 1 FROM tags
                 WHERE picture_id = $1
                   AND tag_path <@ ANY($2::ltree[])
                   AND source <> 'manual'::tag_source
               ) AS "exists!""#,
            picture_id,
            paths as &[String],
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(row.exists)
    }

    /// Assign a `/SharedToMe/…` tag to a received picture, linked to the incoming share that
    /// created it. Used exclusively by the share-acceptance and picture-announcement flows.
    ///
    /// `picture_token` is the per-picture presign token the sender generated; it is stored on
    /// this row and used to authorise presign calls to the sender (and forwarded downstream in
    /// transitive announcements). Uses `ON CONFLICT DO UPDATE SET picture_token` so re-announcing
    /// the same picture refreshes the token without error (token-refresh path).
    #[tracing::instrument(skip(ex), fields(picture_id = %picture_id, incoming_share_id = %incoming_share_id))]
    pub async fn assign_incoming_share_tag<'e, E>(
        ex: E,
        picture_id: Uuid,
        tag_path_ltree: &str,
        incoming_share_id: Uuid,
        picture_token: Uuid,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"INSERT INTO tags (picture_id, tag_path, source, source_id, picture_token)
               VALUES ($1, $2::text::ltree, 'incoming_share'::tag_source, $3, $4)
               ON CONFLICT (picture_id, tag_path, source, source_id) WHERE source <> 'manual'
               DO UPDATE SET picture_token = EXCLUDED.picture_token"#,
            picture_id,
            tag_path_ltree,
            incoming_share_id,
            picture_token,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Remove all `incoming_share` tags assigned by the given share, returning the distinct
    /// picture IDs that were affected (needed by `cleanup_incoming_share` to compute survivors).
    /// Called on share revocation to clean up all `/SharedToMe/…` entries for that share.
    #[tracing::instrument(skip(ex), fields(incoming_share_id = %incoming_share_id))]
    pub async fn remove_incoming_share_tags<'e, E>(
        ex: E,
        incoming_share_id: Uuid,
    ) -> Result<Vec<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows = sqlx::query_scalar!(
            r#"DELETE FROM tags
               WHERE source = 'incoming_share'::tag_source AND source_id = $1
               RETURNING picture_id"#,
            incoming_share_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)?;
        // Distinct picture ids.
        let mut ids: Vec<Uuid> = rows;
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// Distinct `SharedToMe.*` tag paths currently assigned by an incoming share. Used by
    /// transitive revocation to locate downstream shares re-sharing this tag (before the tags
    /// are removed).
    #[tracing::instrument(skip(ex), fields(incoming_share_id = %incoming_share_id))]
    pub async fn incoming_share_tag_paths<'e, E>(
        ex: E,
        incoming_share_id: Uuid,
    ) -> Result<Vec<String>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_scalar!(
            r#"SELECT DISTINCT tag_path::text as "tag_path!"
               FROM tags
               WHERE source = 'incoming_share'::tag_source AND source_id = $1"#,
            incoming_share_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Remove the incoming-share tags of a specific share for a specific set of pictures.
    /// Used by per-picture unannounce (a subset of the share leaves coverage). Returns the
    /// affected picture ids.
    #[tracing::instrument(skip(ex, picture_ids), fields(incoming_share_id = %incoming_share_id))]
    pub async fn remove_incoming_share_tags_for_pictures<'e, E>(
        ex: E,
        incoming_share_id: Uuid,
        picture_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if picture_ids.is_empty() {
            return Ok(vec![]);
        }
        let rows = sqlx::query_scalar!(
            r#"DELETE FROM tags
               WHERE source = 'incoming_share'::tag_source
                 AND source_id = $1
                 AND picture_id = ANY($2::uuid[])
               RETURNING picture_id"#,
            incoming_share_id,
            picture_ids as &[Uuid],
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)?;
        let mut ids = rows;
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// Select the active presign token for a received picture: the `picture_token` of any
    /// `incoming_share` tag whose share is still active, chosen deterministically by
    /// `source_id` (lowest UUID) so the choice is stable across runs. Returns `None` for owned
    /// pictures or when every covering share has been revoked.
    ///
    /// Used both by the recipient's presign path and by the pipeline's transitive token
    /// selection (§5.3).
    #[tracing::instrument(skip(ex), fields(picture_id = %picture_id))]
    pub async fn find_active_picture_token<'e, E>(
        ex: E,
        picture_id: Uuid,
    ) -> Result<Option<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_scalar!(
            r#"SELECT t.picture_token
               FROM tags t
               JOIN incoming_shares ish ON ish.id = t.source_id
               WHERE t.picture_id = $1
                 AND t.source = 'incoming_share'::tag_source
                 AND t.picture_token IS NOT NULL
                 AND ish.status = 'active'::share_status
               ORDER BY t.source_id
               LIMIT 1"#,
            picture_id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
        .map(|opt| opt.flatten())
    }

    /// Batch variant of [`find_active_picture_token`](Self::find_active_picture_token): for each
    /// of `picture_ids` that is a received picture with an active covering share, return its
    /// deterministically-chosen token. Used by the pipeline announcement step (token-refresh).
    #[tracing::instrument(skip(ex, picture_ids))]
    pub async fn active_picture_tokens_for<'e, E>(
        ex: E,
        picture_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if picture_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        // DISTINCT ON picks the lowest source_id per picture (matching the single-row query).
        let rows = sqlx::query!(
            r#"SELECT DISTINCT ON (t.picture_id) t.picture_id, t.picture_token as "picture_token!"
               FROM tags t
               JOIN incoming_shares ish ON ish.id = t.source_id
               WHERE t.picture_id = ANY($1::uuid[])
                 AND t.source = 'incoming_share'::tag_source
                 AND t.picture_token IS NOT NULL
                 AND ish.status = 'active'::share_status
               ORDER BY t.picture_id, t.source_id"#,
            picture_ids as &[Uuid],
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(rows
            .into_iter()
            .map(|r| (r.picture_id, r.picture_token))
            .collect())
    }

    /// Remove every pipeline tag (`rule`/`segment`/`share_mapping`) produced by a service.
    /// Called when a service is disabled or deleted without tag promotion — its tags are no longer
    /// live. Also resets `last_pipeline_run_at = NULL` on every affected picture so the pipeline
    /// re-evaluates their coverage and unannounces them from any active share they no longer cover.
    #[tracing::instrument(skip(ex))]
    pub async fn remove_service_tags<'e, E>(ex: E, service_id: Uuid) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"WITH removed AS (
                   DELETE FROM tags
                   WHERE source_id = $1
                     AND source IN ('rule'::tag_source, 'segment'::tag_source, 'share_mapping'::tag_source)
                   RETURNING picture_id
               )
               UPDATE pictures SET last_pipeline_run_at = NULL
               WHERE id IN (SELECT DISTINCT picture_id FROM removed)"#,
            service_id,
        )
            .execute(ex)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Count pictures in the selection holding a **manual** tag at or under any of `paths`
    /// (inclusive) — the removable count for the tags batch dry-run (feature 14 §6.1). Mirrors the
    /// `batch_remove` predicate (`tag_path <@ path AND source = 'manual'`).
    #[tracing::instrument(skip(db, sel, paths), fields(user_id = %local_user_id))]
    pub async fn count_selection_with_manual_under(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
        paths: &[String],
    ) -> Result<i64, AppError> {
        if sel.is_empty() || paths.is_empty() {
            return Ok(0);
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT COUNT(*)::bigint FROM pictures p WHERE EXISTS (SELECT 1 FROM tags tg \
             WHERE tg.picture_id = p.id AND tg.source = 'manual'::tag_source AND tg.tag_path <@ ANY(",
        );
        q.push_bind(paths.to_vec()).push("::ltree[])) AND ");
        PictureRepository::push_selection_where(&mut q, local_user_id, sel);
        q.build_query_scalar()
            .fetch_one(db)
            .await
            .map_err(map_sqlx_error)
    }

    /// Ancestor-expanded tag aggregation over a selection (feature 14 §4.2). When `provenance` is
    /// set, each path also carries its per-source breakdown (a heavier path×source query).
    #[tracing::instrument(skip(db, sel), fields(user_id = %local_user_id, provenance))]
    pub async fn aggregate_tags(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
        provenance: bool,
    ) -> Result<Vec<TagAgg>, AppError> {
        use sqlx::Row;
        if sel.is_empty() {
            return Ok(vec![]);
        }

        // Path → (count, manual_count), ancestor-expanded via a per-tag prefix lateral.
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT pfx.prefix::text AS path, COUNT(DISTINCT tg.picture_id)::bigint AS cnt, \
             (COUNT(DISTINCT tg.picture_id) FILTER (WHERE tg.source = 'manual'::tag_source))::bigint AS manual_count \
             FROM tags tg JOIN pictures p ON p.id = tg.picture_id \
             CROSS JOIN LATERAL (SELECT subpath(tg.tag_path, 0, gs) AS prefix \
                                 FROM generate_series(1, nlevel(tg.tag_path)) gs) pfx \
             WHERE ",
        );
        PictureRepository::push_selection_where(&mut q, local_user_id, sel);
        q.push(" GROUP BY pfx.prefix ORDER BY pfx.prefix");
        let rows = q.build().fetch_all(db).await.map_err(map_sqlx_error)?;

        let mut aggs: Vec<TagAgg> = Vec::with_capacity(rows.len());
        for r in rows {
            aggs.push(TagAgg {
                path: r.try_get("path").map_err(map_sqlx_error)?,
                count: r.try_get("cnt").map_err(map_sqlx_error)?,
                manual_count: r.try_get("manual_count").map_err(map_sqlx_error)?,
                sources: vec![],
            });
        }

        if provenance {
            let mut pq = sqlx::QueryBuilder::<Postgres>::new(
                "SELECT pfx.prefix::text AS path, tg.source AS \"source\", COUNT(DISTINCT tg.picture_id)::bigint AS cnt \
                 FROM tags tg JOIN pictures p ON p.id = tg.picture_id \
                 CROSS JOIN LATERAL (SELECT subpath(tg.tag_path, 0, gs) AS prefix \
                                     FROM generate_series(1, nlevel(tg.tag_path)) gs) pfx \
                 WHERE ",
            );
            PictureRepository::push_selection_where(&mut pq, local_user_id, sel);
            pq.push(" GROUP BY pfx.prefix, tg.source");
            let rows = pq.build().fetch_all(db).await.map_err(map_sqlx_error)?;
            let mut by_path: std::collections::HashMap<String, Vec<(TagSource, i64)>> =
                std::collections::HashMap::new();
            for r in rows {
                let path: String = r.try_get("path").map_err(map_sqlx_error)?;
                let source: TagSource = r.try_get("source").map_err(map_sqlx_error)?;
                let cnt: i64 = r.try_get("cnt").map_err(map_sqlx_error)?;
                by_path.entry(path).or_default().push((source, cnt));
            }
            for agg in &mut aggs {
                if let Some(sources) = by_path.remove(&agg.path) {
                    agg.sources = sources;
                }
            }
        }

        Ok(aggs)
    }

    /// Promote a service's pipeline tags to `manual`, preserving the user's curation when
    /// the service is deleted. The result keeps manual tags in minimal (deepest-only) form,
    /// mirroring [`batch_assign`](Self::batch_assign):
    ///
    /// - A pipeline tag whose **exact** path is already a manual tag is dropped (the manual
    ///   row wins).
    /// - An existing manual tag that is a strict **ancestor** of a tag being promoted is
    ///   pruned (the deeper promoted tag makes it redundant).
    /// - The remaining pipeline rows are converted in place.
    ///
    /// Done as one statement: the data-modifying CTEs delete the colliding rows and the
    /// redundant ancestors, and the outer UPDATE converts the disjoint remainder — no row is
    /// touched twice and the manual uniqueness index is never violated.
    #[tracing::instrument(skip(ex))]
    pub async fn promote_service_tags_to_manual<'e, E>(
        ex: E,
        service_id: Uuid,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"WITH to_promote AS (
                 SELECT t.id, t.picture_id, t.tag_path
                 FROM tags t
                 WHERE t.source_id = $1
                   AND t.source IN ('rule'::tag_source, 'segment'::tag_source, 'share_mapping'::tag_source)
               ),
               -- Pipeline row whose exact path is already held manually → manual wins, drop it.
               drop_collide AS (
                 DELETE FROM tags t
                 USING to_promote tp
                 WHERE t.id = tp.id
                   AND EXISTS (
                     SELECT 1 FROM tags m
                     WHERE m.picture_id = tp.picture_id
                       AND m.tag_path = tp.tag_path
                       AND m.source = 'manual'::tag_source
                   )
                 RETURNING t.id
               ),
               -- Existing manual ancestor made redundant by a deeper tag we are about to promote.
               prune_ancestors AS (
                 DELETE FROM tags m
                 USING to_promote tp
                 WHERE m.source = 'manual'::tag_source
                   AND m.picture_id = tp.picture_id
                   AND m.tag_path @> tp.tag_path
                   AND m.tag_path <> tp.tag_path
                   AND tp.id NOT IN (SELECT id FROM drop_collide)
                 RETURNING m.id
               ),
               invalidate AS (
                 UPDATE pictures SET last_pipeline_run_at = NULL
                 WHERE id IN (SELECT DISTINCT picture_id FROM to_promote)
               )
               UPDATE tags t
               SET source = 'manual'::tag_source, source_id = NULL
               WHERE t.source_id = $1
                 AND t.source IN ('rule'::tag_source, 'segment'::tag_source, 'share_mapping'::tag_source)
                 AND NOT EXISTS (
                   SELECT 1 FROM tags m
                   WHERE m.picture_id = t.picture_id
                     AND m.tag_path = t.tag_path
                     AND m.source = 'manual'::tag_source
                 )"#,
            service_id,
        )
            .execute(ex)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;
    use uuid::Uuid;

    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

    async fn seed_user(db: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO users (id, username, email, display_name) VALUES ($1, $2, $3, $4)",
            id,
            format!("u_{}", &id.to_string()[..8]),
            format!("{}@t.com", id),
            "T",
        )
        .execute(db)
        .await
        .unwrap();
        id
    }

    async fn seed_picture(db: &PgPool, user_id: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO pictures (id, local_user_id) VALUES ($1, $2)",
            id,
            user_id,
        )
        .execute(db)
        .await
        .unwrap();
        id
    }

    /// Insert an active incoming_share row for the given recipient and return its id.
    async fn seed_incoming_share(db: &PgPool, recipient: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO incoming_shares
                 (id, recipient_id, sender_username, sender_instance, name, outgoing_share_id, status)
             VALUES ($1, $2, 'alice', 'ex.com', 'Test share', $3, 'active'::share_status)",
            id,
            recipient,
            Uuid::new_v4(),
        )
        .execute(db)
        .await
        .unwrap();
        id
    }

    /// Insert a pipeline tag directly (bypassing the pipeline) for tests setup.
    async fn seed_pipeline_tag(db: &PgPool, pic: Uuid, path: &str, source: &str, source_id: Uuid) {
        sqlx::query(
            "INSERT INTO tags (picture_id, tag_path, source, source_id) \
             VALUES ($1, $2::text::ltree, $3::text::tag_source, $4)",
        )
        .bind(pic)
        .bind(path)
        .bind(source)
        .bind(source_id)
        .execute(db)
        .await
        .unwrap();
    }

    /// Force `last_pipeline_run_at` to a non-NULL value so a test can assert it was re-NULLed.
    async fn mark_pipeline_ran(db: &PgPool, pic: Uuid) {
        sqlx::query!(
            "UPDATE pictures SET last_pipeline_run_at = now() AT TIME ZONE 'utc' WHERE id = $1",
            pic,
        )
        .execute(db)
        .await
        .unwrap();
    }

    async fn is_dirty(db: &PgPool, pic: Uuid) -> bool {
        sqlx::query_scalar!(
            r#"SELECT (last_pipeline_run_at IS NULL) AS "dirty!" FROM pictures WHERE id = $1"#,
            pic,
        )
        .fetch_one(db)
        .await
        .unwrap()
    }

    // ── intrinsic pipeline invalidation ────────────────────────────────────────

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn batch_assign_invalidates_pipeline(db: PgPool) {
        let user = seed_user(&db).await;
        let pic = seed_picture(&db, user).await;
        mark_pipeline_ran(&db, pic).await;

        TagRepository::batch_assign(&db, user, &[pic], &["Photos.Travel".to_string()])
            .await
            .unwrap();

        assert!(
            is_dirty(&db, pic).await,
            "a manual tag add re-dirties the picture"
        );
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn batch_remove_invalidates_only_changed(db: PgPool) {
        let user = seed_user(&db).await;
        let tagged = seed_picture(&db, user).await;
        let untouched = seed_picture(&db, user).await;
        TagRepository::batch_assign(&db, user, &[tagged], &["Photos.Travel".to_string()])
            .await
            .unwrap();
        mark_pipeline_ran(&db, tagged).await;
        mark_pipeline_ran(&db, untouched).await;

        TagRepository::batch_remove(&db, user, &[tagged, untouched], &["Photos".to_string()])
            .await
            .unwrap();

        assert!(
            is_dirty(&db, tagged).await,
            "the picture that lost a tag is re-dirtied"
        );
        assert!(
            !is_dirty(&db, untouched).await,
            "a picture with nothing removed is left untouched"
        );
    }

    // ── batch_assign ──────────────────────────────────────────────────────────

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn batch_assign_adds_tags(db: PgPool) {
        let user = seed_user(&db).await;
        let pic = seed_picture(&db, user).await;

        TagRepository::batch_assign(&db, user, &[pic], &["Photos.Travel".to_string()])
            .await
            .unwrap();

        let tags = TagRepository::list_for_picture(&db, user, pic)
            .await
            .unwrap();
        assert!(tags.iter().any(|t| t.tag_path == "Photos.Travel"));
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn batch_assign_is_idempotent(db: PgPool) {
        let user = seed_user(&db).await;
        let pic = seed_picture(&db, user).await;

        let tags = vec!["Photos.Travel".to_string()];
        TagRepository::batch_assign(&db, user, &[pic], &tags)
            .await
            .unwrap();
        TagRepository::batch_assign(&db, user, &[pic], &tags)
            .await
            .unwrap();

        let stored = TagRepository::list_for_picture(&db, user, pic)
            .await
            .unwrap();
        let count = stored
            .iter()
            .filter(|t| t.tag_path == "Photos.Travel")
            .count();
        assert_eq!(count, 1, "idempotent — no duplicate");
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn batch_assign_prunes_ancestor_when_deeper_added(db: PgPool) {
        let user = seed_user(&db).await;
        let pic = seed_picture(&db, user).await;

        // Add parent first
        TagRepository::batch_assign(&db, user, &[pic], &["Photos".to_string()])
            .await
            .unwrap();
        // Then add a child — parent should be pruned (becomes redundant)
        TagRepository::batch_assign(&db, user, &[pic], &["Photos.Travel".to_string()])
            .await
            .unwrap();

        let stored = TagRepository::list_for_picture(&db, user, pic)
            .await
            .unwrap();
        assert!(
            !stored.iter().any(|t| t.tag_path == "Photos"),
            "ancestor pruned"
        );
        assert!(stored.iter().any(|t| t.tag_path == "Photos.Travel"));
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn batch_assign_prunes_ancestor_when_shallow_added(db: PgPool) {
        let user = seed_user(&db).await;
        let pic = seed_picture(&db, user).await;

        // Add child first
        TagRepository::batch_assign(&db, user, &[pic], &["Photos.Travel".to_string()])
            .await
            .unwrap();
        // Then add a parend — parent should not be added
        TagRepository::batch_assign(&db, user, &[pic], &["Photos".to_string()])
            .await
            .unwrap();

        let stored = TagRepository::list_for_picture(&db, user, pic)
            .await
            .unwrap();
        assert!(
            !stored.iter().any(|t| t.tag_path == "Photos"),
            "ancestor not added because child exists"
        );
        assert!(stored.iter().any(|t| t.tag_path == "Photos.Travel"));
    }

    // ── batch_remove ──────────────────────────────────────────────────────────

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn batch_remove_removes_tag_and_subtags(db: PgPool) {
        let user = seed_user(&db).await;
        let pic = seed_picture(&db, user).await;

        TagRepository::batch_assign(&db, user, &[pic], &["Photos.Travel.Alps".to_string()])
            .await
            .unwrap();
        // Remove at Photos level — Alps is a subtag so it should also be removed
        TagRepository::batch_remove(&db, user, &[pic], &["Photos".to_string()])
            .await
            .unwrap();

        let stored = TagRepository::list_for_picture(&db, user, pic)
            .await
            .unwrap();
        assert!(stored.is_empty(), "subtags removed");
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn batch_remove_removes_tag_and_keep_parents(db: PgPool) {
        let user = seed_user(&db).await;
        let pic = seed_picture(&db, user).await;

        TagRepository::batch_assign(
            &db,
            user,
            &[pic],
            &["Photos.Travel.Alps.Grenoble".to_string()],
        )
        .await
        .unwrap();
        // Currently, deleting a tag does not keep the parent tags.
        TagRepository::batch_remove(&db, user, &[pic], &["Photos.Travel.Alps".to_string()])
            .await
            .unwrap();

        let stored = TagRepository::list_for_picture(&db, user, pic)
            .await
            .unwrap();
        assert!(stored.is_empty(), "parent tags kept");
        //assert!(stored.iter().any(|t| t.tag_path == "Photos.Travel"));
    }

    // ── assign_incoming_share_tag / remove_incoming_share_tags ────────────────

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn assign_and_remove_incoming_share_tag(db: PgPool) {
        let user = seed_user(&db).await;
        let pic = seed_picture(&db, user).await;
        // A real incoming_shares row is required so the token-selection join works and the
        // FK-free source_id is meaningful.
        let share_id = seed_incoming_share(&db, user).await;
        let token = Uuid::new_v4();

        TagRepository::assign_incoming_share_tag(
            &db,
            pic,
            "SharedToMe.alice_AT_ex_DOT_com.Photos",
            share_id,
            token,
        )
        .await
        .unwrap();

        let tags = TagRepository::list_for_picture(&db, user, pic)
            .await
            .unwrap();
        assert!(tags.iter().any(|t| t.source_id == Some(share_id)));

        let affected = TagRepository::remove_incoming_share_tags(&db, share_id)
            .await
            .unwrap();
        assert_eq!(affected, vec![pic]);

        let tags = TagRepository::list_for_picture(&db, user, pic)
            .await
            .unwrap();
        assert!(tags.is_empty());
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn assign_incoming_share_tag_refreshes_token_on_conflict(db: PgPool) {
        let user = seed_user(&db).await;
        let pic = seed_picture(&db, user).await;
        let share_id = seed_incoming_share(&db, user).await;
        let token1 = Uuid::new_v4();
        let token2 = Uuid::new_v4();

        TagRepository::assign_incoming_share_tag(
            &db,
            pic,
            "SharedToMe.alice_AT_ex_DOT_com.Photos",
            share_id,
            token1,
        )
        .await
        .unwrap();
        // Replay with a new token updates the stored token (ON CONFLICT DO UPDATE).
        TagRepository::assign_incoming_share_tag(
            &db,
            pic,
            "SharedToMe.alice_AT_ex_DOT_com.Photos",
            share_id,
            token2,
        )
        .await
        .unwrap();

        let tags = TagRepository::list_for_picture(&db, user, pic)
            .await
            .unwrap();
        assert_eq!(tags.len(), 1);
        // Active share → token selection returns the refreshed token.
        let selected = TagRepository::find_active_picture_token(&db, pic)
            .await
            .unwrap();
        assert_eq!(selected, Some(token2));
    }

    // ── per-source storage / lifecycle ────────────────────────────────────────

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn manual_and_pipeline_tags_coexist_for_same_path(db: PgPool) {
        let user = seed_user(&db).await;
        let pic = seed_picture(&db, user).await;
        let svc = Uuid::new_v4();

        TagRepository::batch_assign(&db, user, &[pic], &["Photos.Travel".to_string()])
            .await
            .unwrap();
        seed_pipeline_tag(&db, pic, "Photos.Travel", "rule", svc).await;

        // Same path, two sources → two rows (different partial unique indexes).
        let stored = TagRepository::list_for_picture(&db, user, pic)
            .await
            .unwrap();
        assert_eq!(
            stored
                .iter()
                .filter(|t| t.tag_path == "Photos.Travel")
                .count(),
            2,
        );
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn remove_service_tags_drops_only_that_service(db: PgPool) {
        let user = seed_user(&db).await;
        let pic = seed_picture(&db, user).await;
        let svc_a = Uuid::new_v4();
        let svc_b = Uuid::new_v4();

        seed_pipeline_tag(&db, pic, "A.Tag", "rule", svc_a).await;
        seed_pipeline_tag(&db, pic, "B.Tag", "segment", svc_b).await;
        TagRepository::batch_assign(&db, user, &[pic], &["Manual.Tag".to_string()])
            .await
            .unwrap();

        TagRepository::remove_service_tags(&db, svc_a)
            .await
            .unwrap();

        let stored = TagRepository::list_for_picture(&db, user, pic)
            .await
            .unwrap();
        assert!(
            !stored.iter().any(|t| t.tag_path == "A.Tag"),
            "svc_a tag gone"
        );
        assert!(
            stored.iter().any(|t| t.tag_path == "B.Tag"),
            "svc_b tag kept"
        );
        assert!(
            stored.iter().any(|t| t.tag_path == "Manual.Tag"),
            "manual tag untouched"
        );
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn promote_service_tags_converts_to_manual(db: PgPool) {
        let user = seed_user(&db).await;
        let pic = seed_picture(&db, user).await;
        let svc = Uuid::new_v4();

        seed_pipeline_tag(&db, pic, "Photos.Alps", "segment", svc).await;
        TagRepository::promote_service_tags_to_manual(&db, svc)
            .await
            .unwrap();

        let stored = TagRepository::list_for_picture(&db, user, pic)
            .await
            .unwrap();
        let tag = stored
            .iter()
            .find(|t| t.tag_path == "Photos.Alps")
            .expect("tag still present");
        assert_eq!(tag.source, TagSource::Manual);
        assert!(tag.source_id.is_none());
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn promote_service_tags_drops_row_colliding_with_existing_manual(db: PgPool) {
        let user = seed_user(&db).await;
        let pic = seed_picture(&db, user).await;
        let svc = Uuid::new_v4();

        // A manual tag already holds the path the service also produced.
        TagRepository::batch_assign(&db, user, &[pic], &["Photos.Alps".to_string()])
            .await
            .unwrap();
        seed_pipeline_tag(&db, pic, "Photos.Alps.Test", "segment", svc).await;

        TagRepository::promote_service_tags_to_manual(&db, svc)
            .await
            .unwrap();

        // The manual row wins; the colliding pipeline row is dropped — exactly one row remains.
        let stored = TagRepository::list_for_picture(&db, user, pic)
            .await
            .unwrap();
        let matching: Vec<_> = stored
            .iter()
            .filter(|t| t.tag_path == "Photos.Alps.Test")
            .collect();
        assert_eq!(matching.len(), 1, "Matching len is not 1");
        assert_eq!(
            matching[0].source,
            TagSource::Manual,
            "Matching source is not Manual"
        );
        assert_eq!(stored.len(), 1, "More than 1 remaining tag");
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn promote_service_tags_drops_row_with_exact_manual_twin(db: PgPool) {
        let user = seed_user(&db).await;
        let pic = seed_picture(&db, user).await;
        let svc = Uuid::new_v4();

        // Manual tag holds the exact path the service also produced.
        TagRepository::batch_assign(&db, user, &[pic], &["Photos.Alps".to_string()])
            .await
            .unwrap();
        seed_pipeline_tag(&db, pic, "Photos.Alps", "segment", svc).await;

        TagRepository::promote_service_tags_to_manual(&db, svc)
            .await
            .unwrap();

        // Exact twin → the manual row wins, the pipeline row is dropped: one manual row remains.
        let stored = TagRepository::list_for_picture(&db, user, pic)
            .await
            .unwrap();
        assert_eq!(stored.len(), 1, "exactly one row remains");
        assert_eq!(stored[0].tag_path, "Photos.Alps");
        assert_eq!(stored[0].source, TagSource::Manual);
    }
}
