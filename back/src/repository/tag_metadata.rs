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
                      children_order as "children_order!: TagOrder",
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
                    date_from, date_to, show_when_empty, sort_index, children_order, view_mode,
                    subtag_placement, grouping, webdav_dir_name, updated_at)
               SELECT $1, t.path::ltree, t.display_name, t.description, t.cover, t.color,
                      t.date_from, t.date_to, t.show_when_empty, t.sort_index,
                      t.children_order, t.view_mode, t.subtag_placement, t.grouping,
                      t.webdav_dir_name, now() AT TIME ZONE 'utc'
               FROM unnest($2::text[], $3::text[], $4::text[], $5::uuid[], $6::text[],
                           $7::timestamp[], $8::timestamp[], $9::bool[], $10::int[],
                           $11::tag_order[], $12::tag_view_mode[], $13::tag_subtag_placement[],
                           $14::jsonb[], $15::text[])
                    AS t(path, display_name, description, cover, color, date_from, date_to,
                         show_when_empty, sort_index, children_order, view_mode, subtag_placement,
                         grouping, webdav_dir_name)
               ON CONFLICT (user_id, tag_path) DO UPDATE SET
                   display_name     = EXCLUDED.display_name,
                   description      = EXCLUDED.description,
                   cover_picture_id = EXCLUDED.cover_picture_id,
                   color            = EXCLUDED.color,
                   date_from        = EXCLUDED.date_from,
                   date_to          = EXCLUDED.date_to,
                   show_when_empty  = EXCLUDED.show_when_empty,
                   sort_index       = EXCLUDED.sort_index,
                   children_order   = EXCLUDED.children_order,
                   view_mode        = EXCLUDED.view_mode,
                   subtag_placement = EXCLUDED.subtag_placement,
                   grouping         = EXCLUDED.grouping,
                   webdav_dir_name  = EXCLUDED.webdav_dir_name,
                   updated_at       = EXCLUDED.updated_at"#,
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

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

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

    fn named(path: &str, name: &str) -> TagMetadata {
        TagMetadata {
            display_name: Some(name.to_string()),
            ..TagMetadata::new(path.to_string())
        }
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn upsert_then_list_roundtrips_every_field(db: PgPool) {
        let user = seed_user(&db).await;
        let row = TagMetadata {
            display_name: Some("Vietnam 🇻🇳".into()),
            description: Some("Two weeks".into()),
            color: Some("#a1b2c3".into()),
            date_from: Some("2026-08-01T09:12:00".parse().unwrap()),
            show_when_empty: true,
            sort_index: Some(300),
            children_order: TagOrder::DateFrom,
            view_mode: TagViewMode::All,
            subtag_placement: Some(TagSubtagPlacement::InSections),
            grouping: serde_json::from_value(serde_json::json!({"captured_at": {"kind": "year"}}))
                .unwrap(),
            webdav_dir_name: Some("Vietnam 2026".into()),
            ..TagMetadata::new("Era.2026.Vietnam".into())
        };
        TagMetadataRepository::upsert_many(&db, user, &[row.clone()])
            .await
            .unwrap();

        let stored = TagMetadataRepository::list_for_user(&db, user).await.unwrap();
        assert_eq!(stored, vec![row]);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn upsert_overwrites_the_whole_row(db: PgPool) {
        let user = seed_user(&db).await;
        TagMetadataRepository::upsert_many(&db, user, &[named("A", "first")])
            .await
            .unwrap();
        TagMetadataRepository::upsert_many(&db, user, &[named("A", "second")])
            .await
            .unwrap();
        let stored = TagMetadataRepository::list_for_user(&db, user).await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].display_name.as_deref(), Some("second"));
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn root_row_is_storable_and_findable(db: PgPool) {
        // §3.3: `tag_path = ''` is an ordinary row reached by exactly the same code.
        let user = seed_user(&db).await;
        let root = TagMetadata {
            view_mode: TagViewMode::Direct,
            ..TagMetadata::new(String::new())
        };
        TagMetadataRepository::upsert_many(&db, user, &[root.clone()])
            .await
            .unwrap();
        let found = TagMetadataRepository::find_many(&db, user, &[String::new()])
            .await
            .unwrap();
        assert_eq!(found, vec![root]);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn rename_subtree_swaps_the_prefix(db: PgPool) {
        let user = seed_user(&db).await;
        TagMetadataRepository::upsert_many(
            &db,
            user,
            &[
                named("Era.Travel", "Travel"),
                named("Era.Travel.Alps", "Alps"),
                named("Era.Other", "Other"),
            ],
        )
        .await
        .unwrap();

        TagMetadataRepository::rename_subtree(&db, user, "Era.Travel", "Trips.2024")
            .await
            .unwrap();

        let mut paths: Vec<String> = TagMetadataRepository::list_for_user(&db, user)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.tag_path)
            .collect();
        paths.sort();
        assert_eq!(paths, vec!["Era.Other", "Trips.2024", "Trips.2024.Alps"]);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn rename_subtree_target_wins_on_collision(db: PgPool) {
        let user = seed_user(&db).await;
        TagMetadataRepository::upsert_many(
            &db,
            user,
            &[named("Era.Travel", "source"), named("Era.Trips", "target")],
        )
        .await
        .unwrap();

        TagMetadataRepository::rename_subtree(&db, user, "Era.Travel", "Era.Trips")
            .await
            .unwrap();

        let stored = TagMetadataRepository::list_for_user(&db, user).await.unwrap();
        assert_eq!(stored.len(), 1, "the source row was dropped");
        assert_eq!(stored[0].display_name.as_deref(), Some("target"));
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn rename_subtree_never_moves_the_root_row(db: PgPool) {
        // §12: `'' @> anything`, so an unguarded swap would rewrite the whole table.
        let user = seed_user(&db).await;
        TagMetadataRepository::upsert_many(
            &db,
            user,
            &[named("", "root"), named("Era.Travel", "Travel")],
        )
        .await
        .unwrap();

        TagMetadataRepository::rename_subtree(&db, user, "", "Moved")
            .await
            .unwrap();
        TagMetadataRepository::rename_subtree(&db, user, "Era", "Trips")
            .await
            .unwrap();

        let mut paths: Vec<String> = TagMetadataRepository::list_for_user(&db, user)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.tag_path)
            .collect();
        paths.sort();
        assert_eq!(paths, vec!["", "Trips.Travel"]);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn delete_many_removes_only_the_named_paths(db: PgPool) {
        let user = seed_user(&db).await;
        TagMetadataRepository::upsert_many(&db, user, &[named("A", "a"), named("A.B", "b")])
            .await
            .unwrap();
        TagMetadataRepository::delete_many(&db, user, &["A".to_string()])
            .await
            .unwrap();
        let stored = TagMetadataRepository::list_for_user(&db, user).await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].tag_path, "A.B");
    }
}
