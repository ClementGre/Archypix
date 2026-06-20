// Re-export the shared types so code inside back/ can use `crate::domain::job::…`
// without long common:: paths.
#[allow(unused_imports)]
pub use archypix_common::job::{
    // Core types used throughout back/.
    CameraExif,
    CropTransform,
    EditPictureConfig,
    ExifEdit,
    ExifField,
    ExtractedExif,
    FullExif,
    GenThumbnailConfig,
    // Auxiliary types re-exported here so callers can import from one place.
    JobConfig,
    JobStatus,
    JobType,
    ResizeTransform,
    VisualTransformations,
};
use chrono::NaiveDateTime;
use sqlx::types::Json;
use uuid::Uuid;

/// The raw database row for a job. Uses `sqlx::FromRow` for direct query mapping.
///
/// The `config` field stores a `JobConfig` serialised as JSONB (with the `"type"` tag).
/// Call `.typed_config()` to deserialise it into the strongly-typed `JobConfig` enum.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct Job {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub job_type: JobType,
    pub status: JobStatus,
    pub config: Json<serde_json::Value>,
    pub result: Option<Json<serde_json::Value>>,
    pub error_message: Option<String>,
    pub retry_count: i32,
    pub max_retries: i32,
    pub idempotency_key: Option<String>,
    pub picture_id: Option<Uuid>,
    pub claimed_by: Option<String>,
    /// One-time token issued at claim time; echoed back by workers in complete/fail.
    pub claim_token: Option<Uuid>,
    pub created_at: NaiveDateTime,
    pub started_at: Option<NaiveDateTime>,
    pub completed_at: Option<NaiveDateTime>,
}

impl Job {
    /// Deserialise the JSONB `config` column into the typed `JobConfig` enum.
    pub fn typed_config(&self) -> Result<JobConfig, serde_json::Error> {
        serde_json::from_value(self.config.0.clone())
    }
}
