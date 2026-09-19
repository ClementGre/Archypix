use crate::domain::picture::{Picture, PictureVersion};
use crate::domain::tag::TagPath;
use crate::infra::settings::keys;
use crate::repository::picture::{
    PictureSortField, PresenceFilter,
    SortOrder, TrashFilter,
};
use archypix_common::error::AppError;
use archypix_common::settings::Settings;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

/// Selectable picture variant for presigning. Used both in list thumbnails and the per-picture URL endpoint.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PictureVariant {
    Original,
    Small,
    Medium,
    Large,
}

impl PictureVariant {
    pub fn bucket(&self, settings: &Settings) -> String {
        match self {
            PictureVariant::Original => settings.get(keys::S3_BUCKET_PICTURES),
            PictureVariant::Small => settings.get(keys::S3_BUCKET_SMALL),
            PictureVariant::Medium => settings.get(keys::S3_BUCKET_MEDIUM),
            PictureVariant::Large => settings.get(keys::S3_BUCKET_LARGE),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    /// A generated-thumbnail variant (small/medium/large). These only exist once the worker has
    /// generated thumbnails (`pictures.thumbnails_generated_at`), which it skips for
    /// non-thumbnailable formats (PDFs, some videos, …). The `original` always exists.
    pub fn is_thumbnail(&self) -> bool {
        !matches!(self, PictureVariant::Original)
    }
}

impl FromStr for PictureVariant {
    type Err = AppError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "original" => Ok(Self::Original),
            "small" => Ok(Self::Small),
            "medium" => Ok(Self::Medium),
            "large" => Ok(Self::Large),
            other => Err(AppError::BadRequest(format!("Unknown variant: {other}"))),
        }
    }
}

// Keep the old name as an alias so list_pictures still compiles.
pub type ThumbnailSize = PictureVariant;

#[derive(Debug, Clone, Deserialize)]
pub struct UploadMetadata {
    pub mime_type: Option<String>,
    pub file_size: Option<i64>,
    pub file_hash: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub exif_data: Option<serde_json::Value>,
    pub captured_at: Option<NaiveDateTime>,
    /// Optional source file creation time (feature 30 §10) for clients that can provide one. The web
    /// upload leaves it unset (browsers expose no creation date); persisted, never applied to `captured_at`.
    pub original_file_created_at: Option<NaiveDateTime>,
    pub initial_tags: Option<Vec<String>>,
    /// Front-provided import label (`Uploaded.YYYY_MM_DD_HH_MM`, fixed per batch). When set, the
    /// picture is tagged with it (feature 15). A single ltree label, validated server-side.
    pub upload_label: Option<String>,
    #[serde(default)]
    pub defer_pipeline: bool,
}

fn default_page() -> u32 {
    1
}
fn default_page_size() -> u32 {
    50
}

