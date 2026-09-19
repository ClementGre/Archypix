//! Tag metadata (feature 34): the decorative side-table row, its wire patch, and validation.
//!
//! Nothing in the engine reads these values — see 34_tag_metadata.md §2 for why, and §8 for the one
//! WebDAV exception.

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::domain::tag::TagPath;

pub const MAX_DISPLAY_NAME_LEN: usize = 128;
pub const MAX_DESCRIPTION_LEN: usize = 2000;
pub const MAX_WEBDAV_DIR_NAME_LEN: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "tag_order", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TagOrder {
    #[default]
    Manual,
    DateFrom,
    DateTo,
    Path,
    DisplayName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "tag_view_mode", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TagViewMode {
    Direct,
    #[default]
    Subtag,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "tag_subtag_placement", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TagSubtagPlacement {
    Top,
    InSections,
}

// ── Grouping (§3.1) ───────────────────────────────────────────────────────────

/// A sort field that grouping can bucket. Mirrors `PictureSortField` — grouping buckets the *sort*
/// field, because sections must be contiguous runs in the sorted order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupingField {
    CapturedAt,
    IngestedAt,
    UpdatedAt,
    Filename,
    FileSize,
    GeoNear,
    TimeNear,
}

impl GroupingField {
    fn is_date(self) -> bool {
        matches!(self, Self::CapturedAt | Self::IngestedAt | Self::UpdatedAt)
    }
}

/// `magnitude` is one kind carrying a per-field ladder (feature 35 §4) rather than three kinds, so
/// a new bucketable field ships by declaring its ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GroupingKind {
    None,
    Year,
    Quarter,
    Season,
    Month,
    Prefix { chars: u8 },
    Magnitude,
}

/// Per-sort-field grouping, keyed so switching sort back and forth does not lose the setting. A
/// missing key means that field's default: `month` for the date fields, `none` for everything else.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Grouping(pub BTreeMap<GroupingField, GroupingKind>);

impl Grouping {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Each `kind` must be valid **for that field**, so a typo surfaces instead of silently
    /// reverting to the default (§3.2). Unknown keys are already rejected by deserialization.
    pub fn validate(&self) -> Result<(), String> {
        for (field, kind) in &self.0 {
            let ok = match kind {
                GroupingKind::None => true,
                GroupingKind::Year | GroupingKind::Quarter | GroupingKind::Season
                | GroupingKind::Month => field.is_date(),
                GroupingKind::Prefix { chars } => {
                    if !(1..=16).contains(chars) {
                        return Err("grouping prefix.chars must be 1..=16".to_string());
                    }
                    *field == GroupingField::Filename
                }
                GroupingKind::Magnitude => matches!(
                    field,
                    GroupingField::FileSize | GroupingField::GeoNear | GroupingField::TimeNear
                ),
            };
            if !ok {
                return Err(format!("grouping kind {kind:?} is not valid for {field:?}"));
            }
        }
        Ok(())
    }
}

// ── The row ───────────────────────────────────────────────────────────────────

/// One `tag_metadata` row. `Default` is the column-default state: a row equal to it carries no
/// information and is pruned instead of stored (§2).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TagMetadata {
    /// ltree form; `""` is the root view (§3.3).
    pub tag_path: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub cover_picture_id: Option<Uuid>,
    pub color: Option<String>,
    pub date_from: Option<NaiveDateTime>,
    pub date_to: Option<NaiveDateTime>,
    pub show_when_empty: bool,
    pub sort_index: Option<i32>,
    pub children_order: TagOrder,
    /// Direction for `children_order` — the field says what to sort by, this which way round (§7).
    pub children_order_desc: bool,
    pub view_mode: TagViewMode,
    pub subtag_placement: Option<TagSubtagPlacement>,
    pub grouping: Grouping,
    pub webdav_dir_name: Option<String>,
}

impl TagMetadata {
    pub fn new(tag_path: String) -> Self {
        Self {
            tag_path,
            ..Default::default()
        }
    }

    /// Whether every field equals its default — such a row is deleted rather than written, so
    /// browsing with default view settings never litters the table (§2).
    pub fn is_all_default(&self) -> bool {
        let d = Self::new(self.tag_path.clone());
        *self == d
    }

