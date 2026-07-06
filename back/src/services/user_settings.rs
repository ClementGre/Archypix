use crate::domain::user_settings::{UserSettings, VersioningMode};
use crate::repository::user_settings::UserSettingsRepository;
use archypix_common::error::AppError;
use sqlx::PgPool;
use uuid::Uuid;

#[tracing::instrument(skip(db), fields(user_id = %user_id))]
pub async fn get(db: &PgPool, user_id: Uuid) -> Result<UserSettings, AppError> {
    UserSettingsRepository::get_or_default(db, user_id).await
}

#[tracing::instrument(skip(db), fields(user_id = %user_id))]
pub async fn update(
    db: &PgPool,
    user_id: Uuid,
    versioning_mode: Option<VersioningMode>,
    trash_retention_days: Option<i32>,
) -> Result<UserSettings, AppError> {
    if let Some(days) = trash_retention_days {
        if !(1..=3650).contains(&days) {
            return Err(AppError::BadRequest(
                "trash_retention_days must be between 1 and 3650".to_string(),
            ));
        }
    }
    UserSettingsRepository::upsert(db, user_id, versioning_mode, trash_retention_days).await
}
