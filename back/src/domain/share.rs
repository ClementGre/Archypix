use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
// snake_case keeps the single-word variants ("pending", "active", …) identical to the old
// lowercase form while mapping `PendingFirstAnnouncement` → "pending_first_announcement".
#[sqlx(type_name = "share_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ShareStatus {
    /// Announced to the recipient but not yet accepted.
    Pending,
    /// OutgoingShare only: the recipient accepted, but the sender has not yet announced.
    PendingFirstAnnouncement,
    /// Accepted by the recipient; pictures are visible.
    Active,
    /// OutgoingShare only: an announce/unannounce delivery failed. The pipeline retries it with a
    /// full coverage reconcile (subject to `next_retry_at` backoff) and flips it back to `active`
    /// once fully delivered.
    Errored,
    /// Revoked by the sender; pictures are no longer accessible.
    Revoked,
    /// Rejected or deleted by the recipient.
    Tombstoned,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OutgoingShare {
    pub id: Uuid,
    pub owner_id: Uuid,
    /// ltree stored as text.
    pub tag_path: String,
    pub name: String,
    pub message: Option<String>,
    pub recipient_username: String,
    pub recipient_instance: String,
    pub allow_share_back: bool,
    pub future: bool,
    pub status: ShareStatus,
    pub created_at: NaiveDateTime,
    pub revoked_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct IncomingShare {
    pub id: Uuid,
    pub recipient_id: Uuid,
    pub sender_username: String,
    pub sender_instance: String,
    pub name: String,
    pub message: Option<String>,
    pub outgoing_share_id: Uuid,
    pub local_mapping_service_id: Option<Uuid>,
    pub status: ShareStatus,
    /// Whether the sender allows sharing these pictures back with auto-accept.
    pub allow_share_back: bool,
    pub created_at: NaiveDateTime,
    pub revoked_at: Option<NaiveDateTime>,
}