    /// The subset that leaves the instance (§10.1).
    pub fn shared(&self) -> SharedTagMeta {
        SharedTagMeta {
            display_name: self.display_name.clone(),
            description: self.description.clone(),
            color: self.color.clone(),
            cover_remote_picture_id: self.cover_picture_id,
        }
    }
}

/// The decoration that leaves the owner's instance: carried on the share announcement (§10.1) and
/// rendered by the public landing page (feature 27 §15). Not `webdav_dir_name` (the recipient's
/// mount is theirs), not ordering, not view preferences.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SharedTagMeta {
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    /// The **owner's** picture id; the recipient resolves it through `pictures.remote_picture_id`.
    pub cover_remote_picture_id: Option<Uuid>,
}

impl SharedTagMeta {
    pub fn is_empty(&self) -> bool {
        self.display_name.is_none()
            && self.description.is_none()
            && self.color.is_none()
            && self.cover_remote_picture_id.is_none()
    }

    /// Drop any field the local validators reject rather than failing a whole announcement over
    /// cosmetics (§10.1) — inbound decoration comes from a remote instance.
    pub fn sanitized(self) -> Self {
        Self {
            display_name: self.display_name.and_then(|s| validate_display_name(&s).ok()),
            description: self.description.and_then(|s| validate_description(&s).ok()),
            color: self.color.and_then(|s| validate_color(&s).ok()),
            cover_remote_picture_id: self.cover_remote_picture_id,
        }
    }
}

// ── The wire patch ────────────────────────────────────────────────────────────

/// A partial upsert item (§11). Every nullable field is a double-`Option`: absent leaves it
/// unchanged, present `null` clears it. Writes carry only what changed, so a stale flush can never
/// clobber a concurrent change from another device (§4.1).
#[derive(Debug, Default, Deserialize)]
pub struct TagMetadataPatch {
    pub tag_path: String,
    #[serde(default, deserialize_with = "present_field")]
    pub display_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "present_field")]
    pub description: Option<Option<String>>,
    #[serde(default, deserialize_with = "present_field")]
    pub cover_picture_id: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "present_field")]
    pub color: Option<Option<String>>,
    #[serde(default, deserialize_with = "present_field")]
    pub date_from: Option<Option<NaiveDateTime>>,
    #[serde(default, deserialize_with = "present_field")]
    pub date_to: Option<Option<NaiveDateTime>>,
    pub show_when_empty: Option<bool>,
    #[serde(default, deserialize_with = "present_field")]
    pub sort_index: Option<Option<i32>>,
    pub children_order: Option<TagOrder>,
    pub children_order_desc: Option<bool>,
    pub view_mode: Option<TagViewMode>,
    #[serde(default, deserialize_with = "present_field")]
    pub subtag_placement: Option<Option<TagSubtagPlacement>>,
    pub grouping: Option<Grouping>,
    #[serde(default, deserialize_with = "present_field")]
    pub webdav_dir_name: Option<Option<String>>,
}

/// serde helper: a present field (value or `null`) deserializes to `Some(...)`, an absent one to
/// `None`.
fn present_field<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::deserialize(de)?))
}

impl TagMetadataPatch {
    /// Merge onto `base` (the stored row, or a fresh default one), validating the **merged**
    /// result — `date_from <= date_to` has to hold across a write that only touches one side.
    /// `cover_picture_id` ownership is checked by the caller, which has the DB.
    pub fn apply(self, mut base: TagMetadata) -> Result<TagMetadata, String> {
        if let Some(v) = self.display_name {
            base.display_name = v.map(|s| validate_display_name(&s)).transpose()?;
        }
        if let Some(v) = self.description {
            base.description = v.map(|s| validate_description(&s)).transpose()?;
        }
        if let Some(v) = self.cover_picture_id {
            base.cover_picture_id = v;
        }
        if let Some(v) = self.color {
            base.color = v.map(|s| validate_color(&s)).transpose()?;
        }
        if let Some(v) = self.date_from {
            base.date_from = v;
        }
        if let Some(v) = self.date_to {
            base.date_to = v;
        }
        if let Some(v) = self.show_when_empty {
            base.show_when_empty = v;
        }
        if let Some(v) = self.sort_index {
            base.sort_index = v;
        }
        if let Some(v) = self.children_order {
            base.children_order = v;
        }
        if let Some(v) = self.children_order_desc {
            base.children_order_desc = v;
        }
        if let Some(v) = self.view_mode {
            base.view_mode = v;
        }
        if let Some(v) = self.subtag_placement {
            base.subtag_placement = v;
        }
        if let Some(v) = self.grouping {
            v.validate()?;
            base.grouping = v;
        }
        if let Some(v) = self.webdav_dir_name {
            base.webdav_dir_name = v.map(|s| validate_webdav_dir_name(&s)).transpose()?;
        }
        if let (Some(from), Some(to)) = (base.date_from, base.date_to) {
            if from > to {
                return Err("date_from must not be after date_to".to_string());
            }
        }
        Ok(base)
    }
}

