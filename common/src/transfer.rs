/// HTTP transfer types shared between `back/` (serializes) and `worker/` (deserializes).
///
/// Both sides use this module directly so the shapes never drift.
use crate::job::{ExtractedExif, JobConfig, JobType};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Claim query ──────────────────────────────────────────────────────────────

/// Query parameters for `GET /api/worker/jobs/next`.
///
/// Shared so both sides stay in sync:
/// - **backend** deserializes it from the incoming URL query string.
/// - **worker** serializes it with `reqwest`'s `.query(&claim_query)` to build the URL.
///
/// The `types` field is a `Vec<JobType>` on both ends; the wire representation is a
/// single comma-separated value (e.g. `?types=gen_thumbnail,edit_picture`).
#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimQuery {
    /// Job-type filter. Empty = accept all types; serialized as comma-separated,
    /// absent when empty so no `?types=` appears in the URL.
    #[serde(
        default,
        with = "crate::serde_utils::csv",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub types: Vec<JobType>,
}

// ── Claim response ────────────────────────────────────────────────────────────

/// Response body for `GET /api/worker/jobs/next`.
///
/// The backend returns `null` (JSON) when no job is available; each side maps
/// that to `Option<ClaimJobResponse>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimJobResponse {
    pub job_id: Uuid,
    pub job_type: JobType,
    pub picture_id: Option<Uuid>,
    /// MIME type of the picture (`pictures.mime_type`). Used by the worker to
    /// gate EXIF extraction and thumbnail generation on format support before
    /// downloading the file.
    pub mime_type: Option<String>,
    /// Fully typed job config (same discriminant as `job_type`).
    pub config: JobConfig,
    /// Presigned GET URL for the original picture file. Present for all job types
    /// that need to read the file (thumbnail, edit, ML).
    pub presigned_read: Option<String>,
    /// Presigned PUT URLs for output artifacts the worker must upload.
    pub presigned_writes: PresignedWrites,
    /// One-time token issued at claim time. The worker must echo it back in every
    /// `complete` and `fail` call so the backend can reject stale workers that
    /// were reset by the watchdog and then woke up late.
    pub claim_token: Uuid,
}

/// Typed presigned PUT URL map.
///
/// Fields are optional: only those relevant to the job type will be populated.
///
/// | Job type                 | Populated fields                                       |
/// |--------------------------|--------------------------------------------------------|
/// | `gen_thumbnail`          | `small`, `medium`, `large`                             |
/// | `edit_picture` (exif)    | `output` only (no thumbnails — pixel content unchanged)|
/// | `edit_picture` (visual)  | `output`, `small`, `medium`, `large`                   |
/// | ML types                 | _(none)_                                               |
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PresignedWrites {
    /// WebP thumbnail — height 100 px.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub small: Option<String>,
    /// WebP thumbnail — height 500 px.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub medium: Option<String>,
    /// WebP thumbnail — height 1000 px.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub large: Option<String>,
    /// Edited full-resolution picture (replaces the original in the pictures bucket).
    /// Only present for `edit_picture` jobs that include `visual` transforms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

impl PresignedWrites {
    /// Build a thumbnail-only write set.
    pub fn thumbnails(small: String, medium: String, large: String) -> Self {
        Self {
            small: Some(small),
            medium: Some(medium),
            large: Some(large),
            output: None,
        }
    }

    /// Build a write set for exif-only edits: output file URL only, no thumbnails.
    pub fn exif_only(output: String) -> Self {
        Self {
            output: Some(output),
            small: None,
            medium: None,
            large: None,
        }
    }

    /// Build a write set that includes both output and thumbnail keys.
    pub fn edit_with_visual(output: String, small: String, medium: String, large: String) -> Self {
        Self {
            small: Some(small),
            medium: Some(medium),
            large: Some(large),
            output: Some(output),
        }
    }

    /// Returns `true` if all three thumbnail slots are populated.
    pub fn has_thumbnails(&self) -> bool {
        self.small.is_some() && self.medium.is_some() && self.large.is_some()
    }

    /// Iterate over the (variant_name, url) pairs for each populated thumbnail.
    pub fn thumbnail_pairs(&self) -> impl Iterator<Item = (&str, &str)> {
        [
            ("small", self.small.as_deref()),
            ("medium", self.medium.as_deref()),
            ("large", self.large.as_deref()),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.map(|url| (k, url)))
    }
}

// ── Complete / fail ───────────────────────────────────────────────────────────

/// Request body for `POST /api/worker/jobs/{id}/complete`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompleteJobRequest {
    /// Must match the `claim_token` issued when this job was claimed.
    /// The backend rejects completions from stale/wrong workers.
    pub claim_token: Uuid,
    /// EXIF data extracted from the image.
    /// Required for `gen_thumbnail` with `is_initial = true`; optional otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exif: Option<ExtractedExif>,
    /// BlurHash string computed from the original or processed image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blurhash: Option<String>,
    /// Set to `true` when the worker generated and uploaded thumbnail variants.
    #[serde(default)]
    pub thumbnails_generated: bool,
    /// Size in bytes of the file as it now exists in S3 (after any EXIF writes or
    /// visual transforms). Used to keep `pictures.file_size` accurate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_size: Option<i64>,
    /// SHA-256 hex digest of the file as it now exists in S3. Used as the WebDAV
    /// ETag and for content-addressed deduplication.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_hash: Option<String>,
    /// Authoritative pixel dimensions, read from the **decoded image** (not EXIF).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<i32>,
}

/// Request body for `POST /api/worker/jobs/{id}/fail`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailJobRequest {
    /// Must match the `claim_token` issued when this job was claimed.
    pub claim_token: Uuid,
    /// Human-readable error description for debugging and the job's `error_message` column.
    pub error: String,
    /// When `true`, skip the retry counter and mark the job as permanently `failed`.
    ///
    /// Set this for errors that will never resolve by retrying: unsupported file
    /// format, corrupt image, invalid config, etc. Leave `false` (default) for
    /// transient errors like network failures or backend 5xx responses.
    #[serde(default)]
    pub permanent: bool,
}
