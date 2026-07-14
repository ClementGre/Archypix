//! Public share (feature 27): a link-gated *pull* share served entirely by the owner backend.
//!
//! Unlike an `OutgoingShare` there is **no recipient backend and no `IncomingShare`** — coverage is
//! computed live at request time (the picture's tag `<@ tag_path`, owned by the share owner, not
//! deleted, not hidden-dedupe). This type is pure state + permission helpers; the coverage query and
//! presign live in the repository/service layers.

use base64::Engine;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "public_share_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum PublicShareStatus {
    Active,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PublicShare {
    pub id: Uuid,
    pub owner_id: Uuid,
    /// ltree stored as text (queried with `::text`).
    pub tag_path: String,
    pub name: String,
    pub message: Option<String>,
    /// 256-bit base64url secret embedded in the share URL. Never leaves the owner backend except in
    /// the link the creator copies.
    pub token: String,
    /// Argon2 hash of the optional access password. `None` ⇒ no password gate.
    pub password_hash: Option<String>,
    pub expires_at: Option<NaiveDateTime>,
    /// Single "original-extraction" tier: download original + save-a-copy + convert-to-share.
    pub allow_originals: bool,
    /// Anonymous contribution (bytes → the owner's library).
    pub allow_upload: bool,
    /// Authenticated ShareBack (a tag stays on the contributor's side). Forced on when `allow_upload`.
    pub allow_share_back: bool,
    /// Inherited by the derived share minted on Subscribe (`OutgoingShare.allow_exif_edit`).
    pub conv_allow_exif_edit: bool,
    /// Inherited by the derived share minted on Subscribe (`OutgoingShare.future`).
    pub conv_future: bool,
    pub status: PublicShareStatus,
    pub created_at: NaiveDateTime,
    pub revoked_at: Option<NaiveDateTime>,
}

/// The permission tri-flags exposed to the (unauthenticated) view page and honoured server-side.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PublicPermissions {
    pub allow_originals: bool,
    pub allow_upload: bool,
    pub allow_share_back: bool,
    pub conv_allow_exif_edit: bool,
    pub conv_future: bool,
}

impl PublicShare {
    /// Generate a fresh 256-bit unguessable token (base64url, no padding — URL-safe).
    pub fn generate_token() -> String {
        let bytes: [u8; 32] = rand::random();
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Whether the share currently grants access: `active` and not past its optional expiry.
    pub fn is_accessible(&self, now: NaiveDateTime) -> bool {
        self.status == PublicShareStatus::Active && !self.is_expired(now)
    }

    pub fn is_expired(&self, now: NaiveDateTime) -> bool {
        matches!(self.expires_at, Some(exp) if exp <= now)
    }

    pub fn requires_password(&self) -> bool {
        self.password_hash.is_some()
    }

    /// A view-only gallery (`allow_originals = false`) presigns thumbnails only and strips
    /// EXIF/GPS from the JSON payload (§4).
    pub fn view_only(&self) -> bool {
        !self.allow_originals
    }

    pub fn permissions(&self) -> PublicPermissions {
        PublicPermissions {
            allow_originals: self.allow_originals,
            allow_upload: self.allow_upload,
            allow_share_back: self.allow_share_back,
            conv_allow_exif_edit: self.conv_allow_exif_edit,
            conv_future: self.conv_future,
        }
    }
}

/// A creator name typed by an anonymous contributor becomes `#name` (feature 26 sigil). Trims,
/// collapses internal whitespace lightly, and caps the length. Returns `None` for a blank name.
pub fn contribution_creator(contributor_name: &str) -> Option<String> {
    let name = contributor_name.trim();
    if name.is_empty() {
        return None;
    }
    let capped: String = name.chars().take(64).collect();
    Some(format!("#{capped}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(y: i32, mo: u32, d: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
    }

    fn share() -> PublicShare {
        PublicShare {
            id: Uuid::nil(),
            owner_id: Uuid::nil(),
            tag_path: "Photos".into(),
            name: "Album".into(),
            message: None,
            token: PublicShare::generate_token(),
            password_hash: None,
            expires_at: None,
            allow_originals: true,
            allow_upload: false,
            allow_share_back: false,
            conv_allow_exif_edit: false,
            conv_future: true,
            status: PublicShareStatus::Active,
            created_at: at(2024, 1, 1),
            revoked_at: None,
        }
    }

    #[test]
    fn token_is_unguessable_and_urlsafe() {
        let t = PublicShare::generate_token();
        assert!(t.len() >= 43, "256-bit base64url is ~43 chars");
        assert!(!t.contains('+') && !t.contains('/') && !t.contains('='));
        assert_ne!(t, PublicShare::generate_token());
    }

    #[test]
    fn accessibility_honours_status_and_expiry() {
        let mut s = share();
        assert!(s.is_accessible(at(2024, 6, 1)));
        s.expires_at = Some(at(2024, 5, 1));
        assert!(
            !s.is_accessible(at(2024, 6, 1)),
            "past expiry blocks access"
        );
        assert!(s.is_accessible(at(2024, 4, 1)), "before expiry is fine");
        s.expires_at = None;
        s.status = PublicShareStatus::Revoked;
        assert!(!s.is_accessible(at(2024, 6, 1)), "revoked blocks access");
    }

    #[test]
    fn view_only_is_the_negation_of_allow_originals() {
        let mut s = share();
        assert!(!s.view_only());
        s.allow_originals = false;
        assert!(s.view_only());
    }

    #[test]
    fn contribution_creator_stamps_hash_sigil() {
        assert_eq!(contribution_creator("  Alice "), Some("#Alice".to_string()));
        assert_eq!(contribution_creator("   "), None);
        assert_eq!(contribution_creator(""), None);
        assert!(contribution_creator(&"x".repeat(100)).unwrap().len() <= 65);
    }
}