// ── Field validators (§3.2) ───────────────────────────────────────────────────

/// Reserved prefixes are permitted here, unlike manual tag assignment — a user may name
/// `SharedToMe.alice_AT_instance_DOT_com` "Alice" (§2). The empty path is permitted **only** as the
/// root sentinel (§3.3); `TagPath::parse` still rejects it, so the carve-out lives here.
pub fn validate_metadata_path(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    TagPath::parse(trimmed, true).map(|p| p.as_ltree().to_string())
}

pub fn validate_display_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("display_name must not be empty — send null to clear it".to_string());
    }
    if name.chars().count() > MAX_DISPLAY_NAME_LEN {
        return Err(format!(
            "display_name must be at most {MAX_DISPLAY_NAME_LEN} characters"
        ));
    }
    if name.chars().any(char::is_control) {
        return Err("display_name must not contain control characters".to_string());
    }
    Ok(name.to_string())
}

pub fn validate_description(raw: &str) -> Result<String, String> {
    // CRLF is normalised rather than rejected — a paste from a Windows client is not an error.
    let desc = raw.replace("\r\n", "\n").replace('\r', "\n");
    let desc = desc.trim();
    if desc.chars().count() > MAX_DESCRIPTION_LEN {
        return Err(format!(
            "description must be at most {MAX_DESCRIPTION_LEN} characters"
        ));
    }
    if desc.chars().any(|c| c.is_control() && c != '\n') {
        return Err("description must not contain control characters".to_string());
    }
    Ok(desc.to_string())
}

pub fn validate_color(raw: &str) -> Result<String, String> {
    let c = raw.trim();
    let valid = c.len() == 7
        && c.starts_with('#')
        && c[1..].chars().all(|ch| ch.is_ascii_hexdigit());
    if !valid {
        return Err("color must be #RRGGBB".to_string());
    }
    Ok(c.to_string())
}

