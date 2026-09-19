use crate::domain::tag_metadata::{TagMetadata, TagOrder, TagSubtagPlacement, TagViewMode};
use archypix_common::error::{AppError, map_sqlx_error};
use sqlx::{Executor, Postgres};
use uuid::Uuid;

/// Columns as they come back from Postgres — `grouping` is JSONB and the enums are text-cast, so
/// the row is mapped to [`TagMetadata`] rather than derived onto it.
struct Row {
    tag_path: String,
    display_name: Option<String>,
    description: Option<String>,
    cover_picture_id: Option<Uuid>,
    color: Option<String>,
    date_from: Option<chrono::NaiveDateTime>,
    date_to: Option<chrono::NaiveDateTime>,
    show_when_empty: bool,
    sort_index: Option<i32>,
    children_order: TagOrder,
    children_order_desc: bool,
    view_mode: TagViewMode,
    subtag_placement: Option<TagSubtagPlacement>,
    grouping: serde_json::Value,
    webdav_dir_name: Option<String>,
}

impl From<Row> for TagMetadata {
    fn from(r: Row) -> Self {
        TagMetadata {
            tag_path: r.tag_path,
            display_name: r.display_name,
            description: r.description,
            cover_picture_id: r.cover_picture_id,
            color: r.color,
            date_from: r.date_from,
            date_to: r.date_to,
            show_when_empty: r.show_when_empty,
            sort_index: r.sort_index,
            children_order: r.children_order,
            children_order_desc: r.children_order_desc,
            view_mode: r.view_mode,
            subtag_placement: r.subtag_placement,
            // A row that fails to parse is a schema/version skew, not a user error — fall back to
            // the default rather than failing the whole listing.
            grouping: serde_json::from_value(r.grouping).unwrap_or_default(),
            webdav_dir_name: r.webdav_dir_name,
        }
    }
}

pub struct TagMetadataRepository;

