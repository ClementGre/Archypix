use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "service_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ServiceType {
    SharedTagMapping,
    Rule,
    Segmentation,
}

/// Used only in Hierarchy JSONB config, not a direct column type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SafeDeleteMode {
    SingleBranch,
    FullDelete,
}

/// A user-defined tagging service that assigns tags to pictures according to rules.
/// Services are ordered into a pipeline.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TaggingService {
    pub id: Uuid,
    pub owner_id: Uuid,
    /// User-facing label (may be empty; the UI falls back to a type label).
    pub name: String,
    pub service_type: ServiceType,
    /// Tags that must ALL be present for this service to fire (ltree[] as text[]).
    pub requires: Vec<String>,
    /// Tags where ANY present will suppress this service (ltree[] as text[]).
    pub excludes: Vec<String>,
    pub enabled: bool,
    /// Execution order within Rule/Segmentation services (SharedTagMapping always runs first).
    pub position: i32,
    /// Bumped on every configuration change. Pictures with `last_pipeline_run_at` older
    /// than this value are considered dirty and will be re-evaluated.
    pub last_invalidated_at: NaiveDateTime,
    /// Set when the pipeline fails to evaluate this service; cleared on next success.
    pub last_error_at: Option<NaiveDateTime>,
    pub last_error_msg: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Maps an IncomingShare to a local tag path.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SharedTagMappingRule {
    pub id: Uuid,
    pub service_id: Uuid,
    pub incoming_share_id: Uuid,
    pub assign_tag: String,
    pub is_broken: bool,
}

/// Assigns a tag when a predicate over EXIF/filename/GPS matches.
///
/// `predicate` is a structured JSONB predicate tree (feature 13); parse it into
/// [`crate::domain::pipeline::Predicate`] to validate or evaluate it.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RuleTaggingRule {
    pub id: Uuid,
    pub service_id: Uuid,
    pub predicate: serde_json::Value,
    pub assign_tag: String,
    /// User-controlled display order within the service (presentation only — rules are
    /// evaluated independently).
    pub position: i32,
}

/// Assigns a tag when a picture's capture date falls within a date range.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SegmentationRule {
    pub id: Uuid,
    pub service_id: Uuid,
    pub name: String,
    pub date_start: NaiveDateTime,
    pub date_end: NaiveDateTime,
    pub assign_tag: String,
    pub parent_segment_id: Option<Uuid>,
}

/// Maps a filtered view of the tag graph to a WebDAV filesystem tree.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Hierarchy {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub config: sqlx::types::Json<serde_json::Value>,
    pub enabled: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}
