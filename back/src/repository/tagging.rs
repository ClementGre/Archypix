use crate::domain::tagging::{ServiceType, TaggingService};
use crate::infra::error::{AppError, map_sqlx_error};
use sqlx::{Executor, Postgres};
use uuid::Uuid;

pub struct TaggingServiceRepository;

impl TaggingServiceRepository {
    #[tracing::instrument(skip(ex), fields(owner_id = %owner_id))]
    pub async fn list_by_owner<'e, E>(
        ex: E,
        owner_id: Uuid,
    ) -> Result<Vec<TaggingService>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            TaggingService,
            r#"SELECT id, owner_id, name,
                      service_type as "service_type: ServiceType",
                      requires::text[] as "requires!", excludes::text[] as "excludes!",
                      enabled, position, config as "config!: serde_json::Value",
                      last_invalidated_at, last_error_at, last_error_msg, created_at, updated_at
               FROM tagging_services
               WHERE owner_id = $1
               ORDER BY CASE WHEN service_type = 'shared_tag_mapping' THEN 0 ELSE 1 END,
                        position, created_at"#,
            owner_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Like `list_by_owner` but returns only enabled services (used by the pipeline loop).
    ///
    /// Order: SharedTagMapping always first, then Rule and Segmentation interleaved by `position`.
    #[tracing::instrument(skip(ex), fields(owner_id = %owner_id))]
    pub async fn list_enabled_by_owner<'e, E>(
        ex: E,
        owner_id: Uuid,
    ) -> Result<Vec<TaggingService>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            TaggingService,
            r#"SELECT id, owner_id, name,
                      service_type as "service_type: ServiceType",
                      requires::text[] as "requires!", excludes::text[] as "excludes!",
                      enabled, position, config as "config!: serde_json::Value",
                      last_invalidated_at, last_error_at, last_error_msg, created_at, updated_at
               FROM tagging_services
               WHERE owner_id = $1 AND enabled = true
               ORDER BY CASE WHEN service_type = 'shared_tag_mapping' THEN 0 ELSE 1 END,
                        position, created_at"#,
            owner_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex), fields(owner_id = %owner_id))]
    pub async fn get_by_owner_and_id<'e, E>(
        ex: E,
        owner_id: Uuid,
        service_id: Uuid,
    ) -> Result<Option<TaggingService>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            TaggingService,
            r#"SELECT id, owner_id, name,
                      service_type as "service_type: ServiceType",
                      requires::text[] as "requires!", excludes::text[] as "excludes!",
                      enabled, position, config as "config!: serde_json::Value",
                      last_invalidated_at, last_error_at, last_error_msg, created_at, updated_at
               FROM tagging_services
               WHERE id = $1 AND owner_id = $2"#,
            service_id,
            owner_id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Create a service with an explicit `config` payload (type-specific, validated by the caller).
    #[tracing::instrument(skip(ex, requires, excludes, config), fields(owner_id = %owner_id))]
    pub async fn create<'e, E>(
        ex: E,
        owner_id: Uuid,
        service_type: ServiceType,
        name: &str,
        requires: &[String],
        excludes: &[String],
        config: &serde_json::Value,
    ) -> Result<TaggingService, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            TaggingService,
            r#"INSERT INTO tagging_services (owner_id, service_type, name, requires, excludes, config, position)
               VALUES ($1, $2, $5, $3::ltree[], $4::ltree[], $6,
                       COALESCE((SELECT MAX(position) FROM tagging_services WHERE owner_id = $1), -1) + 1)
               RETURNING id, owner_id, name,
                         service_type as "service_type: ServiceType",
                         requires::text[] as "requires!", excludes::text[] as "excludes!",
                         enabled, position, config as "config!: serde_json::Value",
                         last_invalidated_at, last_error_at, last_error_msg, created_at, updated_at"#,
            owner_id,
            service_type as ServiceType,
            requires as &[String],
            excludes as &[String],
            name,
            config,
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Update optional service-level fields; pass `None` to leave a field unchanged. `config` is
    /// managed by the type-specific helpers below, not here.
    #[tracing::instrument(skip(ex, requires, excludes), fields(owner_id = %owner_id))]
    pub async fn update<'e, E>(
        ex: E,
        owner_id: Uuid,
        service_id: Uuid,
        name: Option<&str>,
        enabled: Option<bool>,
        requires: Option<&[String]>,
        excludes: Option<&[String]>,
    ) -> Result<Option<TaggingService>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            TaggingService,
            r#"UPDATE tagging_services
               SET enabled    = COALESCE($3, enabled),
                   requires   = COALESCE($4::ltree[], requires),
                   excludes   = COALESCE($5::ltree[], excludes),
                   name       = COALESCE($6, name),
                   updated_at = now() AT TIME ZONE 'utc'
               WHERE id = $1 AND owner_id = $2
               RETURNING id, owner_id, name,
                         service_type as "service_type: ServiceType",
                         requires::text[] as "requires!", excludes::text[] as "excludes!",
                         enabled, position, config as "config!: serde_json::Value",
                         last_invalidated_at, last_error_at, last_error_msg, created_at, updated_at"#,
            service_id,
            owner_id,
            enabled as Option<bool>,
            requires as Option<&[String]>,
            excludes as Option<&[String]>,
            name as Option<&str>,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Replace the whole `config` of a service of the given type (validated by the caller). Returns
    /// `false` if the service does not exist, is not owned, or is of a different type.
    #[tracing::instrument(skip(ex, config), fields(owner_id = %owner_id))]
    pub async fn set_config<'e, E>(
        ex: E,
        owner_id: Uuid,
        service_id: Uuid,
        service_type: ServiceType,
        config: &serde_json::Value,
    ) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let result = sqlx::query!(
            r#"UPDATE tagging_services
               SET config = $4, updated_at = now() AT TIME ZONE 'utc'
               WHERE id = $1 AND owner_id = $2 AND service_type = $3"#,
            service_id,
            owner_id,
            service_type as ServiceType,
            config,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(result.rows_affected() > 0)
    }

    /// Overwrite `requires`, `excludes`, and `config` and bump `last_invalidated_at` in one step —
    /// used by the tag-rename cascade (edge case §7) after rewriting a service's tag references. Any
    /// gating/config change invalidates the service, so the pipeline re-derives its tags.
    #[tracing::instrument(skip(ex, requires, excludes, config))]
    pub async fn replace_gating_and_config<'e, E>(
        ex: E,
        service_id: Uuid,
        requires: &[String],
        excludes: &[String],
        config: &serde_json::Value,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"UPDATE tagging_services
               SET requires = $2::ltree[], excludes = $3::ltree[], config = $4,
                   last_invalidated_at = now() AT TIME ZONE 'utc',
                   updated_at = now() AT TIME ZONE 'utc'
               WHERE id = $1"#,
            service_id,
            requires as &[String],
            excludes as &[String],
            config,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Bump `last_invalidated_at` on a specific service to NOW(), marking all pictures dirty.
    /// Called after any configuration change (config replace, enable/disable).
    #[tracing::instrument(skip(ex))]
    pub async fn touch_invalidated<'e, E>(ex: E, service_id: Uuid) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"UPDATE tagging_services
               SET last_invalidated_at = now() AT TIME ZONE 'utc'
               WHERE id = $1"#,
            service_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Record a pipeline evaluation error on a service, or clear it (pass `None`).
    #[tracing::instrument(skip(ex))]
    pub async fn set_error<'e, E>(
        ex: E,
        service_id: Uuid,
        error_msg: Option<&str>,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if let Some(msg) = error_msg {
            sqlx::query!(
                r#"UPDATE tagging_services
                   SET last_error_at  = now() AT TIME ZONE 'utc',
                       last_error_msg = $2
                   WHERE id = $1"#,
                service_id,
                msg,
            )
            .execute(ex)
            .await
            .map_err(map_sqlx_error)?;
        } else {
            sqlx::query!(
                r#"UPDATE tagging_services
                   SET last_error_at  = NULL,
                       last_error_msg = NULL
                   WHERE id = $1"#,
                service_id,
            )
            .execute(ex)
            .await
            .map_err(map_sqlx_error)?;
        }
        Ok(())
    }

    #[tracing::instrument(skip(ex), fields(owner_id = %owner_id))]
    pub async fn delete<'e, E>(ex: E, owner_id: Uuid, service_id: Uuid) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let result = sqlx::query!(
            "DELETE FROM tagging_services WHERE id = $1 AND owner_id = $2",
            service_id,
            owner_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(result.rows_affected() > 0)
    }

    /// Atomically reassign positions for Rule and Segmentation services.
    ///
    /// `ordered_ids` is the complete desired order — each ID gets `position = its index`.
    /// Returns an error if any ID does not belong to `owner_id` or is a `SharedTagMapping`.
    #[tracing::instrument(skip(ex, ordered_ids), fields(owner_id = %owner_id))]
    pub async fn reorder_services<'e, E>(
        ex: E,
        owner_id: Uuid,
        ordered_ids: &[Uuid],
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if ordered_ids.is_empty() {
            return Ok(());
        }
        let positions: Vec<i32> = (0..ordered_ids.len() as i32).collect();
        let updated = sqlx::query_scalar!(
            r#"UPDATE tagging_services ts
               SET position = ord.pos
               FROM (
                   SELECT unnest($1::uuid[]) AS id,
                          unnest($3::int[])  AS pos
               ) AS ord
               WHERE ts.id = ord.id
                 AND ts.owner_id = $2
                 AND ts.service_type <> 'shared_tag_mapping'::service_type
               RETURNING ts.id"#,
            ordered_ids as &[Uuid],
            owner_id,
            &positions as &[i32],
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)?;

        if updated.len() != ordered_ids.len() {
            return Err(AppError::BadRequest(
                "one or more service IDs not found, not owned by you, or are SharedTagMapping services".into(),
            ));
        }
        Ok(())
    }
}
