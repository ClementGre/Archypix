use crate::domain::hierarchy::TagPredicate;
use crate::domain::job::FullExif;
use crate::domain::picture::ExifSyncStatus;
use archypix_common::error::AppError;
use chrono::NaiveDateTime;
use serde::Deserialize;
use sqlx::Postgres;
use uuid::Uuid;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PictureSortField {
    CapturedAt,
    #[default]
    IngestedAt,
    UpdatedAt,
    FileSize,
    Filename,
    /// Proximity sort by `|captured_at − near_time|` (feature 29 §6). Requires `near_time`; rows
    /// without a capture date sort last; `SortOrder` is ignored (always nearest-first).
    TimeNear,
    /// Proximity sort by approximate (equirectangular) distance to `near_lat`/`near_lng` (feature 29
    /// §6). Requires both; ungeotagged rows sort last; `SortOrder` is ignored.
    GeoNear,
}

/// Per-field metadata-presence filter (feature 29 §4). AND-composed with every other list arm.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceFilter {
    /// No constraint.
    #[default]
    Any,
    /// The field is populated.
    Present,
    /// The field is NULL.
    Missing,
}

impl PresenceFilter {
    fn is_any(&self) -> bool {
        matches!(self, Self::Any)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    Asc,
    #[default]
    Desc,
}

/// Trash-membership state for a picture list. The trash is a **filter over the main view**, not a
/// separate page: `Exclude` is the normal gallery, `Only` is the trash, `Include` shows both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrashFilter {
    /// Live pictures only (`deleted_at IS NULL`) — the default gallery.
    #[default]
    Exclude,
    /// Live + `manual`-trashed pictures.
    Include,
    /// `manual`-trashed pictures only — the trash view.
    Only,
}

#[derive(Debug, Clone, Default)]
pub struct PictureListFilter {
    pub page: i64,
    pub page_size: i64,
    pub sort: PictureSortField,
    pub order: SortOrder,
    /// Generalised tag-set predicate: the flat gallery (`include`/`exclude`/`exact`/`match`/
    /// `untagged`) and hierarchy `browse` both lower to this. `None` ⇒ no tag constraint.
    pub predicate: Option<TagPredicate>,
    pub owned_only: bool,
    pub shared_with_me: bool,
    pub trash: TrashFilter,
    pub captured_after: Option<NaiveDateTime>,
    pub captured_before: Option<NaiveDateTime>,
    /// GPS-presence filter (feature 29 §4). AND-composed with the other arms.
    pub gps: PresenceFilter,
    /// Capture-date presence filter (over `captured_at`).
    pub capture_date: PresenceFilter,
    /// "Any issue" OR convenience (§4): `(gps IS NULL OR captured_at IS NULL)`. Mutually exclusive
    /// with a non-`Any` `gps`/`capture_date` (rejected at construction).
    pub missing_any: bool,
    /// Reference instant for `PictureSortField::TimeNear`.
    pub near_time: Option<NaiveDateTime>,
    /// Reference point for `PictureSortField::GeoNear`.
    pub near_lat: Option<f64>,
    pub near_lng: Option<f64>,
    /// Date-fix mode (feature 30 §4): float undated rows (`captured_at IS NULL`) to the top with a
    /// `filename, id` tiebreaker so the broken ones surface for fixing while the dated references stay
    /// scrollable below. A prefix on top of the current column sort; ignored for proximity sorts.
    pub undated_first: bool,
}

