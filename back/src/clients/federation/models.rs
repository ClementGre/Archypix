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
    pub allow_share_back: bool,
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
/// owner holds (and so recipient-side `gps_within_bbox`/date tagging works on shared pictures).
#[derive(Debug, Serialize, Deserialize)]
pub struct AnnouncedPicture {
    pub picture_id: String,
    pub owner_username: String,
    pub owner_instance_domain: String,
    pub picture_token: Uuid,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<i64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub captured_at: Option<NaiveDateTime>,
    pub blurhash: Option<String>,
    #[serde(default)]
    pub gps_lat: Option<f64>,
    #[serde(default)]
    pub gps_lng: Option<f64>,
    #[serde(default)]
    pub gps_alt: Option<i32>,
    #[serde(default)]
    pub orientation: Option<i16>,
    #[serde(default)]
    pub exif_data: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PicturesUnannouncementRequest {
    pub outgoing_share_id: Uuid,
    pub sender_username: String,
    pub sender_instance: String,
    pub picture_ids: Vec<String>,
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
    pub fn from_picture(
        picture: &Picture,
        picture_token: Uuid,
        sender_username: &str,
        global_domain: &str,
    ) -> Self {
        let (owner_username, owner_instance) = if picture.is_owned() {
            (sender_username.to_string(), global_domain.to_string())
        } else {
            (
                picture.owner_username.clone().unwrap_or_default(),
                picture.owner_instance_domain.clone().unwrap_or_default(),
            )
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
            width: picture.width,
            height: picture.height,
            captured_at: picture.captured_at,
            blurhash: picture.blurhash.clone(),
            gps_lat: picture.gps_lat,
            gps_lng: picture.gps_lng,
            gps_alt: picture.gps_alt,
            orientation: picture.orientation,
            exif_data: Some(picture.exif_data.0.clone()),
        }
    }
}
