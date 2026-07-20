use crate::domain::job::FullExif;
use crate::domain::picture::Picture;
use chrono::NaiveDateTime;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

//———————————————— Message envelope (feature 28 §5) ————————————————

/// The single authenticated federation wire message. Every verb travels as one of these to
/// `POST /api/federation/message`. The envelope carries the per-message protocol version
/// (`msg_version`, §5.4) alongside the internally-tagged message body.
#[derive(Debug, Serialize, Deserialize)]
pub struct FederationEnvelope {
    pub msg_version: u16,
    #[serde(flatten)]
    pub message: FederationMessage,
}

/// Internally-tagged (`type`) enum of every authenticated federation verb (§5.1). Each variant
/// wraps the pre-existing per-verb request struct — the envelope wraps them, it does not flatten
/// their fields away.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FederationMessage {
    ShareAnnounce(ShareAnnouncementRequest),
    ShareAccept(ShareAcceptRequest),
    ShareReject(ShareRejectRequest),
    ShareRevoke(ShareRevokeRequest),
    PublicShareClaim(PublicShareClaimRequest),
    PicturesAnnounce(PicturesAnnouncementRequest),
    PicturesUnannounce(PicturesUnannouncementRequest),
    PictureEditRequest(PictureEditRequest),
}

/// The response body of `POST /api/federation/message`, internally-tagged so the client can decode
/// it directly into the concrete per-message `Response` (extra `type` tag is ignored).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FederationResponse {
    /// revoke / reject / unannounce — nothing to convey beyond success.
    Ack,
    ShareAnnounce(ShareAnnouncementResponse),
    PicturesAnnounce(PicturesAnnouncementResponse),
    PublicShareClaim(PublicShareClaimResponse),
    PictureEdit(PictureEditResponse),
}

/// A concrete federation message paired with its protocol version and decodable response type.
/// Implemented by each per-verb request struct; drives the generic client `send` and the receiver
/// version check.
pub trait FederationMessageType: Sized {
    /// This message type's protocol version, bumped whenever its request/response shape changes.
    const VERSION: u16;
    /// The wire `type` tag (used in the 426 body + error messages).
    const TYPE_NAME: &'static str;
    /// The response shape (deserialized from the tagged `FederationResponse` body).
    type Response: DeserializeOwned;
    fn into_message(self) -> FederationMessage;
}

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
    /// Relative lifetime (seconds) from issuance — the receiver computes its own `expires_at`
    /// against its own clock, so the TTL is correct under cross-instance clock skew (§4.4).
    pub ttl_secs: i64,
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
#[derive(Debug, Serialize, Deserialize)]
pub struct ShareAnnouncementResponse {
    pub accepted: bool,
    pub auto_accepted: bool,
}

/// Empty success body for the ack-only verbs (revoke / reject / accept / unannounce).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AckResponse {}

#[derive(Debug, Serialize, Deserialize)]
pub struct PicturesAnnouncementResponse {
    pub registered: usize,
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
    /// The owner's monotonic `updated_at` at announce time (feature 28 §7). The recipient applies an
    /// announcement only when this is newer than the last-applied `remote_updated_at`, dropping a
    /// retried *older* announcement that arrived out of order. `None` for peers predating the field
    /// (always applied — no regression).
    #[serde(default)]
    pub owner_updated_at: Option<NaiveDateTime>,
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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PictureEditResponse {
    pub accepted: bool,
}

//———————————————— Public share claim (27 §11) ————————————————

/// Sent by a **visitor's** backend to the **owner's** backend to convert a public share into a real
/// derived `OutgoingShare` (feature 27 §8, recipient-initiated — the reverse of a normal share). The
/// owner validates the token is `active` + `allow_originals`, mints the share, and returns its
/// metadata (federation rule 2 — no callback into the visitor's uncommitted state).
#[derive(Debug, Serialize, Deserialize)]
pub struct PublicShareClaimRequest {
    pub token: String,
    pub requester_username: String,
    pub requester_instance: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PublicShareClaimResponse {
    /// The minted derived `OutgoingShare` id — the visitor creates its matching `IncomingShare`
    /// against it (so the owner's picture-announcement resolves the share).
    pub outgoing_share_id: Uuid,
    pub name: String,
    pub message: Option<String>,
    /// ltree wire form of the covered tag — the visitor lands received pictures under
    /// `/SharedToMe/<owner>/<tag_path>`.
    pub tag_path: String,
    pub allow_share_back: bool,
    pub allow_exif_edit: bool,
    pub future: bool,
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
    /// The owner's presign expiry (epoch seconds), so the recipient caches the URL under
    /// `min(local TTL − margin, remote expiry)` and never advertises a lifetime past the owner's
    /// actual presign (feature 28 §10). `None` for peers predating the field.
    #[serde(default)]
    pub expires_at: Option<i64>,
}

//———————————————— Message-type trait impls (feature 28 §5) ————————————————

macro_rules! impl_federation_message {
    ($req:ty, $version:expr, $tag:literal, $variant:ident, $resp:ty) => {
        impl FederationMessageType for $req {
            const VERSION: u16 = $version;
            const TYPE_NAME: &'static str = $tag;
            type Response = $resp;
            fn into_message(self) -> FederationMessage {
                FederationMessage::$variant(self)
            }
        }
    };
}

impl_federation_message!(ShareAnnouncementRequest, 1, "share_announce", ShareAnnounce, ShareAnnouncementResponse);
impl_federation_message!(ShareAcceptRequest, 1, "share_accept", ShareAccept, AckResponse);
impl_federation_message!(ShareRejectRequest, 1, "share_reject", ShareReject, AckResponse);
impl_federation_message!(ShareRevokeRequest, 1, "share_revoke", ShareRevoke, AckResponse);
impl_federation_message!(PublicShareClaimRequest, 1, "public_share_claim", PublicShareClaim, PublicShareClaimResponse);
impl_federation_message!(PicturesAnnouncementRequest, 1, "pictures_announce", PicturesAnnounce, PicturesAnnouncementResponse);
impl_federation_message!(PicturesUnannouncementRequest, 1, "pictures_unannounce", PicturesUnannounce, AckResponse);
impl_federation_message!(PictureEditRequest, 1, "picture_edit_request", PictureEditRequest, PictureEditResponse);

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
        // Owner's monotonic version (§7 stale-announce guard): owned → the row's `updated_at`;
        // relayed → the last-applied owner value.
        let owner_updated_at = if picture.is_owned() {
            Some(picture.updated_at)
        } else {
            picture.remote_updated_at
        };
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
            owner_updated_at,
        }
    }
}
