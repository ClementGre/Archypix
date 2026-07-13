use crate::domain::job::FullExif;
use crate::domain::picture::Picture;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

//———————————————— Auth ————————————————

#[derive(Debug, Serialize, Deserialize)]
pub struct FederationAuthRequest {
    pub requester_instance: String,
    pub username: String,
    pub scope: String,
    pub nonce: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct FederationAuthGrant {
    pub issuer_instance: String,
    pub token: String,
    pub expires_at: i64,
    pub scope: String,
    pub nonce: String,
}

//———————————————— Share lifecycle ————————————————

#[derive(Debug, Serialize, Deserialize)]
pub struct ShareAnnouncementRequest {
    pub sender_username: String,
    pub sender_instance: String,
    pub recipient_username: String,
    pub recipient_instance: String,
    pub outgoing_share_id: Uuid,
    pub tag_path: String,
    pub name: String,
    pub message: Option<String>,
    pub allow_share_back: bool,
    /// Whether the recipient may propose EXIF edits the owner auto-applies (10 §3). Default `false`
    /// for peers that predate the field.
    #[serde(default)]
    pub allow_exif_edit: bool,
    pub future: bool,
    pub shareback_of: Option<Uuid>,
}
#[derive(Serialize, Deserialize)]
pub struct ShareAnnouncementResponse {
    pub accepted: bool,
    pub auto_accepted: bool,
}

/// Sent by the sender to the recipient to revoke a share.
#[derive(Debug, Serialize, Deserialize)]
pub struct ShareRevokeRequest {
    pub outgoing_share_id: Uuid,
}
/// Sent by the sender to the recipient to accept a share.
#[derive(Debug, Serialize, Deserialize)]
pub struct ShareAcceptRequest {
    pub outgoing_share_id: Uuid,
}
/// Sent by the recipient to the sender to reject a share.
#[derive(Debug, Serialize, Deserialize)]
pub struct ShareRejectRequest {
    pub outgoing_share_id: Uuid,
}

//———————————————— Announcements ————————————————

#[derive(Debug, Serialize, Deserialize)]
pub struct PicturesAnnouncementRequest {
    pub outgoing_share_id: Uuid,
    pub tag_path: String,
    pub sender_username: String,
    pub sender_instance: String,
    pub pictures: Vec<AnnouncedPicture>,
}

/// A picture announced in a [PicturesAnnouncementRequest].
///
/// Carries the owner's EXIF/geo metadata so federated recipients converge on the same metadata the
/// owner holds (and so recipient-side GPS/date rule tagging works on shared pictures).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnouncedPicture {
    pub picture_id: String,
    pub owner_username: String,
    pub owner_instance_domain: String,
    pub picture_token: Uuid,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<i64>,
    /// SHA-256 (lowercase hex) of the owner's original file — the recipient's WebDAV ETag for the
    /// received picture. `None` until the owner's worker has hashed it.
    #[serde(default)]
    pub file_hash: Option<String>,
    /// Metadata-stripped content hash (feature 11 §4), forwarded downstream so recipients can group
    /// byte-identical copies across owners. `None` for a not-yet-hashed or unstrippable picture.
    #[serde(default)]
    pub content_hash: Option<String>,
    /// When the owner generated thumbnails, so the recipient knows a thumbnail variant is fetchable
    /// before requesting a presign. `None` ⇒ only the original is available.
    #[serde(default)]
    pub thumbnails_generated_at: Option<NaiveDateTime>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub blurhash: Option<String>,
    /// The owner's authoritative editable EXIF
    #[serde(default, flatten)]
    pub exif: FullExif,
    /// The origin's already-resolved creator credit (feature 26 §6). Concrete on the wire: the sender
    /// resolves `NULL → @owner:domain` before announcing, and a relay forwards the *origin's* value
    /// (never its own local override). Default empty for peers that predate the field.
    #[serde(default)]
    pub creator: String,
    /// Owner-deletion lifecycle
    #[serde(default)]
    pub owner_deleted_at: Option<NaiveDateTime>,
    #[serde(default)]
    pub owner_purge_at: Option<NaiveDateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PicturesUnannouncementRequest {
    pub outgoing_share_id: Uuid,
    pub sender_username: String,
    pub sender_instance: String,
    pub picture_ids: Vec<String>,
}

//———————————————— Recipient EXIF edit proposal (10 §4.2) ————————————————

/// Sent by the recipient's backend to the **owner's** backend to propose an EXIF edit on a received
/// picture (10 §4.2). The owner re-verifies the grant (an active `OutgoingShare` to `requester` with
/// `allow_exif_edit`), validates the fields, and applies it through its own `edit_picture`
/// write-through — re-announcing the change to all recipients. `set`/`clear` reuse the owned-edit
/// three-state shape ([04 §7.3]).
#[derive(Debug, Serialize, Deserialize)]
pub struct PictureEditRequest {
    /// The owner's picture id (UUID string) — i.e. the recipient's `remote_picture_id`.
    pub picture_id: String,
    /// The proposing recipient's identity, as recorded on the owner's `OutgoingShare.recipient_*`.
    pub requester_username: String,
    pub requester_instance: String,
    #[serde(default)]
    pub set: FullExif,
    #[serde(default)]
    pub clear: Vec<crate::domain::job::ExifField>,
    /// Dedupe key for a retried delivery (the apply itself is last-write-wins idempotent).
    pub idempotency_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PictureEditResponse {
    pub accepted: bool,
}

//———————————————— Presigning ————————————————

#[derive(Debug, Serialize, Deserialize)]
pub struct PresignRequest {
    pub pictures: Vec<PresignRequestItem>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct PresignRequestItem {
    pub picture_token: Uuid,
    pub variant: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PresignResponse {
    pub urls: Vec<PresignResultItem>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct PresignResultItem {
    pub picture_token: Uuid,
    pub url: String,
}

//———————————————— Impl ————————————————

impl AnnouncedPicture {
    /// Build an announce item for `picture` with an already-resolved `picture_token`. The single
    /// source of truth for the picture → announce-item mapping.
    /// The `picture_id` and `(owner_username, owner_instance)` are derived
    /// the same way everywhere — a relayed (received) picture forwards its original owner's id and
    /// identity; an owned picture uses its local id and the sender's identity.
    ///
    /// The announced **EXIF and lifecycle are owner-authoritative** (09 §7/§8): for an owned row they
    /// come from the picture's own columns and `deleted_at`; for a relayed (received) row they come
    /// from the stored owner snapshot (`remote_exif_data`) and `owner_deleted_at`/`owner_purge_at` —
    /// never the relayer's merged `exif_data` or its local `deleted_at`. `owner_purge_at` for an owned
    /// trashed row is the caller-derived `deleted_at + retention`.
    pub fn from_picture(
        picture: &Picture,
        picture_token: Uuid,
        sender_username: &str,
        global_domain: &str,
        owner_purge_at: Option<NaiveDateTime>,
    ) -> Self {
        let (owner_username, owner_instance) = if picture.is_owned() {
            (sender_username.to_string(), global_domain.to_string())
        } else {
            (
                picture.owner_username.clone().unwrap_or_default(),
                picture.owner_instance_domain.clone().unwrap_or_default(),
            )
        };
        // Owner-authoritative EXIF + deletion: owned → the row's own columns / `deleted_at`; received
        // (relay) → the stored owner snapshot / propagated `owner_deleted_at`.
        let (exif, owner_deleted_at) = if picture.is_owned() {
            (picture.full_exif(), picture.deleted_at)
        } else {
            (
                picture
                    .remote_exif_data
                    .as_ref()
                    .map(|j| j.0.clone())
                    .unwrap_or_default(),
                picture.owner_deleted_at,
            )
        };
        // Propagated creator (§6): the stored value, or the owner default resolved to the same owner
        // identity derived above. Never the relayer's local `creator_override`.
        let creator = picture.propagated_creator(&owner_username, &owner_instance);
        Self {
            picture_id: picture
                .remote_picture_id
                .clone()
                .unwrap_or_else(|| picture.id.to_string()),
            picture_token,
            owner_username,
            owner_instance_domain: owner_instance,
            filename: picture.filename.clone(),
            mime_type: picture.mime_type.clone(),
            file_size: picture.file_size,
            file_hash: picture.file_hash.clone(),
            content_hash: picture.content_hash.clone(),
            thumbnails_generated_at: picture.thumbnails_generated_at,
            width: picture.width,
            height: picture.height,
            blurhash: picture.blurhash.clone(),
            exif,
            creator,
            owner_deleted_at,
            owner_purge_at,
        }
    }
}
