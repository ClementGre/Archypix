use crate::domain::user::User;
use archypix_common::error::{map_sqlx_error, AppError};
use sqlx::{Executor, Postgres};
use uuid::Uuid;

pub struct UserRepository;

impl UserRepository {
    #[tracing::instrument(skip(ex))]
    pub async fn find_by_username<'e, E>(ex: E, username: &str) -> Result<Option<User>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            User,
            r#"SELECT id, username, email, display_name, is_admin, created_at, updated_at
               FROM users WHERE username = $1"#,
            username,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex), fields(user_id = %user_id))]
    pub async fn find_by_id<'e, E>(ex: E, user_id: Uuid) -> Result<Option<User>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            User,
            r#"SELECT id, username, email, display_name, is_admin, created_at, updated_at
               FROM users WHERE id = $1"#,
            user_id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex))]
    pub async fn list<'e, E>(ex: E) -> Result<Vec<User>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            User,
            r#"SELECT id, username, email, display_name, is_admin, created_at, updated_at
               FROM users ORDER BY created_at DESC"#
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex))]
    pub async fn create<'e, E>(
        ex: E,
        username: &str,
        email: &str,
        display_name: &str,
        is_admin: bool,
        invited_by: Option<&str>,
    ) -> Result<User, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            User,
            r#"INSERT INTO users (username, email, display_name, is_admin, invited_by)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING id, username, email, display_name, is_admin, created_at, updated_at"#,
            username,
            email,
            display_name,
            is_admin,
            invited_by
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex), fields(user_id = %user_id))]
    pub async fn update<'e, E>(
        ex: E,
        user_id: Uuid,
        display_name: Option<&str>,
        is_admin: Option<bool>,
    ) -> Result<User, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            User,
            r#"UPDATE users
               SET display_name = COALESCE($2, display_name),
                   is_admin = COALESCE($3, is_admin)
               WHERE id = $1
               RETURNING id, username, email, display_name, is_admin, created_at, updated_at"#,
            user_id,
            display_name,
            is_admin
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex), fields(user_id = %user_id))]
    pub async fn update_profile<'e, E>(
        ex: E,
        user_id: Uuid,
        display_name: Option<&str>,
        email: Option<&str>,
    ) -> Result<User, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            User,
            r#"UPDATE users
               SET display_name = COALESCE($2, display_name),
                   email = COALESCE($3, email)
               WHERE id = $1
               RETURNING id, username, email, display_name, is_admin, created_at, updated_at"#,
            user_id,
            display_name,
            email
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex), fields(user_id = %user_id))]
    pub async fn delete<'e, E>(ex: E, user_id: Uuid) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(r#"DELETE FROM users WHERE id = $1"#, user_id)
            .execute(ex)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }
}
