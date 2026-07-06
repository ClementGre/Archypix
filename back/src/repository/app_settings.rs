//! DB runtime-settings overrides (feature 23 §4.3). The layered [`Settings`](archypix_common::settings)
//! engine merges these over `default → env`; this repo just reads/writes the raw override rows.

use archypix_common::error::AppError;
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;

pub struct AppSettingsRepository;

impl AppSettingsRepository {
    /// All DB overrides as a `key → value` map, for building a settings snapshot.
    pub async fn load_all(db: &PgPool) -> Result<HashMap<String, Value>, AppError> {
        let rows = sqlx::query!("SELECT key, value FROM app_settings")
            .fetch_all(db)
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(rows.into_iter().map(|r| (r.key, r.value)).collect())
    }

    /// Insert or replace one override.
    pub async fn upsert(db: &PgPool, key: &str, value: &Value) -> Result<(), AppError> {
        sqlx::query!(
            "INSERT INTO app_settings (key, value, updated_at) VALUES ($1, $2, now())
             ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = now()",
            key,
            value
        )
            .execute(db)
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(())
    }

    /// Clear one override (revert to env/default).
    pub async fn delete(db: &PgPool, key: &str) -> Result<(), AppError> {
        sqlx::query!("DELETE FROM app_settings WHERE key = $1", key)
            .execute(db)
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(())
    }
}
