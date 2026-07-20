//! Type-aware aggregation over a [`PictureSelection`](crate::services::selection::PictureSelection)
//! for the multi-select panel (feature 14 §4). Computed server-side with GROUP BY / conditional
//! aggregates, so a select-all of 10k pictures is never materialised or downloaded.
//!
//! The `sections` request field keeps the sidebar cheap: the panel fetches `summary` immediately and
//! requests the heavier `tags` (ltree ancestor expansion) / `exif` (per-field) sections only when
//! those foldable sections are expanded.

use crate::domain::picture::{ExifSyncStatus, format_identity};
use crate::domain::tag::TagPath;
use crate::infra::settings::keys;
use crate::repository::picture::{DistinctAgg, PictureRepository};
use crate::repository::tag::TagRepository;
use crate::services::selection::{self, PictureSelection};
use archypix_common::error::AppError;
use archypix_common::settings::Settings;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use uuid::Uuid;

/// Which aggregate sections to compute. `summary` is always cheap and on by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateSection {
    Summary,
    Tags,
    Exif,
}

#[derive(Debug, Deserialize)]
pub struct AggregateRequest {
    pub selection: PictureSelection,
    #[serde(default)]
    pub sections: Option<Vec<AggregateSection>>,
    #[serde(default)]
    pub tag_provenance: bool,
}

/// Distinct cap for string/enum fields (§4.3): the first N inline, the rest counted as overflow.
const DISTINCT_CAP: usize = 10;

/// Camera/lens fields aggregated as distinct-value sets, plus their `exif_data` JSONB expression.
const DISTINCT_FIELDS: &[(&str, &str)] = &[
    ("camera_brand", "(p.exif_data->>'camera_brand')"),
    ("camera_model", "(p.exif_data->>'camera_model')"),
    ("mime_type", "p.mime_type"),
];

/// Numeric fields aggregated as min/max/avg, with their SQL value expression (cast to float8).
const NUMERIC_FIELDS: &[(&str, &str)] = &[
    ("iso_speed", "(p.exif_data->>'iso_speed')::float8"),
    ("f_number", "(p.exif_data->>'f_number')::float8"),
    (
        "focal_length_mm",
        "(p.exif_data->>'focal_length_mm')::float8",
    ),
    (
        "exposure_time_num",
        "(p.exif_data->>'exposure_time_num')::float8",
    ),
    (
        "exposure_time_den",
        "(p.exif_data->>'exposure_time_den')::float8",
    ),
    ("file_size", "p.file_size::float8"),
    ("width", "p.width::float8"),
    ("height", "p.height::float8"),
    ("orientation", "p.orientation::float8"),
    ("gps_alt", "p.gps_alt::float8"),
];

/// Date fields aggregated as min/max range + avg instant.
const DATE_FIELDS: &[(&str, &str)] = &[
    ("captured_at", "p.captured_at"),
    ("ingested_at", "p.ingested_at"),
    ("updated_at", "p.updated_at"),
];

/// Build the full aggregate response for `request`. `local_username` + the instance `global_domain`
/// (from `settings`) resolve owner-default creators for the summary's creator histogram (feature 26).
#[tracing::instrument(skip(db, settings, request), fields(user_id = %user_id))]
pub async fn aggregate(
    db: &PgPool,
    settings: &Settings,
    user_id: Uuid,
    local_username: &str,
    request: AggregateRequest,
) -> Result<Value, AppError> {
    let sections = request
        .sections
        .unwrap_or_else(|| vec![AggregateSection::Summary]);
    let resolved = selection::resolve(db, user_id, &request.selection).await?;

    let mut out = Map::new();

    if sections.contains(&AggregateSection::Summary) {
        let s = PictureRepository::aggregate_summary(db, user_id, &resolved).await?;
        out.insert("count".into(), json!(s.count));
        out.insert("owned_count".into(), json!(s.owned_count));
        out.insert("received_count".into(), json!(s.received_count));
        out.insert("total_file_size".into(), json!(s.total_file_size));
        out.insert("trashed_count".into(), json!(s.trashed_count));
        out.insert("owner_deleting_count".into(), json!(s.owner_deleting_count));
        out.insert(
            "thumbnail_pending_count".into(),
            json!(s.thumbnail_pending_count),
        );
        out.insert("duplicate_count".into(), json!(s.duplicate_count));
        out.insert(
            "owners".into(),
            json!(
                s.owners
                    .iter()
                    .map(|o| json!({ "username": o.username, "instance": o.instance, "count": o.count }))
                    .collect::<Vec<_>>()
            ),
        );
        out.insert("exif_sync".into(), exif_sync_histogram(&s.exif_sync));

        // Resolved-creator histogram (feature 26) — a distinct FieldAggregate, so the panel reuses the
        // "Mixed / common value" rendering. Cheap (all on the pictures row, one GROUP BY like `owners`).
        let owner_default = format_identity(local_username, &settings.get(keys::GLOBAL_DOMAIN));
        let creator =
            PictureRepository::aggregate_creator(db, user_id, &resolved, &owner_default).await?;
        out.insert("creator".into(), distinct_field(&creator));
    }

    if sections.contains(&AggregateSection::Tags) {
        let aggs =
            TagRepository::aggregate_tags(db, user_id, &resolved, request.tag_provenance).await?;
        let tags: Vec<Value> = aggs
            .iter()
            .map(|a| {
                let mut obj = json!({
                    "path": TagPath::from_ltree(a.path.clone()).as_ltree().to_string(),
                    "count": a.count,
                    "manual_count": a.manual_count,
                });
                if request.tag_provenance {
                    obj["sources"] = json!(
                        a.sources
                            .iter()
                            .map(|(src, n)| json!({ "source": src, "count": n }))
                            .collect::<Vec<_>>()
                    );
                }
                // `count == total` (on-all vs on-some) is compared client-side against `summary.count`.
                obj
            })
            .collect();
        out.insert("tags".into(), json!(tags));
    }

    if sections.contains(&AggregateSection::Exif) {
        out.insert("exif".into(), exif_section(db, user_id, &resolved).await?);
    }

    Ok(Value::Object(out))
}

