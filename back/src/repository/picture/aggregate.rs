use archypix_common::error::{AppError, map_sqlx_error};
use sqlx::{PgPool, Postgres};
use uuid::Uuid;
use super::*;

impl PictureRepository {
    /// Compute the [`SelectionSummary`] (feature 14 §4.1) — all from the `pictures` row (no joins).
    #[tracing::instrument(skip(db, sel), fields(user_id = %local_user_id))]
    pub async fn aggregate_summary(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
    ) -> Result<SelectionSummary, AppError> {
        if sel.is_empty() {
            return Ok(SelectionSummary::default());
        }

        // Scalar aggregates (one row).
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            r#"SELECT
                 COUNT(*)::bigint AS count,
                 COUNT(*) FILTER (WHERE p.remote_picture_id IS NULL)::bigint AS owned_count,
                 COUNT(*) FILTER (WHERE p.remote_picture_id IS NOT NULL)::bigint AS received_count,
                 COALESCE(SUM(p.file_size), 0)::bigint AS total_file_size,
                 COUNT(*) FILTER (WHERE p.deleted_at IS NOT NULL)::bigint AS trashed_count,
                 COUNT(*) FILTER (WHERE p.owner_deleted_at IS NOT NULL)::bigint AS owner_deleting_count,
                 COUNT(*) FILTER (WHERE p.thumbnails_generated_at IS NULL)::bigint AS thumbnail_pending_count
               FROM pictures p WHERE "#,
        );
        Self::push_selection_where(&mut q, local_user_id, sel);
        let row = q.build().fetch_one(db).await.map_err(map_sqlx_error)?;
        use sqlx::Row;
        let mut summary = SelectionSummary {
            count: row.try_get("count").map_err(map_sqlx_error)?,
            owned_count: row.try_get("owned_count").map_err(map_sqlx_error)?,
            received_count: row.try_get("received_count").map_err(map_sqlx_error)?,
            total_file_size: row.try_get("total_file_size").map_err(map_sqlx_error)?,
            trashed_count: row.try_get("trashed_count").map_err(map_sqlx_error)?,
            owner_deleting_count: row
                .try_get("owner_deleting_count")
                .map_err(map_sqlx_error)?,
            thumbnail_pending_count: row
                .try_get("thumbnail_pending_count")
                .map_err(map_sqlx_error)?,
            ..Default::default()
        };

        // Duplicate count: pictures sharing a file_hash with another in the selection.
        let mut dq = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT COALESCE(SUM(c), 0)::bigint FROM (SELECT COUNT(*) AS c FROM pictures p WHERE p.file_hash IS NOT NULL AND ",
        );
        Self::push_selection_where(&mut dq, local_user_id, sel);
        dq.push(" GROUP BY p.file_hash HAVING COUNT(*) > 1) g");
        summary.duplicate_count = dq
            .build_query_scalar()
            .fetch_one(db)
            .await
            .map_err(map_sqlx_error)?;

