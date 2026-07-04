//! Pipeline-specific repository queries.
//!
//! These are kept separate from the general picture/tagging repositories because
//! they operate on a projection of `pictures` that the pipeline needs, and on
//! bulk tag-assignment logic specific to pipeline output.

use crate::domain::job::CameraExif;
use crate::infra::error::{AppError, map_sqlx_error};
use chrono::NaiveDateTime;
use sqlx::types::Json;
use sqlx::{Executor, PgPool, Postgres};
use std::collections::HashMap;
use uuid::Uuid;

/// Minimal picture projection used by the pipeline evaluator. Carries every field the rule
/// predicates (feature 13) and segmentation can read; camera/lens fields come from `exif_data`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PipelinePicture {
    pub id: Uuid,
    pub captured_at: Option<NaiveDateTime>,
    pub ingested_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub gps_lat: Option<f64>,
    pub gps_lng: Option<f64>,
    pub gps_alt: Option<i32>,
    pub orientation: Option<i16>,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<i64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// Camera/lens EXIF (brand, model, focal length, f-number, ISO, exposure num/den).
    pub exif_data: Json<CameraExif>,
    /// `true` when this user owns the picture (`remote_picture_id IS NULL`).
    pub is_owned: bool,
}

/// A tag to assign as output of the pipeline, with its source.
pub struct PipelineTagAssignment {
    /// Ltree-formatted tag path (e.g. `Photos.Travel.Alps`).
    pub tag_path: String,
    /// Postgres `tag_source` enum value as a string (`"rule"`, `"segment"`, `"share_mapping"`).
    pub source: String,
    /// ID of the tagging service that produced this tag.
    pub source_id: Uuid,
}

pub struct PipelineRepository;

impl PipelineRepository {
    /// Return the IDs of users the pipeline needs to process: those with a dirty picture
    /// (for tagging or the announcement diff) **or** an `OutgoingShare` awaiting its first
    /// announcement (which the pipeline must announce and flip to `active`, even if the share has
    /// no pictures or none are dirty).
    #[tracing::instrument(skip(db))]
    pub async fn find_users_with_dirty_pictures(db: &PgPool) -> Result<Vec<Uuid>, AppError> {
        // Soft-deleted pictures are intentionally *not* filtered out: they stay tagged and announced
        // until permanently removed (see doc/features/02 §6).
        let rows = sqlx::query_scalar!(
            r#"SELECT DISTINCT p.local_user_id AS "id!"
               FROM pictures p
               WHERE (p.last_pipeline_run_at IS NULL)
                  OR EXISTS (
                     SELECT 1 FROM tagging_services ts
                     WHERE ts.owner_id = p.local_user_id
                       AND ts.enabled = true
                       AND p.last_pipeline_run_at < ts.last_invalidated_at
                  )
               UNION
               -- Owners of shares awaiting a (re)announcement whose backoff window has elapsed.
               SELECT DISTINCT os.owner_id AS "id!"
               FROM outgoing_shares os
               WHERE os.status IN ('pending_first_announcement'::share_status, 'errored'::share_status)
                 AND (os.next_retry_at IS NULL OR os.next_retry_at <= now() AT TIME ZONE 'utc')"#,
        )
        .fetch_all(db)
        .await
        .map_err(map_sqlx_error)?;
        Ok(rows)
    }