pub fn validate_webdav_dir_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("webdav_dir_name must not be empty — send null to clear it".to_string());
    }
    if name.chars().count() > MAX_WEBDAV_DIR_NAME_LEN {
        return Err(format!(
            "webdav_dir_name must be at most {MAX_WEBDAV_DIR_NAME_LEN} characters"
        ));
    }
    if name.contains('/') || name.chars().any(char::is_control) {
        return Err("webdav_dir_name must not contain '/' or control characters".to_string());
    }
    if name == "." || name == ".." {
        return Err("webdav_dir_name must not be '.' or '..'".to_string());
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(json: serde_json::Value) -> TagMetadataPatch {
        serde_json::from_value(json).unwrap()
    }

    // ── prune-if-all-default (§2) ─────────────────────────────────────────────

    #[test]
    fn fresh_row_is_all_default() {
        assert!(TagMetadata::new("Era.2026".into()).is_all_default());
    }

    #[test]
    fn show_when_empty_alone_is_not_default() {
        let m = TagMetadata {
            show_when_empty: true,
            ..TagMetadata::new("Era.2026".into())
        };
        assert!(!m.is_all_default());
    }

    #[test]
    fn resetting_the_last_field_prunes() {
        // Set grouping, then clear it back to `{}` — the merged row is all-default again (§13.10).
        let base = TagMetadata::new(String::new());
        let set = patch(serde_json::json!({
            "tag_path": "", "grouping": {"captured_at": {"kind": "year"}}
        }))
        .apply(base)
        .unwrap();
        assert!(!set.is_all_default());
        let cleared = patch(serde_json::json!({ "tag_path": "", "grouping": {} }))
            .apply(set)
            .unwrap();
        assert!(cleared.is_all_default());
    }

    // ── partial upsert (§4.1) ─────────────────────────────────────────────────

    #[test]
    fn absent_field_is_untouched_and_null_clears() {
        let base = TagMetadata {
            display_name: Some("Vietnam".into()),
            color: Some("#ff0000".into()),
            ..TagMetadata::new("Era.2026".into())
        };
        let merged = patch(serde_json::json!({ "tag_path": "Era.2026", "color": null }))
            .apply(base)
            .unwrap();
        assert_eq!(merged.display_name.as_deref(), Some("Vietnam"));
        assert_eq!(merged.color, None);
    }

    #[test]
    fn date_order_is_checked_across_a_one_sided_write() {
        let base = TagMetadata {
            date_to: Some("2026-01-01T00:00:00".parse().unwrap()),
            ..TagMetadata::new("Era.2026".into())
        };
        let err = patch(serde_json::json!({
            "tag_path": "Era.2026", "date_from": "2026-06-01T00:00:00"
        }))
        .apply(base)
        .unwrap_err();
        assert!(err.contains("date_from"));
    }

    // ── validators (§3.2) ─────────────────────────────────────────────────────

    #[test]
    fn display_name_accepts_emoji_rejects_blank_and_control() {
        assert_eq!(validate_display_name("  Vietnam 🇻🇳  ").unwrap(), "Vietnam 🇻🇳");
        assert!(validate_display_name("   ").is_err());
        assert!(validate_display_name("a\u{7}b").is_err());
        assert!(validate_display_name(&"a".repeat(MAX_DISPLAY_NAME_LEN + 1)).is_err());
    }

    #[test]
    fn description_keeps_newlines_and_normalises_crlf() {
        assert_eq!(validate_description("a\r\nb").unwrap(), "a\nb");
        assert!(validate_description("a\tb").is_err());
    }

    #[test]
    fn color_is_six_hex_digits() {
        assert_eq!(validate_color(" #A1b2C3 ").unwrap(), "#A1b2C3");
        assert!(validate_color("#abc").is_err());
        assert!(validate_color("red").is_err());
    }

    #[test]
    fn webdav_dir_name_rejects_slash_but_display_name_allows_it() {
        // §13.7: a display name containing `/` is fine (never parsed) and is not propagated here.
        assert!(validate_display_name("AC/DC").is_ok());
        assert!(validate_webdav_dir_name("AC/DC").is_err());
        assert!(validate_webdav_dir_name("..").is_err());
        assert_eq!(validate_webdav_dir_name(" Vietnam 2024 ").unwrap(), "Vietnam 2024");
    }

    #[test]
    fn metadata_path_allows_root_and_reserved_prefixes() {
        assert_eq!(validate_metadata_path("  ").unwrap(), "");
        assert_eq!(
            validate_metadata_path("SharedToMe.alice_AT_x_DOT_com").unwrap(),
            "SharedToMe.alice_AT_x_DOT_com"
        );
        assert!(validate_metadata_path("bad path").is_err());
    }

    // ── grouping (§3.1) ───────────────────────────────────────────────────────

    #[test]
    fn grouping_kind_must_suit_its_field() {
        let ok: Grouping = serde_json::from_value(serde_json::json!({
            "captured_at": {"kind": "season"},
            "filename": {"kind": "prefix", "chars": 3},
            "geo_near": {"kind": "magnitude"},
        }))
        .unwrap();
        assert!(ok.validate().is_ok());

        let bad: Grouping =
            serde_json::from_value(serde_json::json!({"filename": {"kind": "month"}})).unwrap();
        assert!(bad.validate().is_err());

        let bad_chars: Grouping = serde_json::from_value(
            serde_json::json!({"filename": {"kind": "prefix", "chars": 0}}),
        )
        .unwrap();
        assert!(bad_chars.validate().is_err());
    }

    #[test]
    fn grouping_rejects_an_unknown_field() {
        // A typo must surface rather than silently revert to the default (§3.2).
        assert!(
            serde_json::from_value::<Grouping>(serde_json::json!({"capturedAt": {"kind": "year"}}))
                .is_err()
        );
    }
}
