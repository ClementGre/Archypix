use crate::domain::job::{CameraExif, FullExif};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Picture {
    pub id: Uuid,
    pub local_user_id: Uuid,
    /// Set only for pictures received via federation (not owned by this instance's user).
    pub remote_picture_id: Option<String>,
    pub owner_username: Option<String>,
    pub owner_instance_domain: Option<String>,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<i64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// Camera/lens EXIF only — the promoted fields (`captured_at`, `gps_*`, `orientation`) live in
    /// their own columns. For received rows this is the camera part of the materialised merge.
    pub exif_data: Json<CameraExif>,
    pub metadata: Json<serde_json::Value>,
    pub deleted_at: Option<NaiveDateTime>,
    /// Why this row was soft-deleted (set together with `deleted_at`). Only `Manual` is produced
    /// today; the other variants are reserved for the physical-copy/dedup work (spec 11).
    pub deleted_reason: Option<DeletedReason>,
    /// Received rows only: the owner's soft-delete timestamp, propagated on announcement. Distinct
    /// from `deleted_at` (the recipient's own local trash). Drives the grace-window badge.
    pub owner_deleted_at: Option<NaiveDateTime>,
    /// Received rows only: the owner's announced purge deadline (their `deleted_at + retention`).
    pub owner_purge_at: Option<NaiveDateTime>,
    /// Received rows only: the owner's authoritative EXIF snapshot (canonical full editable-EXIF
    /// JSON), refreshed on every announcement. `exif_data` for received rows is the merge of this
    /// with `local_exif_overrides`.
    pub remote_exif_data: Option<Json<FullExif>>,
    /// Received rows only: the recipient's sticky per-field EXIF overrides (sparse key set, `null` to claim the field as empty).
    pub local_exif_overrides: Option<Json<serde_json::Value>>,
    pub captured_at: Option<NaiveDateTime>,
    pub ingested_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub blurhash: Option<String>,
    pub gps_lat: Option<f64>,
    pub gps_lng: Option<f64>,
    pub gps_alt: Option<i32>,
    pub orientation: Option<i16>,
    pub thumbnails_generated_at: Option<NaiveDateTime>,
    /// SHA-256 hex digest of the stored file. Used as WebDAV ETag.
    pub file_hash: Option<String>,
    /// Convergence of the S3 original's embedded EXIF versus this row (the source of truth).
    pub exif_sync_status: ExifSyncStatus,
    /// Hash stable across EXIF edits, changes on a visual re-encode. `None` for a format the worker cannot strip (dedup then groups by `file_hash`).
    pub content_hash: Option<String>,
    /// Provenance of a physical copy (feature 11 §3) — the **original owner identity** the copy was
    /// rescued from (root-resolved across copy chains), for display and survivor selection. All
    /// `None` for a normal upload/received row; set together on a copy.
    pub copy_source_owner_username: Option<String>,
    pub copy_source_owner_instance: Option<String>,
    pub copy_source_picture_id: Option<String>,
    /// Owner-authoritative creator credit (feature 26 §4). `NULL` ⇒ the owner default
    /// (`@username:global_domain`, resolved on read/announce). For a received row it holds the
    /// origin's already-resolved, propagated value. Format convention: `@user:domain` (identity),
    /// `#name` (anonymous uploader), or plain text.
    pub creator: Option<String>,
    /// Recipient-local relabel of the creator (received pictures only). Never propagates, not even
    /// transitively. Displayed creator = `coalesce(creator_override, creator, owner_identity)`.
    pub creator_override: Option<String>,
}

/// Why a picture was soft-deleted (set with `deleted_at`). Feature 09 only produces `Manual`; the
/// other reasons are reserved for the physical-copy/dedup work (spec 11) so no later migration is
/// needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "picture_deleted_reason", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum DeletedReason {
    Manual,
    Boomerang,
    ContentDedupe,
}

/// Convergence state of a picture's embedded-file EXIF versus the DB row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "picture_exif_sync_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ExifSyncStatus {
    Synced,
    Pending,
    Unsupported,
    PendingJobCreation,
}