        // exif_sync histogram.
        let mut hq = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT p.exif_sync_status::text AS status, COUNT(*)::bigint AS cnt FROM pictures p WHERE ",
        );
        Self::push_selection_where(&mut hq, local_user_id, sel);
        hq.push(" GROUP BY p.exif_sync_status");
        let rows = hq.build().fetch_all(db).await.map_err(map_sqlx_error)?;
        for r in rows {
            let label: String = r.try_get("status").map_err(map_sqlx_error)?;
            let cnt: i64 = r.try_get("cnt").map_err(map_sqlx_error)?;
            if let Some(status) = parse_exif_sync_status(&label) {
                summary.exif_sync.push((status, cnt));
            }
        }

        // Distinct remote owners of received pictures.
        let mut oq = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT p.owner_username AS username, p.owner_instance_domain AS instance, COUNT(*)::bigint AS cnt \
             FROM pictures p WHERE p.remote_picture_id IS NOT NULL AND ",
        );
        Self::push_selection_where(&mut oq, local_user_id, sel);
        oq.push(" GROUP BY p.owner_username, p.owner_instance_domain ORDER BY cnt DESC");
        let rows = oq.build().fetch_all(db).await.map_err(map_sqlx_error)?;
        for r in rows {
            let username: Option<String> = r.try_get("username").map_err(map_sqlx_error)?;
            let instance: Option<String> = r.try_get("instance").map_err(map_sqlx_error)?;
            let count: i64 = r.try_get("cnt").map_err(map_sqlx_error)?;
            summary.owners.push(OwnerCount {
                username: username.unwrap_or_default(),
                instance: instance.unwrap_or_default(),
                count,
            });
        }

        Ok(summary)
    }

    /// Per-field numeric aggregates (min/max/avg/null_count) over the selection, one row per field.
    /// `fields` is `(field_name, SQL value expression)`; expressions are trusted constants (never
    /// user input).
    #[tracing::instrument(skip(db, sel, fields), fields(user_id = %local_user_id))]
    pub async fn aggregate_numeric(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
        fields: &[(&str, &str)],
    ) -> Result<Vec<(String, NumericAgg)>, AppError> {
        if sel.is_empty() || fields.is_empty() {
            return Ok(vec![]);
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new("");
        for (i, (name, expr)) in fields.iter().enumerate() {
            if i > 0 {
                q.push(" UNION ALL ");
            }
            q.push("SELECT '")
                .push(name)
                .push("' AS field, MIN(v)::float8 AS min_v, MAX(v)::float8 AS max_v, \
                       AVG(v)::float8 AS avg_v, (COUNT(*) FILTER (WHERE v IS NULL))::bigint AS null_count \
                       FROM (SELECT ")
                .push(expr)
                .push(" AS v FROM pictures p WHERE ");
            Self::push_selection_where(&mut q, local_user_id, sel);
            q.push(") s");
        }
        let rows = q.build().fetch_all(db).await.map_err(map_sqlx_error)?;
        use sqlx::Row;
        let mut out = Vec::new();
        for r in rows {
            out.push((
                r.try_get::<String, _>("field").map_err(map_sqlx_error)?,
                NumericAgg {
                    min: r.try_get("min_v").map_err(map_sqlx_error)?,
                    max: r.try_get("max_v").map_err(map_sqlx_error)?,
                    avg: r.try_get("avg_v").map_err(map_sqlx_error)?,
                    null_count: r.try_get("null_count").map_err(map_sqlx_error)?,
                },
            ));
        }
        Ok(out)
    }

    /// Per-field date aggregates (min/max range + avg instant + null_count), one row per field.
    #[tracing::instrument(skip(db, sel, fields), fields(user_id = %local_user_id))]
    pub async fn aggregate_dates(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
        fields: &[(&str, &str)],
    ) -> Result<Vec<(String, DateAgg)>, AppError> {
        if sel.is_empty() || fields.is_empty() {
            return Ok(vec![]);
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new("");
        for (i, (name, expr)) in fields.iter().enumerate() {
            if i > 0 {
                q.push(" UNION ALL ");
            }
            q.push("SELECT '")
                .push(name)
                .push(
                    "' AS field, MIN(v) AS min_v, MAX(v) AS max_v, \
                       (to_timestamp(AVG(EXTRACT(EPOCH FROM v))) AT TIME ZONE 'utc') AS avg_v, \
                       (COUNT(*) FILTER (WHERE v IS NULL))::bigint AS null_count \
                       FROM (SELECT ",
                )
                .push(expr)
                .push(" AS v FROM pictures p WHERE ");
            Self::push_selection_where(&mut q, local_user_id, sel);
            q.push(") s");
        }
        let rows = q.build().fetch_all(db).await.map_err(map_sqlx_error)?;
        use sqlx::Row;
        let mut out = Vec::new();
        for r in rows {
            out.push((
                r.try_get::<String, _>("field").map_err(map_sqlx_error)?,
                DateAgg {
                    min: r.try_get("min_v").map_err(map_sqlx_error)?,
                    max: r.try_get("max_v").map_err(map_sqlx_error)?,
                    avg: r.try_get("avg_v").map_err(map_sqlx_error)?,
                    null_count: r.try_get("null_count").map_err(map_sqlx_error)?,
                },
            ));
        }
        Ok(out)
    }

    /// GPS bounding box + centroid over the selection.
    #[tracing::instrument(skip(db, sel), fields(user_id = %local_user_id))]
    pub async fn aggregate_gps(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
    ) -> Result<GpsAgg, AppError> {
        if sel.is_empty() {
            return Ok(GpsAgg::default());
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            r#"SELECT MIN(p.gps_lat)::float8 AS lat_min, MAX(p.gps_lat)::float8 AS lat_max,
                      MIN(p.gps_lng)::float8 AS lng_min, MAX(p.gps_lng)::float8 AS lng_max,
                      AVG(p.gps_lat)::float8 AS clat, AVG(p.gps_lng)::float8 AS clng,
                      (COUNT(*) FILTER (WHERE p.gps_lat IS NULL OR p.gps_lng IS NULL))::bigint AS null_count
               FROM pictures p WHERE "#,
        );
        Self::push_selection_where(&mut q, local_user_id, sel);
        let r = q.build().fetch_one(db).await.map_err(map_sqlx_error)?;
        use sqlx::Row;
        Ok(GpsAgg {
            lat_min: r.try_get("lat_min").map_err(map_sqlx_error)?,
            lat_max: r.try_get("lat_max").map_err(map_sqlx_error)?,
            lng_min: r.try_get("lng_min").map_err(map_sqlx_error)?,
            lng_max: r.try_get("lng_max").map_err(map_sqlx_error)?,
            centroid_lat: r.try_get("clat").map_err(map_sqlx_error)?,
            centroid_lng: r.try_get("clng").map_err(map_sqlx_error)?,
            null_count: r.try_get("null_count").map_err(map_sqlx_error)?,
        })
    }

    /// Distinct-value histogram of a string/enum field (`expr` is a trusted SQL value expression).
    #[tracing::instrument(skip(db, sel), fields(user_id = %local_user_id))]
    pub async fn aggregate_distinct(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
        expr: &str,
    ) -> Result<DistinctAgg, AppError> {
        if sel.is_empty() {
            return Ok(DistinctAgg::default());
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT v AS value, COUNT(*)::bigint AS cnt FROM (SELECT ",
        );
        q.push(expr).push(" AS v FROM pictures p WHERE ");
        Self::push_selection_where(&mut q, local_user_id, sel);
        q.push(") s GROUP BY v ORDER BY cnt DESC");
        Self::fold_distinct(db, q).await
    }

    /// Distinct-value histogram of the **resolved displayed creator** (feature 26): `coalesce(
    /// creator_override, creator, owner_default)`, where the owner default is `owner_default_identity`
    /// for owned rows and the stored origin owner for received rows. `null_count` = unresolvable rows.
    #[tracing::instrument(skip(db, sel), fields(user_id = %local_user_id))]
    pub async fn aggregate_creator(
        db: &PgPool,
        local_user_id: Uuid,
        sel: &ResolvedSelection,
        owner_default_identity: &str,
    ) -> Result<DistinctAgg, AppError> {
        if sel.is_empty() {
            return Ok(DistinctAgg::default());
        }
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT v AS value, COUNT(*)::bigint AS cnt FROM (SELECT COALESCE(\
             NULLIF(p.creator_override, ''), NULLIF(p.creator, ''), \
             CASE WHEN p.remote_picture_id IS NULL THEN ",
        );
        q.push_bind(owner_default_identity.to_string());
        q.push(
            " WHEN COALESCE(p.owner_username, '') <> '' \
             THEN '@' || p.owner_username || ':' || COALESCE(p.owner_instance_domain, '') \
             ELSE NULL END) AS v FROM pictures p WHERE ",
        );
        Self::push_selection_where(&mut q, local_user_id, sel);
        q.push(") s GROUP BY v ORDER BY cnt DESC");
        Self::fold_distinct(db, q).await
    }

    /// Run a `(value, cnt)` distinct query built by the caller and fold it into a [`DistinctAgg`]
    /// (NULLs land in `null_count`). Shared by [`aggregate_distinct`] and [`aggregate_creator`].
    async fn fold_distinct(
        db: &PgPool,
        mut q: sqlx::QueryBuilder<Postgres>,
    ) -> Result<DistinctAgg, AppError> {
        let rows = q.build().fetch_all(db).await.map_err(map_sqlx_error)?;
        use sqlx::Row;
        let mut agg = DistinctAgg::default();
        for r in rows {
            let value: Option<String> = r.try_get("value").map_err(map_sqlx_error)?;
            let cnt: i64 = r.try_get("cnt").map_err(map_sqlx_error)?;
            match value {
                Some(v) => agg.values.push((v, cnt)),
                None => agg.null_count = cnt,
            }
        }
        Ok(agg)
    }

}
