use crate::domain::job::{CameraExif, FullExif};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Picture {
    pub id: Uuid,
    pub local_user_id: Uuid,
    /// Set only for pictures received via federation (not owned by this instance's user).
    pub remote_picture_id: Option<String>,
    pub owner_username: Option<String>,
    pub owner_instance_domain: Option<String>,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<i64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// Camera/lens EXIF only — the promoted fields (`captured_at`, `gps_*`, `orientation`) live in
    /// their own columns. For received rows this is the camera part of the materialised merge.
    pub exif_data: Json<CameraExif>,
    pub metadata: Json<serde_json::Value>,
    pub deleted_at: Option<NaiveDateTime>,
    /// Why this row was soft-deleted (set together with `deleted_at`). Only `Manual` is produced
    /// today; the other variants are reserved for the physical-copy/dedup work (spec 11).
    pub deleted_reason: Option<DeletedReason>,
    /// Received rows only: the owner's soft-delete timestamp, propagated on announcement. Distinct
    /// from `deleted_at` (the recipient's own local trash). Drives the grace-window badge.
    pub owner_deleted_at: Option<NaiveDateTime>,
    /// Received rows only: the owner's announced purge deadline (their `deleted_at + retention`).
    pub owner_purge_at: Option<NaiveDateTime>,
    /// Received rows only: the owner's authoritative EXIF snapshot (canonical full editable-EXIF
    /// JSON), refreshed on every announcement. `exif_data` for received rows is the merge of this
    /// with `local_exif_overrides`.
    pub remote_exif_data: Option<Json<FullExif>>,
    /// Received rows only: the recipient's sticky per-field EXIF overrides (sparse key set).
    pub local_exif_overrides: Option<Json<FullExif>>,
    pub captured_at: Option<NaiveDateTime>,
    pub ingested_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub blurhash: Option<String>,
    pub gps_lat: Option<f64>,
    pub gps_lng: Option<f64>,
    pub gps_alt: Option<i32>,
    pub orientation: Option<i16>,
    pub thumbnails_generated_at: Option<NaiveDateTime>,
    /// SHA-256 hex digest of the stored file. Used as WebDAV ETag.
    pub file_hash: Option<String>,
    /// Convergence of the S3 original's embedded EXIF versus this row (the source of truth).
    pub exif_sync_status: ExifSyncStatus,
}

/// Why a picture was soft-deleted (set with `deleted_at`). Feature 09 only produces `Manual`; the
/// other reasons are reserved for the physical-copy/dedup work (spec 11) so no later migration is
/// needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "picture_deleted_reason", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum DeletedReason {
    Manual,
    Boomerang,
    ContentDedupe,
}

/// Convergence state of a picture's embedded-file EXIF versus the DB row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "picture_exif_sync_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ExifSyncStatus {
    Synced,
    Pending,
    Unsupported,
    PendingJobCreation,
}

impl Picture {
    /// The picture's effective editable EXIF as a [`FullExif`] — the promoted columns plus the
    /// camera/lens fields from `exif_data`. For owned rows this is authoritative; for received rows
    /// it is the materialised merge. Used as an edit's revert baseline and convergence comparison.
    pub fn full_exif(&self) -> FullExif {
        FullExif {
            captured_at: self.captured_at,
            gps_lat: self.gps_lat,
            gps_lng: self.gps_lng,
            gps_alt: self.gps_alt,
            orientation: self.orientation,
            camera: self.exif_data.0.clone(),
        }
    }
}

impl Picture {
    pub fn is_owned(&self) -> bool {
        self.remote_picture_id.is_none()
    }
}

/// Transient upload state stored in Redis during the presigned-URL upload window.
#[derive(Debug, Serialize, Deserialize)]
pub struct UploadSession {
    pub user_id: Uuid,
    pub picture_id: Uuid,
    pub s3_key_staging: String,
    pub filename: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PictureVersion {
    pub id: Uuid,
    pub picture_id: Uuid,
    pub version_number: i32,
    pub file_size: Option<i64>,
    pub mime_type: Option<String>,
    pub created_at: NaiveDateTime,
}