impl Picture {
    /// The picture's effective editable EXIF as a [`FullExif`] — the promoted columns plus the
    /// camera/lens fields from `exif_data`. For owned rows this is authoritative; for received rows
    /// it is the materialised merge. Used as an edit's revert baseline and convergence comparison.
    pub fn full_exif(&self) -> FullExif {
        FullExif {
            captured_at: self.captured_at,
            gps_lat: self.gps_lat,
            gps_lng: self.gps_lng,
            gps_alt: self.gps_alt,
            orientation: self.orientation,
            camera: self.exif_data.0.clone(),
        }
    }
}

impl Picture {
    pub fn is_owned(&self) -> bool {
        self.remote_picture_id.is_none()
    }

    /// The owner-default creator identity string `@username:domain` (feature 26 §3).
    pub fn format_identity(username: &str, domain: &str) -> String {
        format!("@{username}:{domain}")
    }

    /// The identity to attribute an owner-default (`creator IS NULL`) to: for an owned row the local
    /// holder (`local_username`/`global_domain`), for a received row the stored origin owner. Returns
    /// `None` when the owner is unresolvable (e.g. a deleted account with no stored username).
    fn owner_default_identity(&self, local_username: &str, global_domain: &str) -> Option<String> {
        if self.is_owned() {
            Some(Self::format_identity(local_username, global_domain))
        } else {
            match (
                self.owner_username.as_deref(),
                self.owner_instance_domain.as_deref(),
            ) {
                (Some(u), Some(d)) if !u.is_empty() => Some(Self::format_identity(u, d)),
                _ => None,
            }
        }
    }

    /// The **propagated** creator (no local override) — the value announced downstream and the
    /// origin baseline shown for a received picture: `coalesce(creator, owner_default)` (§6). For an
    /// owned row `local_username`/`global_domain` is the owner; for a received row they are ignored
    /// (the stored origin owner is used). Falls back to a neutral placeholder if unresolvable (§9).
    pub fn propagated_creator(&self, local_username: &str, global_domain: &str) -> String {
        self.creator
            .clone()
            .filter(|c| !c.is_empty())
            .or_else(|| self.owner_default_identity(local_username, global_domain))
            .unwrap_or_else(|| "Unknown".to_string())
    }

    /// The creator to display to the holder: `coalesce(creator_override, creator, owner_identity)`
    /// (§4/§5). `local_username`/`global_domain` resolve the owner default for owned rows.
    pub fn display_creator(&self, local_username: &str, global_domain: &str) -> String {
        self.creator_override
            .clone()
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| self.propagated_creator(local_username, global_domain))
    }
}

/// Validate a manually-entered creator (§3 sigil guard). The `#…` contribution sigil is system-owned
/// (feature 27 anonymous uploads) and always rejected. A leading `@` is accepted **only** as a
/// well-formed `@username:domain` identity — the creator autocomplete lets a user attribute a picture
/// to a real account (resolver-verified client-side); creator is pure attribution and grants no
/// access, so this is not an authorization principal. A malformed `@…` (no `:domain`) is rejected so
/// it can't masquerade as an identity. A blank value is accepted (the caller normalises it to `None`).
pub fn validate_manual_creator(value: &str) -> Result<(), String> {
    let v = value.trim_start();
    if v.starts_with('#') {
        return Err(
            "Creator may not begin with '#' (reserved for public-share contributions)".to_string(),
        );
    }
    if v.starts_with('@') && !is_well_formed_identity(v) {
        return Err("An '@' creator must be a full @username:domain identity".to_string());
    }
    Ok(())
}

/// Whether `s` is a well-formed `@username:domain` identity (both parts non-empty, single `:`).
fn is_well_formed_identity(s: &str) -> bool {
    let Some(rest) = s.strip_prefix('@') else {
        return false;
    };
    match rest.split_once(':') {
        Some((user, domain)) => !user.is_empty() && !domain.is_empty() && !domain.contains(':'),
        None => false,
    }
}