impl TagMetadataRepository {
    /// Every row the user has.
    pub async fn list_for_user<'e, E>(ex: E, user_id: Uuid) -> Result<Vec<TagMetadata>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        Self::list(ex, user_id, None).await
    }

    /// The rows for a specific set of paths — the read half of a partial upsert, which merges onto
    /// the stored row before writing (§4.1).
    pub async fn find_many<'e, E>(
        ex: E,
        user_id: Uuid,
        paths: &[String],
    ) -> Result<Vec<TagMetadata>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if paths.is_empty() {
            return Ok(vec![]);
        }
        Self::list(ex, user_id, Some(paths)).await
    }

    /// `paths = None` reads the whole user; `Some` narrows to those paths.
    #[tracing::instrument(skip(ex, paths), fields(user_id = %user_id))]
    async fn list<'e, E>(
        ex: E,
        user_id: Uuid,
        paths: Option<&[String]>,
    ) -> Result<Vec<TagMetadata>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows = sqlx::query_as!(
            Row,
            r#"SELECT tag_path::text as "tag_path!", display_name, description, cover_picture_id,
                      color, date_from, date_to, show_when_empty, sort_index,
                      children_order as "children_order!: TagOrder", children_order_desc,
                      view_mode as "view_mode!: TagViewMode",
                      subtag_placement as "subtag_placement?: TagSubtagPlacement",
                      grouping, webdav_dir_name
               FROM tag_metadata
               WHERE user_id = $1
                 AND ($2::text[] IS NULL OR tag_path = ANY($2::text[]::ltree[]))"#,
            user_id,
            paths as Option<&[String]>,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(rows.into_iter().map(TagMetadata::from).collect())
    }

    /// Write whole rows (the caller has already merged its patch onto the stored row). One
    /// statement so a coalesced batch flush is one round trip (§4.1).
    #[tracing::instrument(skip(ex, rows), fields(user_id = %user_id, count = rows.len()))]
    pub async fn upsert_many<'e, E>(
        ex: E,
        user_id: Uuid,
        rows: &[TagMetadata],
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if rows.is_empty() {
            return Ok(0);
        }
        let paths: Vec<String> = rows.iter().map(|r| r.tag_path.clone()).collect();
        let display_names: Vec<Option<String>> =
            rows.iter().map(|r| r.display_name.clone()).collect();
        let descriptions: Vec<Option<String>> =
            rows.iter().map(|r| r.description.clone()).collect();
        let covers: Vec<Option<Uuid>> = rows.iter().map(|r| r.cover_picture_id).collect();
        let colors: Vec<Option<String>> = rows.iter().map(|r| r.color.clone()).collect();
        let date_from: Vec<Option<chrono::NaiveDateTime>> =
            rows.iter().map(|r| r.date_from).collect();
        let date_to: Vec<Option<chrono::NaiveDateTime>> = rows.iter().map(|r| r.date_to).collect();
        let show_when_empty: Vec<bool> = rows.iter().map(|r| r.show_when_empty).collect();
        let sort_index: Vec<Option<i32>> = rows.iter().map(|r| r.sort_index).collect();
        let children_order: Vec<TagOrder> = rows.iter().map(|r| r.children_order).collect();
        let children_order_desc: Vec<bool> =
            rows.iter().map(|r| r.children_order_desc).collect();
        let view_mode: Vec<TagViewMode> = rows.iter().map(|r| r.view_mode).collect();
        let subtag_placement: Vec<Option<TagSubtagPlacement>> =
            rows.iter().map(|r| r.subtag_placement).collect();
        let grouping: Vec<serde_json::Value> = rows
            .iter()
            .map(|r| serde_json::to_value(&r.grouping))
            .collect::<Result<_, _>>()
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        let webdav_dir_name: Vec<Option<String>> =
            rows.iter().map(|r| r.webdav_dir_name.clone()).collect();

        let res = sqlx::query!(
            r#"INSERT INTO tag_metadata
                   (user_id, tag_path, display_name, description, cover_picture_id, color,
                    date_from, date_to, show_when_empty, sort_index, children_order,
                    children_order_desc, view_mode, subtag_placement, grouping, webdav_dir_name,
                    updated_at)
               SELECT $1, t.path::ltree, t.display_name, t.description, t.cover, t.color,
                      t.date_from, t.date_to, t.show_when_empty, t.sort_index,
                      t.children_order, t.children_order_desc, t.view_mode, t.subtag_placement,
                      t.grouping, t.webdav_dir_name, now() AT TIME ZONE 'utc'
               FROM unnest($2::text[], $3::text[], $4::text[], $5::uuid[], $6::text[],
                           $7::timestamp[], $8::timestamp[], $9::bool[], $10::int[],
                           $11::tag_order[], $12::bool[], $13::tag_view_mode[],
                           $14::tag_subtag_placement[], $15::jsonb[], $16::text[])
                    AS t(path, display_name, description, cover, color, date_from, date_to,
                         show_when_empty, sort_index, children_order, children_order_desc,
                         view_mode, subtag_placement, grouping, webdav_dir_name)
               ON CONFLICT (user_id, tag_path) DO UPDATE SET
                   display_name        = EXCLUDED.display_name,
                   description         = EXCLUDED.description,
                   cover_picture_id    = EXCLUDED.cover_picture_id,
                   color               = EXCLUDED.color,
                   date_from           = EXCLUDED.date_from,
                   date_to             = EXCLUDED.date_to,
                   show_when_empty     = EXCLUDED.show_when_empty,
                   sort_index          = EXCLUDED.sort_index,
                   children_order      = EXCLUDED.children_order,
                   children_order_desc = EXCLUDED.children_order_desc,
                   view_mode           = EXCLUDED.view_mode,
                   subtag_placement    = EXCLUDED.subtag_placement,
                   grouping            = EXCLUDED.grouping,
                   webdav_dir_name     = EXCLUDED.webdav_dir_name,
                   updated_at          = EXCLUDED.updated_at"#,
            user_id,
            &paths,
            &display_names as &[Option<String>],
            &descriptions as &[Option<String>],
            &covers as &[Option<Uuid>],
            &colors as &[Option<String>],
            &date_from as &[Option<chrono::NaiveDateTime>],
            &date_to as &[Option<chrono::NaiveDateTime>],
            &show_when_empty,
            &sort_index as &[Option<i32>],
            &children_order as &[TagOrder],
            &children_order_desc,
            &view_mode as &[TagViewMode],
            &subtag_placement as &[Option<TagSubtagPlacement>],
            &grouping,
            &webdav_dir_name as &[Option<String>],
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected())
    }

    #[tracing::instrument(skip(ex, paths), fields(user_id = %user_id, count = paths.len()))]
    pub async fn delete_many<'e, E>(
        ex: E,
        user_id: Uuid,
        paths: &[String],
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if paths.is_empty() {
            return Ok(0);
        }
        let res = sqlx::query!(
            "DELETE FROM tag_metadata WHERE user_id = $1 AND tag_path = ANY($2::text[]::ltree[])",
            user_id,
            paths as &[String],
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected())
    }

    /// ltree prefix swap for the rename cascade (§12), shaped like
    /// [`TagRepository::rename_manual_subtree`](crate::repository::tag::TagRepository::rename_manual_subtree).
    /// On PK collision the **target row wins and the source is dropped**. The root row is excluded:
    /// `'' @> anything` is true, so a swap rooted there would rewrite every row the user has.
    #[tracing::instrument(skip(ex), fields(user_id = %user_id))]
    pub async fn rename_subtree<'e, E>(
        ex: E,
        user_id: Uuid,
        old_ltree: &str,
        new_ltree: &str,
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if old_ltree.is_empty() || new_ltree.is_empty() {
            return Ok(0);
        }
        // Disjoint row sets, as in `rename_manual_subtree`: `dedup` drops source rows whose renamed
        // path already exists, the UPDATE renames the rest.
        let res = sqlx::query!(
            r#"WITH dedup AS (
                 DELETE FROM tag_metadata t
                 WHERE t.user_id = $1
                   AND nlevel(t.tag_path) > 0
                   AND t.tag_path <@ $2::text::ltree
                   AND EXISTS (
                     SELECT 1 FROM tag_metadata e
                     WHERE e.user_id = t.user_id
                       AND e.tag_path = CASE WHEN t.tag_path = $2::text::ltree THEN $3::text::ltree
                                             ELSE $3::text::ltree || subpath(t.tag_path, nlevel($2::text::ltree)) END
                   )
               )
               UPDATE tag_metadata
               SET tag_path = CASE WHEN tag_path = $2::text::ltree THEN $3::text::ltree
                                   ELSE $3::text::ltree || subpath(tag_path, nlevel($2::text::ltree)) END,
                   updated_at = now() AT TIME ZONE 'utc'
               WHERE user_id = $1
                 AND nlevel(tag_path) > 0
                 AND tag_path <@ $2::text::ltree
                 AND NOT EXISTS (
                   SELECT 1 FROM tag_metadata e
                   WHERE e.user_id = tag_metadata.user_id
                     AND e.tag_path = CASE WHEN tag_metadata.tag_path = $2::text::ltree THEN $3::text::ltree
                                           ELSE $3::text::ltree || subpath(tag_metadata.tag_path, nlevel($2::text::ltree)) END
                 )"#,
            user_id,
            old_ltree,
            new_ltree,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected())
    }
}