/// Great-circle distance in metres (haversine). Used to surface the per-row distance under a
/// `geo_near` sort — the same metric the DB orders by (feature 29 §6), so badge and order agree.
fn haversine_m(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    const EARTH_RADIUS_M: f64 = 6_371_000.0;
    let (a_lat, b_lat) = (lat1.to_radians(), lat2.to_radians());
    let d_lat = (lat2 - lat1).to_radians();
    let d_lng = (lng2 - lng1).to_radians();
    let a = (d_lat / 2.0).sin().powi(2) + a_lat.cos() * b_lat.cos() * (d_lng / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * a.sqrt().asin()
}

#[derive(Debug, Clone, Deserialize)]
pub struct PictureListParams {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
    #[serde(default)]
    pub sort: PictureSortField,
    #[serde(default)]
    pub order: SortOrder,
    /// Flat tag-set filter (§6.3). Comma-separated ltree paths; combined per `match`.
    pub include_tags: Option<String>,
    pub exclude_tags: Option<String>,
    /// One ltree path matched **exactly** (`tag_path = p`, no descendants) — the timeline's
    /// per-section scope (feature 35 §7). Combined with `include`/`exclude` per `match`.
    pub exact: Option<String>,
    /// `all` (AND) | `any` (OR) over `include_tags`. Default `all`.
    #[serde(rename = "match")]
    pub match_mode: Option<String>,
    /// `true` ⇒ pictures with no stored tag of any source. AND-ed with the tag arms, so the gallery
    /// can layer its cross-cutting include/exclude sets onto the root view too (feature 35 §7).
    #[serde(default)]
    pub untagged: bool,
    #[serde(default)]
    pub owned_only: bool,
    #[serde(default)]
    pub shared_with_me: bool,
    /// Trash-membership state: `exclude` (default) | `include` | `only` (trash view).
    #[serde(default)]
    pub trash: TrashFilter,
    pub captured_after: Option<DateTime<Utc>>,
    pub captured_before: Option<DateTime<Utc>>,
    /// Presence filters (feature 29 §4): `?gps=present|missing`, `?capture_date=present|missing`,
    /// `?missing_any=true` (the OR convenience, mutually exclusive with a per-field presence).
    #[serde(default)]
    pub gps: PresenceFilter,
    #[serde(default)]
    pub capture_date: PresenceFilter,
    #[serde(default)]
    pub missing_any: bool,
    /// Proximity-sort reference points (feature 29 §6): required by `sort=time_near` / `geo_near`.
    /// `near_time` is a **naive** instant (no offset), compared against the naive `captured_at`
    /// column — matches the `captured_at` string the client reads back from a picture detail.
    pub near_time: Option<NaiveDateTime>,
    pub near_lat: Option<f64>,
    pub near_lng: Option<f64>,
    /// Date-fix mode (feature 30 §4): float undated pictures to the top of the current sort so the
    /// user can fix them while the dated references stay scrollable below (`?undated_first=true`).
    #[serde(default)]
    pub undated_first: bool,
    pub thumbnail: Option<ThumbnailSize>,
}

#[derive(Debug, Serialize)]
pub struct PictureListItem {
    pub id: Uuid,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub captured_at: Option<NaiveDateTime>,
    pub ingested_at: NaiveDateTime,
    /// Both are `SortField`s, so the timeline buckets them client-side (feature 35 §6).
    pub updated_at: NaiveDateTime,
    pub file_size: Option<i64>,
    /// Source file modification time captured at ingest (feature 30 §10). Suggestion-only date source
    /// for the date-fix chip; `None` when unknown (most received / WebDAV rows).
    pub original_file_created_at: Option<NaiveDateTime>,
    /// Derived GPS presence (feature 29 §3): `gps_lat IS NOT NULL AND gps_lng IS NOT NULL`, for owned
    /// **and** received rows (received GPS lives in the promoted columns). Drives client-side
    /// highlight-in-context and the fix-tools grid-local anchor scan without a round-trip.
    pub has_gps: bool,
    /// Great-circle distance in metres from the `near_lat`/`near_lng` reference, populated **only**
    /// under a `geo_near` sort (feature 29 §6) so the client can show a "N km away" badge. `None`
    /// for other sorts and for ungeotagged rows. The list item never exposes raw coordinates, so
    /// this is the only way the client gets a per-picture distance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance_m: Option<f64>,
    /// BlurHash string for progressive loading. `None` until the thumbnail worker runs.
    pub blurhash: Option<String>,
    /// EXIF orientation value (1–8). Thumbnails are stored in raw pixel orientation, so the
    /// client rotates them to display correctly.
    pub orientation: Option<i16>,
    pub thumbnail_url: Option<String>,
    /// `true` when this row is a picture owned by the local user; `false` for a received
    /// (shared) picture. Lets the client label/filter shared pictures.
    pub owned: bool,
    /// Original owner identity for received pictures (`@owner_username:owner_instance`); `None`
    /// for owned pictures.
    pub owner_username: Option<String>,
    pub owner_instance: Option<String>,
    /// Resolved creator credit for display (feature 26): `coalesce(creator_override, creator,
    /// owner_identity)`. Parsed by its leading sigil client-side (`@user:domain` / `#name` / plain).
    pub creator: String,
    /// Convergence of the file's embedded EXIF vs the DB row.
    pub exif_sync_status: crate::domain::picture::ExifSyncStatus,
    /// The recipient's own local soft-delete timestamp (trash view); `None` when not trashed.
    pub deleted_at: Option<NaiveDateTime>,
    /// Owner-deletion lifecycle for received pictures (09 §5.3): the owner's soft-delete timestamp
    /// and announced purge deadline. Drive the red "owner will delete this on X" badge.
    pub owner_deleted_at: Option<NaiveDateTime>,
    pub owner_purge_at: Option<NaiveDateTime>,
    /// `false` when this (cross-instance) picture's owner backend was unreachable while presigning
    /// its thumbnail (feature 28 §3.2), so the client can render a distinct "owner offline" tile
    /// rather than a generic placeholder. Always `true` for owned / same-backend / reachable owners,
    /// and for a cross-instance picture with no active token yet (a "no thumbnail" state, not an
    /// outage).
    pub owner_reachable: bool,
}

#[derive(Debug, Serialize)]
pub struct PictureListResult {
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
    pub items: Vec<PictureListItem>,
}

#[derive(Debug, Serialize)]
pub struct PictureDetails {
    pub picture: Picture,
    pub versions: Vec<PictureVersion>,
}

/// One requested upload slot in a batch presign: a filename and, optionally, the client-computed
/// SHA-256 (lowercase hex) of the bytes about to be uploaded. The hash drives upload-time
/// deduplication against the user's existing owned pictures.
#[derive(Debug, Clone, Deserialize)]
pub struct BatchUploadFile {
    pub filename: String,
    pub file_hash: Option<String>,
    /// Client-declared byte size, enabling the presign-time quota reservation (feature 22 §5.3).
    /// When absent, only the coarse `at-or-over-quota` gate applies and the `complete_upload` hard
    /// check is the backstop.
    pub size: Option<i64>,
}

/// The outcome of one batch presign slot: either a fresh upload slot (PUT the bytes to
/// `presigned_url`, then `complete`), or a deduplication hit against an existing owned picture
/// (no upload needed — `picture_id` is the existing picture).
pub enum BatchUploadOutcome {
    New {
        picture_id: Uuid,
        presigned_url: String,
    },
    Duplicate {
        picture_id: Uuid,
        was_deleted: bool,
    },
}

/// Presign upload slots for a batch of files in one call. Returns one outcome per file in input
/// order.
///
/// Validate a front-provided import label and derive the three marker tag paths (wire form):
/// `(base, base.AlreadyExisting, base.AlreadyExisting.Deleted)`.
fn upload_marker_tags(label: &str) -> Result<(String, String, String), AppError> {
    let base = TagPath::parse(label, false)
        .map_err(AppError::BadRequest)?
        .as_ltree()
        .to_string();
    Ok((
        base.clone(),
        format!("{base}.AlreadyExisting"),
        format!("{base}.AlreadyExisting.Deleted"),
    ))
}

mod copy;
mod exif;
mod lifecycle;
mod listing;
mod presign;
mod upload;

pub use copy::*;
pub use exif::*;
pub use lifecycle::*;
pub use listing::*;
pub use presign::*;
pub use upload::*;
