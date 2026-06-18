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
            | Self::MissingPresignedUrl { .. }
            | Self::Jwt(_)
            | Self::Json(_) => false,
        }
    }
}

pub type Result<T> = std::result::Result<T, WorkerError>;