/// Transient upload state stored in Redis during the presigned-URL upload window.
#[derive(Debug, Serialize, Deserialize)]
pub struct UploadSession {
    pub user_id: Uuid,
    pub picture_id: Uuid,
    pub s3_key_staging: String,
    pub filename: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PictureVersion {
    pub id: Uuid,
    pub picture_id: Uuid,
    pub version_number: i32,
    pub file_size: Option<i64>,
    pub mime_type: Option<String>,
    pub created_at: NaiveDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::types::Json;

    /// A minimal owned picture for the creator-resolution unit tests.
    fn owned() -> Picture {
        let now = chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        Picture {
            id: Uuid::nil(),
            local_user_id: Uuid::nil(),
            remote_picture_id: None,
            owner_username: None,
            owner_instance_domain: None,
            filename: None,
            mime_type: None,
            file_size: None,
            width: None,
            height: None,
            exif_data: Json(CameraExif::default()),
            metadata: Json(serde_json::json!({})),
            deleted_at: None,
            deleted_reason: None,
            owner_deleted_at: None,
            owner_purge_at: None,
            remote_exif_data: None,
            local_exif_overrides: None,
            captured_at: None,
            ingested_at: now,
            updated_at: now,
            blurhash: None,
            gps_lat: None,
            gps_lng: None,
            gps_alt: None,
            orientation: None,
            thumbnails_generated_at: None,
            file_hash: None,
            exif_sync_status: ExifSyncStatus::Synced,
            content_hash: None,
            copy_source_owner_username: None,
            copy_source_owner_instance: None,
            copy_source_picture_id: None,
            creator: None,
            creator_override: None,
        }
    }

    /// A received picture from `@alice:alice.test`.
    fn received() -> Picture {
        Picture {
            remote_picture_id: Some(Uuid::nil().to_string()),
            owner_username: Some("alice".to_string()),
            owner_instance_domain: Some("alice.test".to_string()),
            ..owned()
        }
    }

    #[test]
    fn sigil_guard_rejects_system_sigils() {
        // `#…` (contribution sigil) is always system-owned.
        assert!(validate_manual_creator("#anon").is_err());
        // A well-formed `@username:domain` identity is accepted (creator autocomplete).
        assert!(validate_manual_creator("@bob:bob.test").is_ok());
        // A malformed `@…` (no `:domain`) can't masquerade as an identity.
        assert!(validate_manual_creator("@sneaky").is_err());
        assert!(validate_manual_creator("  @sneaky").is_err()); // leading whitespace stripped first
        assert!(validate_manual_creator("@:bob.test").is_err()); // empty username
        assert!(validate_manual_creator("@bob:").is_err()); // empty domain
        assert!(validate_manual_creator("Grandpa's camera").is_ok());
        assert!(validate_manual_creator("").is_ok());
    }

    #[test]
    fn owned_null_creator_resolves_to_owner_default() {
        let p = owned();
        assert_eq!(p.display_creator("bob", "bob.test"), "@bob:bob.test");
        assert_eq!(p.propagated_creator("bob", "bob.test"), "@bob:bob.test");
    }

    #[test]
    fn owned_set_creator_wins_over_default() {
        let p = Picture {
            creator: Some("Grandpa's camera".to_string()),
            ..owned()
        };
        assert_eq!(p.display_creator("bob", "bob.test"), "Grandpa's camera");
    }

    #[test]
    fn received_uses_stored_origin_creator_and_owner_fallback() {
        // Post-announce: creator is set to the origin's resolved value.
        let p = Picture {
            creator: Some("@alice:alice.test".to_string()),
            ..received()
        };
        assert_eq!(p.display_creator("bob", "bob.test"), "@alice:alice.test");
        // Legacy received row (creator NULL) falls back to the stored owner identity, not the holder.
        let legacy = received();
        assert_eq!(
            legacy.display_creator("bob", "bob.test"),
            "@alice:alice.test"
        );
    }

    #[test]
    fn override_shows_locally_but_never_propagates() {
        let p = Picture {
            creator: Some("@alice:alice.test".to_string()),
            creator_override: Some("Aunt May".to_string()),
            ..received()
        };
        // Display honours the recipient's local relabel …
        assert_eq!(p.display_creator("bob", "bob.test"), "Aunt May");
        // … but the propagated value (announced downstream) is always the origin's, never the override.
        assert_eq!(p.propagated_creator("bob", "bob.test"), "@alice:alice.test");
    }

    #[test]
    fn unresolvable_owner_falls_back_to_placeholder() {
        // Received row with no stored owner identity and no creator ⇒ neutral placeholder, not a panic.
        let p = Picture {
            owner_username: None,
            owner_instance_domain: None,
            ..received()
        };
        assert_eq!(p.display_creator("bob", "bob.test"), "Unknown");
    }
}
