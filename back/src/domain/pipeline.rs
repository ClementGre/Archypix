//! The picture projection the tagging pipeline evaluates against.
//!
//! Service evaluation (gating + the per-type evaluators) lives in [`crate::domain::tagging`]; the
//! rule predicate engine in [`crate::domain::predicate`]; calendar segmentation in
//! [`crate::domain::segmentation`].

use chrono::NaiveDateTime;
use uuid::Uuid;

/// Input fed to the pipeline evaluator for a single picture.
#[derive(Debug, Clone)]
pub struct PipelineInput {
    pub picture_id: Uuid,
    pub captured_at: Option<NaiveDateTime>,
    pub ingested_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub gps_lat: Option<f64>,
    pub gps_lng: Option<f64>,
    pub gps_alt: Option<i32>,
    pub filename: Option<String>,
    // ── Better-rules (feature 13) fields — all Option except the derived `is_owned`. ──
    pub camera_brand: Option<String>,
    pub camera_model: Option<String>,
    pub focal_length_mm: Option<f64>,
    pub f_number: Option<f64>,
    pub iso_speed: Option<i32>,
    /// `exposure_time_num / exposure_time_den` in seconds; `None` if either column is NULL.
    pub exposure_time: Option<f64>,
    pub orientation: Option<i16>,
    pub mime_type: Option<String>,
    pub file_size: Option<i64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// Derived: `remote_picture_id IS NULL` (the picture is owned by this user). Queryable as the
    /// `is_owned` rule field.
    pub is_owned: bool,
    /// Resolved **owner** identity `@username:domain` — the local holder for an owned picture, the
    /// stored origin owner for a received one (`"Unknown"` if unresolvable). Never empty. Queryable as
    /// the (non-nullable) `owner` rule field — a string comparison alongside the `is_owned` boolean.
    pub owner: String,
    /// Resolved **displayed** creator credit (feature 26): `coalesce(creator_override, creator,
    /// owner_default)`. Effectively always `Some` (the owner default backstops it); `None` only when
    /// the owner is unresolvable. Queryable as the (non-nullable) `creator` rule field.
    pub creator: Option<String>,
}
