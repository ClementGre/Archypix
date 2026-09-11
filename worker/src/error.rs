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

    // ── Permanent — do not retry ──────────────────────────────────────────────
    /// Image processing failure (corrupt file, codec error, etc.).
    #[error("Image processing error: {0}")]
    Imaging(String),
    /// EXIF library error.
    #[error("EXIF error: {0}")]
    Exif(String),
    /// The file cannot carry the requested metadata (container/codec limitation, corrupt EXIF
    /// block). Reported to the backend as `unsupported` so the picture leaves the sync queue
    /// instead of showing a retryable write failure (feature 31 §6).
    #[error("Unsupported metadata format: {0}")]
    UnsupportedFormat(String),
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
            // Everything else is a permanent failure.
            Self::Imaging(_)
            | Self::Exif(_)
            | Self::UnsupportedFormat(_)
            | Self::MissingPresignedUrl { .. }
            | Self::Jwt(_)
            | Self::Json(_) => false,
        }
    }

    /// Returns `true` when the failure means the file can never carry the metadata.
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::UnsupportedFormat(_))
    }
}

pub type Result<T> = std::result::Result<T, WorkerError>;