impl PictureListFilter {
    /// Reject the mutually-exclusive presence combination (§4) and a proximity sort missing its
    /// reference param (§6). Called at the wire-parse boundary so a bad request surfaces as a 400.
    pub fn validate(&self) -> Result<(), AppError> {
        if self.missing_any && (!self.gps.is_any() || !self.capture_date.is_any()) {
            return Err(AppError::BadRequest(
                "missing_any cannot be combined with a per-field gps/capture_date presence filter"
                    .to_string(),
            ));
        }
        match self.sort {
            PictureSortField::TimeNear if self.near_time.is_none() => {
                return Err(AppError::BadRequest(
                    "sort=time_near requires near_time".to_string(),
                ));
            }
            PictureSortField::GeoNear if self.near_lat.is_none() || self.near_lng.is_none() => {
                return Err(AppError::BadRequest(
                    "sort=geo_near requires near_lat and near_lng".to_string(),
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

/// A picture selection (feature 14 §2) resolved against the DB: the query lowered to a
/// [`PictureListFilter`] (`None` ⇒ pure explicit set, or a hierarchy directory with no direct files)
/// plus the explicit id deltas. The reusable membership term every batch endpoint resolves to:
/// `(filter ∪ include_ids) \ exclude_ids`, scoped to the caller. Built by `services::selection`.
#[derive(Debug, Clone)]
pub struct ResolvedSelection {
    pub filter: Option<PictureListFilter>,
    pub include_ids: Vec<Uuid>,
    pub exclude_ids: Vec<Uuid>,
}

impl ResolvedSelection {
    /// A pure explicit set over the given ids (the degenerate single-/multi-click case).
    pub fn explicit(include_ids: Vec<Uuid>) -> Self {
        Self {
            filter: None,
            include_ids,
            exclude_ids: vec![],
        }
    }

    /// True when the selection can match no picture regardless of the user's holdings (no query and
    /// no explicitly-included id). Callers short-circuit to an empty result.
    pub fn is_empty(&self) -> bool {
        self.filter.is_none() && self.include_ids.is_empty()
    }
}

/// Selection-summary aggregate (feature 14 §4.1) — all read straight off the `pictures` row.
#[derive(Debug, Default)]
pub struct SelectionSummary {
    pub count: i64,
    pub owned_count: i64,
    pub received_count: i64,
    pub total_file_size: i64,
    pub trashed_count: i64,
    pub owner_deleting_count: i64,
    pub thumbnail_pending_count: i64,
    pub duplicate_count: i64,
    /// Distinct remote owners of received pictures in the selection.
    pub owners: Vec<OwnerCount>,
    /// `exif_sync_status` histogram (label → count), including `pending_job_creation`.
    pub exif_sync: Vec<(ExifSyncStatus, i64)>,
}

#[derive(Debug)]
pub struct OwnerCount {
    pub username: String,
    pub instance: String,
    pub count: i64,
}

/// Min/max/avg of a numeric field over the selection (`null_count` = rows where the field is NULL).
#[derive(Debug, Default, Clone)]
pub struct NumericAgg {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub avg: Option<f64>,
    pub null_count: i64,
}

/// Min/max range + avg instant of a date field over the selection.
#[derive(Debug, Default, Clone)]
pub struct DateAgg {
    pub min: Option<NaiveDateTime>,
    pub max: Option<NaiveDateTime>,
    pub avg: Option<NaiveDateTime>,
    pub null_count: i64,
}

/// Exact bounding box + centroid of the GPS points in the selection.
#[derive(Debug, Default, Clone)]
pub struct GpsAgg {
    pub lat_min: Option<f64>,
    pub lat_max: Option<f64>,
    pub lng_min: Option<f64>,
    pub lng_max: Option<f64>,
    pub centroid_lat: Option<f64>,
    pub centroid_lng: Option<f64>,
    pub null_count: i64,
}

/// Distinct-value histogram of a string/enum field over the selection.
#[derive(Debug, Default, Clone)]
pub struct DistinctAgg {
    /// `(value, count)` pairs ordered by descending count (NULLs excluded — see `null_count`).
    pub values: Vec<(String, i64)>,
    pub null_count: i64,
}

/// The two partitions a set-based EXIF edit produces: rows whose file sync was queued, and rows
/// carrying a terminal verdict that stay DB-only.
#[derive(Debug, Default, Clone, Copy)]
pub struct BatchExifCounts {
    pub edited: i64,
    pub unsupported: i64,
}

pub struct PictureRepository;

mod aggregate;
mod batch;
mod crud;
mod exif;
mod query;
mod received;
mod selection;

/// Parse an `picture_exif_sync_status` text label into the enum (drives the summary histogram).
fn parse_exif_sync_status(label: &str) -> Option<ExifSyncStatus> {
    match label {
        "synced" => Some(ExifSyncStatus::Synced),
        "pending" => Some(ExifSyncStatus::Pending),
        "extracting" => Some(ExifSyncStatus::Extracting),
        "extract_failed" => Some(ExifSyncStatus::ExtractFailed),
        "unsupported_mime" => Some(ExifSyncStatus::UnsupportedMime),
        "unsupported_file" => Some(ExifSyncStatus::UnsupportedFile),
        "pending_job_creation" => Some(ExifSyncStatus::PendingJobCreation),
        "write_failed" => Some(ExifSyncStatus::WriteFailed),
        _ => None,
    }
}

/// Push the `new_ov` JSONB expression `(overrides - clear) || patch` with fresh binds.
fn push_new_ov(
    q: &mut sqlx::QueryBuilder<Postgres>,
    set_patch: &serde_json::Value,
    clear_keys: &[String],
) {
    q.push("((COALESCE(p.local_exif_overrides, '{}'::jsonb) - ")
        .push_bind(clear_keys.to_vec())
        .push("::text[]) || ")
        .push_bind(set_patch.clone())
        .push("::jsonb)");
}

/// Push `new_ov` with the redundant `set` keys dropped: a set key whose value already equals the
/// owner's `remote_exif_data` value is not stored as an override (it would needlessly shadow a future
/// owner edit — 09 §6.1). Pre-existing overrides on untouched fields are left intact.
fn push_pruned_new_ov(
    q: &mut sqlx::QueryBuilder<Postgres>,
    set_patch: &serde_json::Value,
    clear_keys: &[String],
) {
    q.push("(");
    push_new_ov(q, set_patch, clear_keys);
    q.push(" - ARRAY(SELECT e.k FROM jsonb_each(")
        .push_bind(set_patch.clone())
        .push("::jsonb) AS e(k, v) WHERE COALESCE(p.remote_exif_data -> e.k, 'null'::jsonb) IS NOT DISTINCT FROM e.v))");
}

/// Push the `merged` JSONB expression `remote || new_ov` with fresh binds.
fn push_merged(
    q: &mut sqlx::QueryBuilder<Postgres>,
    set_patch: &serde_json::Value,
    clear_keys: &[String],
) {
    q.push("(COALESCE(p.remote_exif_data, '{}'::jsonb) || ");
    push_new_ov(q, set_patch, clear_keys);
    q.push(")");
}

/// The seven camera/lens JSONB keys held in `exif_data`.
const CAMERA_KEYS: [&str; 7] = [
    "camera_brand",
    "camera_model",
    "focal_length_mm",
    "f_number",
    "iso_speed",
    "exposure_time_num",
    "exposure_time_den",
];

/// Push the promoted-column + `exif_data` assignments for an owned-picture EXIF `set`/`clear`,
/// trailing each with `, ` (the caller appends `exif_sync_status = …`). Only touched fields appear.
fn push_exif_column_assignments(
    q: &mut sqlx::QueryBuilder<Postgres>,
    set: &FullExif,
    clear: &[crate::domain::job::ExifField],
) {
    use crate::domain::job::ExifField;
    let cleared = |f: ExifField| clear.contains(&f);

    // Promoted columns.
    if set.captured_at.is_some() {
        q.push("captured_at = ")
            .push_bind(set.captured_at)
            .push(", ");
    } else if cleared(ExifField::CapturedAt) {
        q.push("captured_at = NULL, ");
    }
    if set.gps_lat.is_some() {
        q.push("gps_lat = ").push_bind(set.gps_lat).push(", ");
    } else if cleared(ExifField::GpsLat) {
        q.push("gps_lat = NULL, ");
    }
    if set.gps_lng.is_some() {
        q.push("gps_lng = ").push_bind(set.gps_lng).push(", ");
    } else if cleared(ExifField::GpsLng) {
        q.push("gps_lng = NULL, ");
    }
    if set.gps_alt.is_some() {
        q.push("gps_alt = ").push_bind(set.gps_alt).push(", ");
    } else if cleared(ExifField::GpsAlt) {
        q.push("gps_alt = NULL, ");
    }
    if set.orientation.is_some() {
        q.push("orientation = ")
            .push_bind(set.orientation)
            .push(", ");
    } else if cleared(ExifField::Orientation) {
        q.push("orientation = NULL, ");
    }

    // Camera/lens JSONB: drop cleared keys, merge the set patch.
    let clear_camera: Vec<String> = CAMERA_KEYS
        .iter()
        .filter(|k| camera_key_cleared(k, clear))
        .map(|k| k.to_string())
        .collect();
    let patch = serde_json::to_value(&set.camera).unwrap_or_else(|_| serde_json::json!({}));
    let patch_empty = patch.as_object().map(|o| o.is_empty()).unwrap_or(true);
    if !clear_camera.is_empty() || !patch_empty {
        q.push("exif_data = (exif_data - ")
            .push_bind(clear_camera)
            .push("::text[]) || ")
            .push_bind(patch)
            .push("::jsonb, ");
    }
}

/// Whether a camera JSONB key is in the `clear` list.
fn camera_key_cleared(key: &str, clear: &[crate::domain::job::ExifField]) -> bool {
    use crate::domain::job::ExifField::*;
    let f = match key {
        "camera_brand" => CameraBrand,
        "camera_model" => CameraModel,
        "focal_length_mm" => FocalLengthMm,
        "f_number" => FNumber,
        "iso_speed" => IsoSpeed,
        "exposure_time_num" => ExposureTimeNum,
        "exposure_time_den" => ExposureTimeDen,
        _ => return false,
    };
    clear.contains(&f)
}
