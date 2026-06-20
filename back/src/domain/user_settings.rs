use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserSettings {
    pub user_id: Uuid,
    pub versioning_mode: VersioningMode,
    pub trash_retention_days: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "versioning_mode", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum VersioningMode {
    None,
    OriginalCopy,
    FullVersioning,
}