/// Serialise the `exif_sync` histogram as a complete `Record<ExifSyncStatus, number>` (zeros filled).
fn exif_sync_histogram(hist: &[(ExifSyncStatus, i64)]) -> Value {
    let lookup = |want: ExifSyncStatus| {
        hist.iter()
            .find(|(s, _)| *s == want)
            .map(|(_, n)| *n)
            .unwrap_or(0)
    };
    json!({
        "synced": lookup(ExifSyncStatus::Synced),
        "pending": lookup(ExifSyncStatus::Pending),
        "unsupported": lookup(ExifSyncStatus::Unsupported),
        "pending_job_creation": lookup(ExifSyncStatus::PendingJobCreation),
    })
}

/// A distinct-value [`FieldAggregate`](§4.3) from a [`DistinctAgg`]: first `DISTINCT_CAP` values
/// inline, the rest as `distinct_overflow`; `common` set when the field collapses to one value.
fn distinct_field(agg: &DistinctAgg) -> Value {
    let total_distinct = agg.values.len();
    let overflow = total_distinct.saturating_sub(DISTINCT_CAP) as i64;
    let common = if total_distinct == 1 && agg.null_count == 0 {
        Some(Value::String(agg.values[0].0.clone()))
    } else {
        None
    };
    let distinct: Vec<Value> = agg
        .values
        .iter()
        .take(DISTINCT_CAP)
        .map(|(v, c)| json!({ "value": v, "count": c }))
        .collect();
    json!({
        "type": "distinct",
        "common": common,
        "distinct": distinct,
        "distinct_overflow": overflow,
        "null_count": agg.null_count,
    })
}

/// Build the per-field type-aware EXIF aggregate map (§4.3).
async fn exif_section(
    db: &PgPool,
    user_id: Uuid,
    resolved: &crate::repository::picture::ResolvedSelection,
) -> Result<Value, AppError> {
    let mut map = Map::new();

    for (name, expr) in DISTINCT_FIELDS {
        let agg = PictureRepository::aggregate_distinct(db, user_id, resolved, expr).await?;
        map.insert((*name).into(), distinct_field(&agg));
    }

    for (name, agg) in
        PictureRepository::aggregate_numeric(db, user_id, resolved, NUMERIC_FIELDS).await?
    {
        map.insert(
            name,
            json!({
                "type": "numeric",
                "min": agg.min,
                "max": agg.max,
                "avg": agg.avg,
                "null_count": agg.null_count,
            }),
        );
    }

    for (name, agg) in
        PictureRepository::aggregate_dates(db, user_id, resolved, DATE_FIELDS).await?
    {
        map.insert(
            name,
            json!({
                "type": "date",
                "min": agg.min,
                "max": agg.max,
                "avg": agg.avg,
                "null_count": agg.null_count,
            }),
        );
    }

    let gps = PictureRepository::aggregate_gps(db, user_id, resolved).await?;
    let bbox = match (gps.lat_min, gps.lat_max, gps.lng_min, gps.lng_max) {
        (Some(lat_min), Some(lat_max), Some(lng_min), Some(lng_max)) => json!({
            "lat_min": lat_min, "lat_max": lat_max, "lng_min": lng_min, "lng_max": lng_max,
        }),
        _ => Value::Null,
    };
    let centroid = match (gps.centroid_lat, gps.centroid_lng) {
        (Some(lat), Some(lng)) => json!({ "lat": lat, "lng": lng }),
        _ => Value::Null,
    };
    map.insert(
        "gps".into(),
        json!({ "type": "gps", "bbox": bbox, "centroid": centroid, "null_count": gps.null_count }),
    );

    Ok(Value::Object(map))
}

/// The dry-run breakdown returned by every batch write when `dry_run = true` (§6.1). Fields are
/// `None` for operations they don't apply to (serialised away), so one shape serves all batches.
#[derive(Debug, Default, Serialize)]
pub struct DryRun {
    pub affected: i64,
    // EXIF batch:
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edited: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_override: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsupported: Option<i64>,
    // tags batch:
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed: Option<i64>,
}
