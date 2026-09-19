use archypix_common::error::{AppError, map_sqlx_error};
use chrono::NaiveDateTime;
use sqlx::{Executor, Postgres};
use uuid::Uuid;

/// A hierarchy row. `config` is the raw JSONB blob — the service parses it into a
/// `domain::hierarchy::HierarchyConfig`.
#[derive(Debug, Clone)]
pub struct HierarchyRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub config: serde_json::Value,
    pub enabled: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// WebDAV mount settings for one hierarchy.
#[derive(Debug, Clone)]
pub struct WebdavRow {
    pub name: String,
    pub enabled: bool,
    pub webdav_token_enc: Option<Vec<u8>>,
    pub webdav_use_redirect: bool,
}

pub struct HierarchyRepository;

impl HierarchyRepository {
    #[tracing::instrument(skip(ex), fields(owner_id = %owner_id))]
    pub async fn list_by_owner<'e, E>(ex: E, owner_id: Uuid) -> Result<Vec<HierarchyRow>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            HierarchyRow,
            r#"SELECT id, owner_id, name, config as "config!: serde_json::Value",
                      enabled, created_at, updated_at
               FROM hierarchies
               WHERE owner_id = $1
               ORDER BY name"#,
            owner_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex), fields(owner_id = %owner_id, hierarchy_id = %id))]
    pub async fn get_by_owner_and_id<'e, E>(
        ex: E,
        owner_id: Uuid,
        id: Uuid,
    ) -> Result<Option<HierarchyRow>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            HierarchyRow,
            r#"SELECT id, owner_id, name, config as "config!: serde_json::Value",
                      enabled, created_at, updated_at
               FROM hierarchies
               WHERE id = $1 AND owner_id = $2"#,
            id,
            owner_id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex, config), fields(owner_id = %owner_id))]
    pub async fn create<'e, E>(
        ex: E,
        owner_id: Uuid,
        name: &str,
        config: &serde_json::Value,
    ) -> Result<HierarchyRow, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            HierarchyRow,
            r#"INSERT INTO hierarchies (owner_id, name, config)
               VALUES ($1, $2, $3)
               RETURNING id, owner_id, name, config as "config!: serde_json::Value",
                         enabled, created_at, updated_at"#,
            owner_id,
            name,
            config,
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Update name / enabled / config (any subset). Omitted fields are left unchanged.
    #[tracing::instrument(skip(ex, config), fields(owner_id = %owner_id, hierarchy_id = %id))]
    pub async fn update<'e, E>(
        ex: E,
        owner_id: Uuid,
        id: Uuid,
        name: Option<&str>,
        enabled: Option<bool>,
        config: Option<&serde_json::Value>,
    ) -> Result<Option<HierarchyRow>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            HierarchyRow,
            r#"UPDATE hierarchies
               SET name = COALESCE($3, name),
                   enabled = COALESCE($4, enabled),
                   config = COALESCE($5, config)
               WHERE id = $1 AND owner_id = $2
               RETURNING id, owner_id, name, config as "config!: serde_json::Value",
                         enabled, created_at, updated_at"#,
            id,
            owner_id,
            name,
            enabled,
            config,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Load the WebDAV mount settings for one hierarchy (06_webdav.md §3).
    #[tracing::instrument(skip(ex), fields(owner_id = %owner_id, hierarchy_id = %id))]
    pub async fn get_webdav<'e, E>(
        ex: E,
        owner_id: Uuid,
        id: Uuid,
    ) -> Result<Option<WebdavRow>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            WebdavRow,
            r#"SELECT name, enabled, webdav_token_enc, webdav_use_redirect
               FROM hierarchies
               WHERE id = $1 AND owner_id = $2"#,
            id,
            owner_id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Store (or rotate) the encrypted WebDAV token. Returns false if the hierarchy is
    /// not owned by the user.
    #[tracing::instrument(skip(ex, token_enc), fields(owner_id = %owner_id, hierarchy_id = %id))]
    pub async fn set_webdav_token<'e, E>(
        ex: E,
        owner_id: Uuid,
        id: Uuid,
        token_enc: &[u8],
    ) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let res = sqlx::query!(
            "UPDATE hierarchies SET webdav_token_enc = $3 WHERE id = $1 AND owner_id = $2",
            id,
            owner_id,
            token_enc,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected() > 0)
    }

    /// Toggle the WebDAV read strategy (presigned redirect vs backend proxy, §6).
    #[tracing::instrument(skip(ex), fields(owner_id = %owner_id, hierarchy_id = %id))]
    pub async fn set_webdav_use_redirect<'e, E>(
        ex: E,
        owner_id: Uuid,
        id: Uuid,
        use_redirect: bool,
    ) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let res = sqlx::query!(
            "UPDATE hierarchies SET webdav_use_redirect = $3 WHERE id = $1 AND owner_id = $2",
            id,
            owner_id,
            use_redirect,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected() > 0)
    }

    #[tracing::instrument(skip(ex), fields(owner_id = %owner_id, hierarchy_id = %id))]
    pub async fn delete<'e, E>(ex: E, owner_id: Uuid, id: Uuid) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let res = sqlx::query!(
            "DELETE FROM hierarchies WHERE id = $1 AND owner_id = $2",
            id,
            owner_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected() > 0)
    }
}

