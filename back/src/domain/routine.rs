//! Routine trigger payloads — plain data, so `services` can schedule work without depending on
//! `routines`. See doc/03 §B (layer table) and `doc/features/17_unified_routine_framework.md`.

use crate::domain::picture::ExifSyncStatus;
use uuid::Uuid;

/// Payload **and** dedup key. Two distinct unannounces (different fields) both run; two identical
/// ones in flight collapse to a single rerun (idempotent).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct UnannounceInput {
    pub outgoing_share_id: Uuid,
    pub sender_username: String,
    pub recipient_username: String,
    pub recipient_instance: String,
    /// Announce ids (recipient's `remote_picture_id`) of the pictures to remove.
    pub picture_ids: Vec<String>,
    pub is_same_backend: bool,
}

/// Payload **and** dedup key. Identical renames in flight collapse to one rerun (idempotent).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TagRenameInput {
    pub user_id: Uuid,
    /// ltree form (dot-separated), already validated non-reserved by the endpoint.
    pub old_tag: String,
    pub new_tag: String,
}

/// Which worklist to drain, and the optional MIME narrowing (lower-cased) the `mime` scope uses
/// after an allowlist bump.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ExifRecheckInput {
    pub scope: RecheckScope,
    pub mime_types: Vec<String>,
}

/// Which worklist an admin EXIF recheck sweep drains (feature 33 §8).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum RecheckScope {
    /// Rows the MIME preflight rejected — the normal case after an allowlist bump.
    #[default]
    Mime,
    /// Rows no engine could open, after an engine upgrade. Rare and mostly futile.
    File,
    /// Rows whose extraction never returned, after a tool outage.
    Failed,
}

impl RecheckScope {
    pub fn status(self) -> ExifSyncStatus {
        match self {
            Self::Mime => ExifSyncStatus::UnsupportedMime,
            Self::File => ExifSyncStatus::UnsupportedFile,
            Self::Failed => ExifSyncStatus::ExtractFailed,
        }
    }
}
