use crate::domain::user::{RefreshToken, UserCredential};
use crate::infra::error::{AppError, map_sqlx_error};
use chrono::{DateTime, Utc};
use sqlx::{Executor, Postgres};
use uuid::Uuid;

pub struct CredentialRepository;

impl CredentialRepository {
    #[tracing::instrument(skip(ex), fields(user_id = %user_id))]
    pub async fn get_password_hash<'e, E>(ex: E, user_id: Uuid) -> Result<Option<String>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_scalar!(
            r#"SELECT password_hash FROM user_credentials WHERE user_id = $1"#,
            user_id
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex, password_hash), fields(user_id = %user_id))]
    pub async fn upsert_password<'e, E>(
        ex: E,
        user_id: Uuid,
        password_hash: &str,
    ) -> Result<UserCredential, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            UserCredential,
            r#"INSERT INTO user_credentials (user_id, password_hash)
               VALUES ($1, $2)
               ON CONFLICT (user_id) DO UPDATE SET password_hash = $2
               RETURNING user_id, password_hash, created_at, updated_at"#,
            user_id,
            password_hash
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)
    }
}

pub struct RefreshTokenRepository;

impl RefreshTokenRepository {
    #[tracing::instrument(skip(ex, token_hash), fields(user_id = %user_id))]
    pub async fn create<'e, E>(
        ex: E,
        user_id: Uuid,
        token_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<RefreshToken, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            RefreshToken,
            r#"INSERT INTO refresh_tokens (user_id, token_hash, expires_at)
               VALUES ($1, $2, $3)
               RETURNING id, user_id, token_hash, expires_at, revoked_at, created_at, updated_at"#,
            user_id,
            token_hash,
            expires_at.naive_utc()
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex, token_hash))]
    pub async fn find_valid<'e, E>(
        ex: E,
        token_hash: &str,
    ) -> Result<Option<RefreshToken>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            RefreshToken,
            r#"SELECT id, user_id, token_hash, expires_at, revoked_at, created_at, updated_at
               FROM refresh_tokens
               WHERE token_hash = $1
                 AND revoked_at IS NULL
                 AND expires_at > (now() at time zone 'utc')"#,
            token_hash
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex))]
    pub async fn revoke<'e, E>(ex: E, token_id: Uuid) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"UPDATE refresh_tokens SET revoked_at = (now() at time zone 'utc') WHERE id = $1"#,
            token_id
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    #[tracing::instrument(skip(ex), fields(user_id = %user_id))]
    pub async fn revoke_all_for_user<'e, E>(ex: E, user_id: Uuid) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"UPDATE refresh_tokens
               SET revoked_at = (now() at time zone 'utc')
               WHERE user_id = $1 AND revoked_at IS NULL"#,
            user_id
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }
}
