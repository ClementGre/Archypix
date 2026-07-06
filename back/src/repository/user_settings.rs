use crate::domain::user_settings::{UserSettings, VersioningMode};
use archypix_common::error::{map_sqlx_error, AppError};
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;

pub struct UserSettingsRepository;

impl UserSettingsRepository {
    /// The user's `trash_retention_days` (defaulting to 30 when no settings row exists yet) — a
    /// read-only lookup that does **not** create a row. Used by the announcement step (derived
    /// `owner_purge_at`) and the purge sweep.
    #[tracing::instrument(skip(ex), fields(user_id = %user_id))]
    pub async fn trash_retention_days<'e, E>(ex: E, user_id: Uuid) -> Result<i32, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let v = sqlx::query_scalar!(
            "SELECT trash_retention_days FROM user_settings WHERE user_id = $1",
            user_id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(v.unwrap_or(30))
    }

    /// Get settings for the given user, inserting a default row if not yet present.
    #[tracing::instrument(skip(db), fields(user_id = %user_id))]
    pub async fn get_or_default(db: &PgPool, user_id: Uuid) -> Result<UserSettings, AppError> {
        // Insert defaults if not present, then select
        sqlx::query!(
            "INSERT INTO user_settings (user_id) VALUES ($1) ON CONFLICT (user_id) DO NOTHING",
            user_id
        )
        .execute(db)
        .await
        .map_err(map_sqlx_error)?;

        sqlx::query_as!(
            UserSettings,
            r#"SELECT user_id, versioning_mode as "versioning_mode: VersioningMode",
                      trash_retention_days, created_at, updated_at
               FROM user_settings
               WHERE user_id = $1"#,
            user_id,
        )
        .fetch_one(db)
        .await
        .map_err(map_sqlx_error)
    }

    /// Upsert settings. `None` fields keep the existing value (or the column default on insert).
    #[tracing::instrument(skip(db), fields(user_id = %user_id))]
    pub async fn upsert(
        db: &PgPool,
        user_id: Uuid,
        versioning_mode: Option<VersioningMode>,
        trash_retention_days: Option<i32>,
    ) -> Result<UserSettings, AppError> {
        sqlx::query_as!(
            UserSettings,
            r#"INSERT INTO user_settings (user_id, versioning_mode, trash_retention_days)
               VALUES ($1, COALESCE($2, 'none'::versioning_mode), COALESCE($3, 30))
               ON CONFLICT (user_id) DO UPDATE SET
                   versioning_mode = COALESCE($2, user_settings.versioning_mode),
                   trash_retention_days = COALESCE($3, user_settings.trash_retention_days)
               RETURNING user_id, versioning_mode as "versioning_mode: VersioningMode",
                         trash_retention_days, created_at, updated_at"#,
            user_id,
            versioning_mode as Option<VersioningMode>,
            trash_retention_days,
        )
        .fetch_one(db)
        .await
        .map_err(map_sqlx_error)
    }
}
