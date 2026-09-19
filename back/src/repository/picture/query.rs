use crate::domain::hierarchy::TagPredicate;
use crate::domain::picture::Picture;
use archypix_common::error::{AppError, map_sqlx_error};
use sqlx::{PgPool, Postgres};
use uuid::Uuid;
use super::*;

impl PictureRepository {
    #[tracing::instrument(skip(db, filter), fields(user_id = %local_user_id))]
    pub async fn list(
        db: &PgPool,
        local_user_id: Uuid,
        filter: &PictureListFilter,
    ) -> Result<(Vec<Picture>, i64), AppError> {
        filter.validate()?;
        let sort_dir = match filter.order {
            SortOrder::Asc => "ASC",
            SortOrder::Desc => "DESC",
        };
        let offset = (filter.page - 1) * filter.page_size;

        let total: i64 = {
            let mut q = sqlx::QueryBuilder::<Postgres>::new(
                "SELECT COUNT(*) FROM pictures p WHERE p.local_user_id = ",
            );
            q.push_bind(local_user_id);
            Self::push_filters(&mut q, filter);
            q.build_query_scalar()
                .fetch_one(db)
                .await
                .map_err(map_sqlx_error)?
        };

        let items: Vec<Picture> = {
            let mut q = sqlx::QueryBuilder::<Postgres>::new(
                r#"SELECT p.id, p.local_user_id, p.remote_picture_id, p.owner_username,
                          p.owner_instance_domain, p.filename, p.mime_type, p.file_size,
                          p.width, p.height, p.exif_data, p.metadata,
                          p.deleted_at, p.deleted_reason, p.owner_deleted_at, p.owner_purge_at,
                          p.remote_exif_data, p.local_exif_overrides,
                          p.captured_at, p.ingested_at, p.updated_at, p.remote_updated_at,
                          p.blurhash, p.gps_lat, p.gps_lng, p.gps_alt, p.orientation,
                          p.thumbnails_generated_at, p.file_hash,
                          p.exif_sync_status, p.file_exif,
                          p.content_hash, p.copy_source_owner_username,
                          p.copy_source_owner_instance, p.copy_source_picture_id,
                          p.creator, p.creator_override, p.original_file_created_at, p.file_modified_at
                   FROM pictures p WHERE p.local_user_id = "#,
            );
            q.push_bind(local_user_id);
            Self::push_filters(&mut q, filter);
            Self::push_order_by(&mut q, filter, sort_dir);
            q.push(" LIMIT ");
            q.push_bind(filter.page_size);
            q.push(" OFFSET ");
            q.push_bind(offset);
            q.build_query_as()
                .fetch_all(db)
                .await
                .map_err(map_sqlx_error)?
        };

        Ok((items, total))
    }

    /// Count pictures matching `filter` (no pagination). Used by the hierarchy `tree` endpoint's
    /// per-directory `picture_count` / empty-directory pruning.
    #[tracing::instrument(skip(db, filter), fields(user_id = %local_user_id))]
    pub async fn count(
        db: &PgPool,
        local_user_id: Uuid,
        filter: &PictureListFilter,
    ) -> Result<i64, AppError> {
        let mut q = sqlx::QueryBuilder::<Postgres>::new(
            "SELECT COUNT(*) FROM pictures p WHERE p.local_user_id = ",
        );
        q.push_bind(local_user_id);
        Self::push_filters(&mut q, filter);
        q.build_query_scalar()
            .fetch_one(db)
            .await
            .map_err(map_sqlx_error)
    }

    /// Emit the `ORDER BY` clause. Column sorts use `NULLS LAST` + the `p.id` tiebreaker (total
    /// order ⇒ stable pagination). Proximity sorts (feature 29 §6) are always nearest-first,
    /// sort field-missing rows last, and ignore `SortOrder`; the reference params are bound.
    fn push_order_by(
        q: &mut sqlx::QueryBuilder<Postgres>,
        filter: &PictureListFilter,
        sort_dir: &str,
    ) {
        match filter.sort {
            PictureSortField::TimeNear => {
                // |captured_at − near_time|. Undated rows are already excluded (push_filters).
                q.push(" ORDER BY abs(extract(epoch FROM (p.captured_at - ");
                q.push_bind(filter.near_time);
                q.push("))) ASC, p.id ASC");
            }
            PictureSortField::GeoNear => {
                // Haversine central-angle term `a` (§6): monotonic with true great-circle distance,
                // so exact for a sort while skipping the final `asin`/`R` scaling; `sin²(Δlng/2)`
                // wraps the antimeridian correctly. Ungeotagged rows are excluded (push_filters).
                q.push(" ORDER BY sin(radians(p.gps_lat - ");
                q.push_bind(filter.near_lat);
                q.push(")/2)^2 + cos(radians(");
                q.push_bind(filter.near_lat);
                q.push(")) * cos(radians(p.gps_lat)) * sin(radians(p.gps_lng - ");
                q.push_bind(filter.near_lng);
                q.push(")/2)^2 ASC, p.id ASC");
            }
            _ => {
                let sort_col = match filter.sort {
                    PictureSortField::CapturedAt => "p.captured_at",
                    PictureSortField::IngestedAt => "p.ingested_at",
                    PictureSortField::UpdatedAt => "p.updated_at",
                    PictureSortField::FileSize => "p.file_size",
                    PictureSortField::Filename => "p.filename",
                    // Proximity variants handled above.
                    PictureSortField::TimeNear | PictureSortField::GeoNear => unreachable!(),
                };
                // Date-fix mode (feature 30 §4): undated rows first, then the column sort, then the
                // load-bearing `filename, id` tiebreak (undated rows have no captured_at to order by,
                // and run interpolation relies on a stable filename-contiguous order across pages).
                let missing_first = if filter.undated_first {
                    "(p.captured_at IS NULL) DESC, "
                } else {
                    ""
                };
                let tiebreak = if filter.undated_first {
                    format!(", p.filename {sort_dir}")
                } else {
                    String::new()
                };
                q.push(format!(
                    " ORDER BY {missing_first}{sort_col} {sort_dir} NULLS LAST{tiebreak}, p.id {sort_dir}"
                ));
            }
        }
    }

