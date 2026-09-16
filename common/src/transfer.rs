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
    /// W3C trace context captured at enqueue time; the worker links its job span to it.
    /// `None` when tracing is disabled or no context was active at enqueue time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_context: Option<std::collections::HashMap<String, String>>,
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

/// The read direction's verdict on a job's file (feature 33 §5). Retriable failures have no
/// variant: they fail the job instead of completing it.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExifExtraction {
    /// Both a successful ingest extraction and the edit path's post-write read-back.
    Extracted(ExtractedExif),
    /// No extraction was asked of this job (a non-initial `gen_thumbnail`) — status untouched.
    #[default]
    NotAttempted,
    /// The MIME can carry no EXIF this worker reads or writes — terminal `unsupported_mime`.
    /// A format verdict, reached without opening the file.
    UnsupportedMime,
    /// Every engine ran and none could open the file — terminal `unsupported_file`. A file verdict:
    /// dispatch **and** fallback on the read side, the write engine on the edit side.
    Failed,
}

impl ExifExtraction {
    /// The extracted metadata, when the read succeeded.
    pub fn extracted(&self) -> Option<&ExtractedExif> {
        match self {
            Self::Extracted(e) => Some(e),
            _ => None,
        }
    }
}

/// How a job ended. Replaces the `/complete` + `/fail` split and its `permanent: bool`: the
/// disposition is three-way, and a job that dies still reports what it produced (04 §"Job response").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum JobOutcome {
    /// Every step succeeded.
    Done,
    /// Transient failure; the backend decrements the retry budget and re-queues while it lasts.
    Retry { error: String },
    /// Will never succeed against these inputs — skip the retry budget.
    Failed { error: String },
}

/// What a job produced, per job type — an ML job has no blurhash to report, and the backend cannot
/// mistake an extraction's EXIF for an edit's read-back.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(tag = "job", rename_all = "snake_case")]
pub enum JobProduct {
    /// Ingest or re-extraction: `exif` is the file's own metadata, authoritative for the row.
    GenThumbnail(PictureWork),
    /// Edit reconcile: `exif` is the post-write read-back, or the verdict that stopped the write.
    EditPicture(PictureWork),
    /// ML jobs touch no picture columns yet.
    #[default]
    Ml,
}

impl JobProduct {
    /// The picture-processing fields, for the two job types that have them.
    pub fn picture_work(&self) -> Option<&PictureWork> {
        match self {
            Self::GenThumbnail(w) | Self::EditPicture(w) => Some(w),
            Self::Ml => None,
        }
    }

    /// Mutable access, for a handler filling in results as it goes.
    pub fn picture_work_mut(&mut self) -> Option<&mut PictureWork> {
        match self {
            Self::GenThumbnail(w) | Self::EditPicture(w) => Some(w),
            Self::Ml => None,
        }
    }
}

/// Everything a picture job produces. Every field is what the job got to before it ended, so a
/// failure reports its partial work rather than discarding it.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct PictureWork {
    /// What this job made of the file's EXIF (feature 33 §5). Absent ⇒ `NotAttempted`.
    #[serde(default)]
    pub exif: ExifExtraction,
    /// BlurHash string computed from the original or processed image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blurhash: Option<String>,
    /// Set once the worker generated **and uploaded** the thumbnail variants.
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
    /// SHA-256 over the image's **metadata-stripped** bytes (feature 11 §4) — stable across EXIF
    /// edits, changes on a visual re-encode. `None` for a format the worker cannot strip (the
    /// backend then groups by `file_hash`). Drives content deduplication.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Authoritative pixel dimensions, read from the **decoded image** (not EXIF).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<i32>,
}

/// Request body for `POST /api/worker/jobs/{id}/respond`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobResponse {
    /// Must match the `claim_token` issued at claim time — the guard against a stale worker
    /// (watchdog-reset, then re-claimed) overwriting a live job's results.
    pub claim_token: Uuid,
    #[serde(flatten)]
    pub outcome: JobOutcome,
    #[serde(flatten)]
    pub product: JobProduct,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both enums flatten into one object, so a response is a flat map with an `outcome` tag and a
    /// `job` tag. Worth pinning: `flatten` + internally-tagged enums is the fragile combination here.
    #[test]
    fn job_response_round_trips_flattened() {
        let body = JobResponse {
            claim_token: Uuid::nil(),
            outcome: JobOutcome::Failed {
                error: "codec".into(),
            },
            product: JobProduct::GenThumbnail(PictureWork {
                exif: ExifExtraction::UnsupportedMime,
                thumbnails_generated: true,
                file_size: Some(42),
                ..Default::default()
            }),
        };
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["outcome"], "failed");
        assert_eq!(v["error"], "codec");
        assert_eq!(v["job"], "gen_thumbnail");
        assert_eq!(v["exif"], "unsupported_mime");
        assert_eq!(v["file_size"], 42);

        let back: JobResponse = serde_json::from_value(v).unwrap();
        assert_eq!(back.outcome, body.outcome);
        assert_eq!(back.product, body.product);
    }

    /// An ML job carries no picture fields at all — the reason the product is per-job-type.
    #[test]
    fn ml_product_has_no_picture_fields() {
        let v = serde_json::to_value(JobResponse {
            claim_token: Uuid::nil(),
            outcome: JobOutcome::Done,
            product: JobProduct::Ml,
        })
        .unwrap();
        assert_eq!(v["job"], "ml");
        assert!(v.get("blurhash").is_none());
        assert!(v.get("exif").is_none());
    }
}
