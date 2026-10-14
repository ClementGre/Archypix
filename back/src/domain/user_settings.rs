use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserSettings {
    pub user_id: Uuid,
    pub versioning_mode: VersioningMode,
    pub trash_retention_days: i32,
    /// Season grouping convention (feature 34 §3) — the *viewer's*, applied to every photo
    /// regardless of where it was taken.
    pub hemisphere: Hemisphere,
    /// How the web viewer renders sensitive pictures (feature 38 §3).
    pub sensitive_display_mode: SensitiveDisplayMode,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// A partial settings update: `None` keeps the stored value (or the column default on insert).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UserSettingsPatch {
    #[serde(default)]
    pub versioning_mode: Option<VersioningMode>,
    /// Retention window (days) before a soft-deleted owned picture is physically purged (09 §5.1).
    #[serde(default)]
    pub trash_retention_days: Option<i32>,
    #[serde(default)]
    pub hemisphere: Option<Hemisphere>,
    #[serde(default)]
    pub sensitive_display_mode: Option<SensitiveDisplayMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "sensitive_display_mode", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum SensitiveDisplayMode {
    Visible,
    #[default]
    Blurred,
    Redacted,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "hemisphere", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Hemisphere {
    #[default]
    North,
    South,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "versioning_mode", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum VersioningMode {
    None,
    OriginalCopy,
    FullVersioning,
}
