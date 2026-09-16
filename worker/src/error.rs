use archypix_common::transfer::ExifExtraction;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WorkerError {
    // ── Transient — worth retrying ────────────────────────────────────────────
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Backend error: status={status}, body={body}")]
    BackendError { status: u16, body: String },
    /// An external tool the job depends on is missing or could not be spawned. A worker-environment
    /// fault, not a file fault — retried so the picture stays `pending` (feature 31 §6).
    #[error("External tool unavailable: {0}")]
    ToolUnavailable(String),

    // ── Permanent — do not retry ──────────────────────────────────────────────
    /// Image processing failure (corrupt file, codec error, etc.).
    #[error("Image processing error: {0}")]
    Imaging(String),
    /// EXIF library error.
    #[error("EXIF error: {0}")]
    Exif(String),
    /// The file cannot carry the requested metadata (container/codec limitation, corrupt EXIF
    /// block). Reported as `ExifExtraction::Failed` so the picture leaves the sync queue instead of
    /// showing a retryable write failure (feature 31 §6).
    #[error("Unsupported metadata format: {0}")]
    UnsupportedFormat(String),
    /// The MIME takes no EXIF writes at all, so there was never anything to attempt — a format
    /// verdict reached without opening the file. Distinct from [`Self::UnsupportedFormat`], which
    /// is a verdict about *these bytes* (feature 33 §4.3/§4.4).
    #[error("Format carries no writable EXIF: {0}")]
    UnsupportedMime(String),
    /// A required presigned URL was absent from the job response.
    #[error("No presigned URL for '{key}'")]
    MissingPresignedUrl { key: String },
    /// JWT signing or serialisation error.
    #[error("JWT error: {0}")]
    Jwt(String),
    /// JSON (de)serialisation error — usually a config/API schema mismatch.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl WorkerError {
    /// Returns `true` when the operation is worth retrying.
    ///
    /// - Transient (network, I/O, backend 5xx / 429) → retriable
    /// - Permanent (bad format, corrupt file, missing config, auth) → not retriable
    pub fn is_retriable(&self) -> bool {
        match self {
            Self::Http(_) => true,
            Self::Io(_) => true,
            Self::BackendError { status, .. } => *status >= 500 || *status == 429,
            Self::ToolUnavailable(_) => true,
            // Everything else is a permanent failure.
            Self::Imaging(_)
            | Self::Exif(_)
            | Self::UnsupportedFormat(_)
            | Self::UnsupportedMime(_)
            | Self::MissingPresignedUrl { .. }
            | Self::Jwt(_)
            | Self::Json(_) => false,
        }
    }

    /// The EXIF verdict this failure carries, if it is one. `None` means the job died without
    /// reaching a verdict, which is an absence, not a judgement (feature 33 §4.2).
    pub fn exif_verdict(&self) -> Option<ExifExtraction> {
        match self {
            Self::UnsupportedMime(_) => Some(ExifExtraction::UnsupportedMime),
            Self::UnsupportedFormat(_) => Some(ExifExtraction::Failed),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, WorkerError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_classification_matches_feature_31_section_6() {
        // An unopenable file is a verdict about these bytes.
        let unopenable = WorkerError::UnsupportedFormat("cannot open".into());
        assert!(!unopenable.is_retriable());
        assert_eq!(unopenable.exif_verdict(), Some(ExifExtraction::Failed));

        // A format that takes no EXIF writes is a verdict about the MIME — the distinction the
        // backend needs to label the row `unsupported_mime` rather than `unsupported_file`.
        let wrong_format = WorkerError::UnsupportedMime("video/mp4".into());
        assert!(!wrong_format.is_retriable());
        assert_eq!(
            wrong_format.exif_verdict(),
            Some(ExifExtraction::UnsupportedMime)
        );

        // A write that failed on an openable file: permanent, but the user may retry it.
        let write_failed = WorkerError::Exif("failed to save EXIF overrides".into());
        assert!(!write_failed.is_retriable());
        assert_eq!(write_failed.exif_verdict(), None);

        // A missing external tool is a worker-environment fault, not a file verdict.
        let tool = WorkerError::ToolUnavailable("exiftool missing".into());
        assert!(tool.is_retriable());
        assert_eq!(tool.exif_verdict(), None);
    }
}