    pub(super) fn push_filters(q: &mut sqlx::QueryBuilder<Postgres>, filter: &PictureListFilter) {
        // Content-dedup rows (`content_dedupe`/`boomerang`) are internal hidden state — they never
        // surface in gallery or trash listings, only via the per-picture copies endpoint. Any state
        // that admits trashed rows therefore shows `manual`-trashed only, so a rejected content group
        // shows exactly one recoverable entry rather than a pile of duplicates.
        match filter.trash {
            TrashFilter::Exclude => {
                q.push(" AND p.deleted_at IS NULL");
            }
            TrashFilter::Include => {
                q.push(" AND (p.deleted_at IS NULL OR p.deleted_reason = 'manual'::picture_deleted_reason)");
            }
            TrashFilter::Only => {
                q.push(" AND p.deleted_at IS NOT NULL AND p.deleted_reason = 'manual'::picture_deleted_reason");
            }
        }
        if filter.owned_only {
            q.push(" AND p.remote_picture_id IS NULL");
        }
        if filter.shared_with_me {
            q.push(" AND p.remote_picture_id IS NOT NULL");
        }
        if let Some(v) = filter.captured_after {
            q.push(" AND p.captured_at >= ").push_bind(v);
        }
        if let Some(v) = filter.captured_before {
            q.push(" AND p.captured_at <= ").push_bind(v);
        }
        // Presence filters (feature 29 §4). `missing_any` is the OR convenience; the per-field
        // arms are AND-composed (mutual exclusion enforced by `PictureListFilter::validate`).
        if filter.missing_any {
            q.push(" AND (p.captured_at IS NULL OR p.gps_lat IS NULL OR p.gps_lng IS NULL)");
        } else {
            match filter.gps {
                PresenceFilter::Any => {}
                PresenceFilter::Present => {
                    q.push(" AND p.gps_lat IS NOT NULL AND p.gps_lng IS NOT NULL");
                }
                PresenceFilter::Missing => {
                    q.push(" AND (p.gps_lat IS NULL OR p.gps_lng IS NULL)");
                }
            }
            match filter.capture_date {
                PresenceFilter::Any => {}
                PresenceFilter::Present => {
                    q.push(" AND p.captured_at IS NOT NULL");
                }
                PresenceFilter::Missing => {
                    q.push(" AND p.captured_at IS NULL");
                }
            }
        }
        // A proximity sort is meaningless for rows missing its field — exclude them entirely
        // (feature 29 §6) rather than trailing them at the end of the page.
        match filter.sort {
            PictureSortField::TimeNear => {
                q.push(" AND p.captured_at IS NOT NULL");
            }
            PictureSortField::GeoNear => {
                q.push(" AND p.gps_lat IS NOT NULL AND p.gps_lng IS NOT NULL");
            }
            _ => {}
        }
        if let Some(ref predicate) = filter.predicate {
            q.push(" AND ");
            Self::render_predicate(q, predicate);
        }
    }

    /// Render a [`TagPredicate`] to a SQL boolean over `pictures p`. Recursive: `minus_children`
    /// are negated sub-predicates ("most-specific node wins"). See `TagPredicate` docs for the
    /// membership semantics.
    fn render_predicate(q: &mut sqlx::QueryBuilder<Postgres>, pred: &TagPredicate) {
        q.push("(");
        // `untagged` is a conjunct, not a branch: the gallery layers its cross-cutting include /
        // exclude sets onto every query, root included (feature 35 §7).
        let has_positive = !pred.include.is_empty() || !pred.exact.is_empty();
        if pred.untagged {
            q.push("NOT EXISTS (SELECT 1 FROM tags t WHERE t.picture_id = p.id)");
            if has_positive {
                q.push(" AND ");
            }
        }
        if has_positive {
            let joiner = if pred.match_all { " AND " } else { " OR " };
            q.push("(");
            let mut first = true;
            for inc in &pred.include {
                if !first {
                    q.push(joiner);
                }
                first = false;
                q.push("EXISTS (SELECT 1 FROM tags t WHERE t.picture_id = p.id AND t.tag_path <@ ")
                    .push_bind(inc.as_ltree().to_string())
                    .push("::ltree)");
            }
            for ex in &pred.exact {
                if !first {
                    q.push(joiner);
                }
                first = false;
                q.push("EXISTS (SELECT 1 FROM tags t WHERE t.picture_id = p.id AND t.tag_path = ")
                    .push_bind(ex.as_ltree().to_string())
                    .push("::ltree)");
            }
            q.push(")");
        } else if !pred.untagged {
            // No positive arms ⇒ membership is vacuously true (all pictures).
            q.push("TRUE");
        }
        for ex in &pred.exclude {
            q.push(" AND NOT EXISTS (SELECT 1 FROM tags t WHERE t.picture_id = p.id AND t.tag_path <@ ")
                .push_bind(ex.as_ltree().to_string())
                .push("::ltree)");
        }
        for term in &pred.and_terms {
            q.push(" AND ");
            Self::render_predicate(q, term);
        }
        for child in &pred.minus_children {
            q.push(" AND NOT ");
            Self::render_predicate(q, child);
        }
        q.push(")");
    }

}