    /// Return all dirty pictures for a specific user.
    ///
    /// A picture is dirty if `last_pipeline_run_at IS NULL` or if any enabled service for that user
    /// has a `last_invalidated_at` newer than the picture's `last_pipeline_run_at`.
    #[tracing::instrument(skip(ex), fields(user_id = %user_id))]
    pub async fn find_dirty_for_user<'e, E>(
        ex: E,
        user_id: Uuid,
    ) -> Result<Vec<PipelinePicture>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        // Soft-deleted pictures are still re-tagged (they stay announced until permanent removal,
        // see doc/features/02 §6), so they are not filtered out here.
        sqlx::query_as!(
            PipelinePicture,
            r#"SELECT p.id, p.captured_at, p.ingested_at, p.updated_at,
                      p.gps_lat, p.gps_lng, p.gps_alt, p.orientation,
                      p.filename, p.mime_type, p.file_size, p.width, p.height,
                      p.exif_data as "exif_data: Json<CameraExif>",
                      (p.remote_picture_id IS NULL) as "is_owned!"
               FROM pictures p
               WHERE p.local_user_id = $1
                 AND (
                   (p.last_pipeline_run_at IS NULL)
                   OR
                   EXISTS (
                     SELECT 1 FROM tagging_services ts
                     WHERE ts.owner_id = $1
                       AND ts.enabled = true
                       AND p.last_pipeline_run_at < ts.last_invalidated_at
                   )
                 )"#,
            user_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Set `last_pipeline_run_at = run_at` on successfully processed pictures.
    #[tracing::instrument(skip(ex, picture_ids))]
    pub async fn mark_run<'e, E>(
        ex: E,
        picture_ids: &[Uuid],
        run_at: NaiveDateTime,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if picture_ids.is_empty() {
            return Ok(());
        }
        sqlx::query!(
            r#"UPDATE pictures SET last_pipeline_run_at = $2 WHERE id = ANY($1::uuid[])"#,
            picture_ids as &[Uuid],
            run_at,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Reset `last_pipeline_run_at = NULL` on pictures that need re-evaluation.
    /// Called after manual tag changes.
    #[tracing::instrument(skip(ex, picture_ids))]
    pub async fn invalidate<'e, E>(ex: E, picture_ids: &[Uuid]) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if picture_ids.is_empty() {
            return Ok(());
        }
        sqlx::query!(
            r#"UPDATE pictures SET last_pipeline_run_at = NULL WHERE id = ANY($1::uuid[])"#,
            picture_ids as &[Uuid],
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Mark every picture of `user_id` carrying a tag at-or-under one of `tag_ltrees` dirty
    /// (`last_pipeline_run_at = NULL`). The `BEFORE UPDATE` trigger also bumps `updated_at`, which
    /// makes announce-stale any already-announced covered picture — the mechanism the tag-rename
    /// cascade (edge case §7) relies on to re-announce shares under a renamed tag. Returns the number
    /// of pictures marked.
    #[tracing::instrument(skip(ex, tag_ltrees), fields(user_id = %user_id))]
    pub async fn invalidate_under_tags<'e, E>(
        ex: E,
        user_id: Uuid,
        tag_ltrees: &[String],
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if tag_ltrees.is_empty() {
            return Ok(0);
        }
        let res = sqlx::query!(
            r#"UPDATE pictures p SET last_pipeline_run_at = NULL
               WHERE p.local_user_id = $1
                 AND EXISTS (
                   SELECT 1 FROM tags t
                   WHERE t.picture_id = p.id AND t.tag_path <@ ANY($2::text[]::ltree[])
                 )"#,
            user_id,
            tag_ltrees as &[String],
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected())
    }

    /// For each picture in the batch, return the set of `incoming_share_id` values
    /// from tags with `source = 'incoming_share'`. Used by the SharedTagMapping evaluator.
    #[tracing::instrument(skip(ex, picture_ids))]
    pub async fn find_incoming_share_ids<'e, E>(
        ex: E,
        picture_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<Uuid>>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if picture_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = sqlx::query!(
            r#"SELECT t.picture_id, t.source_id as "source_id!"
               FROM tags t
               WHERE t.picture_id = ANY($1::uuid[])
                 AND t.source = 'incoming_share'
                 AND t.source_id IS NOT NULL"#,
            picture_ids as &[Uuid],
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)?;

        let mut map: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for row in rows {
            map.entry(row.picture_id).or_default().push(row.source_id);
        }
        Ok(map)
    }

    /// Reconcile the pipeline tags of a single picture to `produced` in one atomic step.
    ///
    /// Pipeline tags (`rule`/`segment`/`share_mapping`) are live: this run's `produced` set is
    /// the complete desired pipeline output for the picture. Any stored pipeline tag not in it
    /// is removed (always-on removal — covers gating changes, edited rules, and tags left by
    /// now-disabled or deleted services), and the produced tags are inserted idempotently.
    ///
    /// `manual` and `incoming_share` tags are never touched. Because tags are stored per-source,
    /// each source owns its rows: removing one source's tag never disturbs another's, so no
    /// ancestor re-derivation is needed.
    ///
    /// Passing an empty `produced` is valid and clears all pipeline tags from the picture.
    #[tracing::instrument(skip(ex, produced), fields(picture_id = %picture_id))]
    pub async fn reconcile_pipeline_tags<'e, E>(
        ex: E,
        picture_id: Uuid,
        produced: &[PipelineTagAssignment],
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let tags: Vec<&str> = produced.iter().map(|a| a.tag_path.as_str()).collect();
        let sources: Vec<&str> = produced.iter().map(|a| a.source.as_str()).collect();
        let source_ids: Vec<Uuid> = produced.iter().map(|a| a.source_id).collect();

        sqlx::query!(
            r#"WITH produced AS (
                 SELECT t.tag::ltree AS tag_path, t.src::tag_source AS source, t.sid AS source_id
                 FROM unnest($2::text[], $3::text[], $4::uuid[]) AS t(tag, src, sid)
               ),
               cleanup AS (
                 DELETE FROM tags
                 WHERE picture_id = $1
                   AND source IN ('rule'::tag_source, 'segment'::tag_source, 'share_mapping'::tag_source)
                   AND NOT EXISTS (
                     SELECT 1 FROM produced p
                     WHERE p.tag_path = tags.tag_path
                       AND p.source = tags.source
                       AND p.source_id = tags.source_id
                   )
               )
               INSERT INTO tags (picture_id, tag_path, source, source_id)
               SELECT $1, p.tag_path, p.source, p.source_id FROM produced p
               ON CONFLICT (picture_id, tag_path, source, source_id) WHERE source <> 'manual' DO NOTHING"#,
            picture_id,
            &tags as &[&str],
            &sources as &[&str],
            &source_ids as &[Uuid],
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }
}
