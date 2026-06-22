use crate::domain::tagging::{
    RuleTaggingRule, SegmentationRule, ServiceType, SharedTagMappingRule, TaggingService,
};
use crate::infra::error::{AppError, map_sqlx_error};
use chrono::NaiveDateTime;
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
                      enabled, position, last_invalidated_at, last_error_at, last_error_msg,
                      created_at, updated_at
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
                      enabled, position, last_invalidated_at, last_error_at, last_error_msg,
                      created_at, updated_at
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
                      enabled, position, last_invalidated_at, last_error_at, last_error_msg,
                      created_at, updated_at
               FROM tagging_services
               WHERE id = $1 AND owner_id = $2"#,
            service_id,
            owner_id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Return the id of the user's first `shared_tag_mapping` service (oldest), if any.
    /// Used by ShareBack auto-accept to attach the new mapping to an existing service.
    #[tracing::instrument(skip(ex), fields(owner_id = %owner_id))]
    pub async fn first_mapping_service_for_owner<'e, E>(
        ex: E,
        owner_id: Uuid,
    ) -> Result<Option<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_scalar!(
            r#"SELECT id FROM tagging_services
               WHERE owner_id = $1 AND service_type = 'shared_tag_mapping'::service_type
               ORDER BY created_at
               LIMIT 1"#,
            owner_id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex, requires, excludes), fields(owner_id = %owner_id))]
    pub async fn create<'e, E>(
        ex: E,
        owner_id: Uuid,
        service_type: ServiceType,
        name: &str,
        requires: &[String],
        excludes: &[String],
    ) -> Result<TaggingService, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            TaggingService,
            r#"INSERT INTO tagging_services (owner_id, service_type, name, requires, excludes, position)
               VALUES ($1, $2, $5, $3::ltree[], $4::ltree[],
                       COALESCE((SELECT MAX(position) FROM tagging_services WHERE owner_id = $1), -1) + 1)
               RETURNING id, owner_id, name,
                         service_type as "service_type: ServiceType",
                         requires::text[] as "requires!", excludes::text[] as "excludes!",
                         enabled, position, last_invalidated_at, last_error_at, last_error_msg,
                         created_at, updated_at"#,
            owner_id,
            service_type as ServiceType,
            requires as &[String],
            excludes as &[String],
            name,
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Update optional fields; pass `None` to leave a field unchanged.
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
                         enabled, position, last_invalidated_at, last_error_at, last_error_msg,
                         created_at, updated_at"#,
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

    /// Bump `last_invalidated_at` on a specific service to NOW(), marking all pictures dirty.
    /// Called after any configuration change (rule/segment/mapping add or delete, enable/disable).
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

// ─── SharedTagMapping rules ────────────────────────────────────────────────────

pub struct SharedTagMappingRuleRepository;

impl SharedTagMappingRuleRepository {
    #[tracing::instrument(skip(ex, service_ids))]
    pub async fn list_for_services<'e, E>(
        ex: E,
        service_ids: &[Uuid],
    ) -> Result<Vec<SharedTagMappingRule>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if service_ids.is_empty() {
            return Ok(vec![]);
        }
        sqlx::query_as!(
            SharedTagMappingRule,
            r#"SELECT id, service_id, incoming_share_id,
                      assign_tag::text as "assign_tag!", is_broken
               FROM shared_tag_mapping_services
               WHERE service_id = ANY($1::uuid[])"#,
            service_ids as &[Uuid],
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex), fields(incoming_share_id = %incoming_share_id))]
    pub async fn create<'e, E>(
        ex: E,
        service_id: Uuid,
        incoming_share_id: Uuid,
        assign_tag: &str,
    ) -> Result<SharedTagMappingRule, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            SharedTagMappingRule,
            r#"INSERT INTO shared_tag_mapping_services (service_id, incoming_share_id, assign_tag)
               VALUES ($1, $2, $3::text::ltree)
               RETURNING id, service_id, incoming_share_id,
                         assign_tag::text as "assign_tag!", is_broken"#,
            service_id,
            incoming_share_id,
            assign_tag,
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Deletes a mapping rule. Verifies ownership via the parent tagging_service.
    #[tracing::instrument(skip(ex), fields(owner_id = %owner_id))]
    pub async fn delete<'e, E>(
        ex: E,
        owner_id: Uuid,
        service_id: Uuid,
        rule_id: Uuid,
    ) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let result = sqlx::query!(
            r#"DELETE FROM shared_tag_mapping_services stms
               USING tagging_services ts
               WHERE stms.id = $1
                 AND stms.service_id = $2
                 AND ts.id = $2
                 AND ts.owner_id = $3"#,
            rule_id,
            service_id,
            owner_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(result.rows_affected() > 0)
    }

    /// Flag every mapping rule referencing an incoming share as broken. Called when the share
    /// is revoked/tombstoned so the UI can surface the now-empty mapping.
    #[tracing::instrument(skip(ex), fields(incoming_share_id = %incoming_share_id))]
    pub async fn flag_broken_for_share<'e, E>(
        ex: E,
        incoming_share_id: Uuid,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query!(
            r#"UPDATE shared_tag_mapping_services SET is_broken = true
               WHERE incoming_share_id = $1"#,
            incoming_share_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }
}

// ─── Rule tagging rules ────────────────────────────────────────────────────────

pub struct RuleTaggingRuleRepository;

impl RuleTaggingRuleRepository {
    #[tracing::instrument(skip(ex, service_ids))]
    pub async fn list_for_services<'e, E>(
        ex: E,
        service_ids: &[Uuid],
    ) -> Result<Vec<RuleTaggingRule>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if service_ids.is_empty() {
            return Ok(vec![]);
        }
        sqlx::query_as!(
            RuleTaggingRule,
            r#"SELECT id, service_id, predicate as "predicate!: serde_json::Value",
                      assign_tag::text as "assign_tag!", position
               FROM rule_tagging_services
               WHERE service_id = ANY($1::uuid[])
               ORDER BY position, id"#,
            service_ids as &[Uuid],
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex, predicate))]
    pub async fn create<'e, E>(
        ex: E,
        service_id: Uuid,
        predicate: &serde_json::Value,
        assign_tag: &str,
    ) -> Result<RuleTaggingRule, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            RuleTaggingRule,
            r#"INSERT INTO rule_tagging_services (service_id, predicate, assign_tag, position)
               VALUES ($1, $2, $3::text::ltree,
                       COALESCE((SELECT MAX(position) FROM rule_tagging_services WHERE service_id = $1), -1) + 1)
               RETURNING id, service_id, predicate as "predicate!: serde_json::Value",
                         assign_tag::text as "assign_tag!", position"#,
            service_id,
            predicate,
            assign_tag,
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Update a rule's predicate and assigned tag. Verifies ownership via the parent service.
    /// Returns `None` if the rule does not exist under that owner's service.
    #[tracing::instrument(skip(ex, predicate), fields(owner_id = %owner_id))]
    pub async fn update<'e, E>(
        ex: E,
        owner_id: Uuid,
        service_id: Uuid,
        rule_id: Uuid,
        predicate: &serde_json::Value,
        assign_tag: &str,
    ) -> Result<Option<RuleTaggingRule>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            RuleTaggingRule,
            r#"UPDATE rule_tagging_services rts
               SET predicate = $4, assign_tag = $5::text::ltree
               FROM tagging_services ts
               WHERE rts.id = $1
                 AND rts.service_id = $2
                 AND ts.id = $2
                 AND ts.owner_id = $3
               RETURNING rts.id, rts.service_id,
                         rts.predicate as "predicate!: serde_json::Value",
                         rts.assign_tag::text as "assign_tag!", rts.position"#,
            rule_id,
            service_id,
            owner_id,
            predicate,
            assign_tag,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Reassign `position` for the given rules of one service. `ordered_ids` is the complete
    /// desired order — each rule gets `position = its index`. Errors if any id does not belong to
    /// the service / owner.
    #[tracing::instrument(skip(ex, ordered_ids), fields(owner_id = %owner_id))]
    pub async fn reorder<'e, E>(
        ex: E,
        owner_id: Uuid,
        service_id: Uuid,
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
            r#"UPDATE rule_tagging_services rts
               SET position = ord.pos
               FROM (SELECT unnest($1::uuid[]) AS id, unnest($2::int[]) AS pos) AS ord,
                    tagging_services ts
               WHERE rts.id = ord.id
                 AND rts.service_id = $3
                 AND ts.id = $3
                 AND ts.owner_id = $4
               RETURNING rts.id"#,
            ordered_ids as &[Uuid],
            &positions as &[i32],
            service_id,
            owner_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)?;

        if updated.len() != ordered_ids.len() {
            return Err(AppError::BadRequest(
                "one or more rule IDs not found or not owned by you".into(),
            ));
        }
        Ok(())
    }

    #[tracing::instrument(skip(ex), fields(owner_id = %owner_id))]
    pub async fn delete<'e, E>(
        ex: E,
        owner_id: Uuid,
        service_id: Uuid,
        rule_id: Uuid,
    ) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let result = sqlx::query!(
            r#"DELETE FROM rule_tagging_services rts
               USING tagging_services ts
               WHERE rts.id = $1
                 AND rts.service_id = $2
                 AND ts.id = $2
                 AND ts.owner_id = $3"#,
            rule_id,
            service_id,
            owner_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(result.rows_affected() > 0)
    }
}

// ─── Segmentation rules ────────────────────────────────────────────────────────

pub struct SegmentationRuleRepository;

impl SegmentationRuleRepository {
    #[tracing::instrument(skip(ex, service_ids))]
    pub async fn list_for_services<'e, E>(
        ex: E,
        service_ids: &[Uuid],
    ) -> Result<Vec<SegmentationRule>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if service_ids.is_empty() {
            return Ok(vec![]);
        }
        sqlx::query_as!(
            SegmentationRule,
            r#"SELECT id, service_id, name,
                      lower(date_range) AT TIME ZONE 'UTC' as "date_start!",
                      upper(date_range) AT TIME ZONE 'UTC' as "date_end!",
                      assign_tag::text as "assign_tag!",
                      parent_segment_id
               FROM segmentation_tagging_services
               WHERE service_id = ANY($1::uuid[])
               ORDER BY lower(date_range)"#,
            service_ids as &[Uuid],
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    pub async fn create<'e, E>(
        ex: E,
        service_id: Uuid,
        name: &str,
        date_start: NaiveDateTime,
        date_end: NaiveDateTime,
        assign_tag: &str,
        parent_segment_id: Option<Uuid>,
    ) -> Result<SegmentationRule, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            SegmentationRule,
            r#"INSERT INTO segmentation_tagging_services
                   (service_id, name, date_range, assign_tag, parent_segment_id)
               VALUES ($1, $2, tstzrange($3::timestamp AT TIME ZONE 'UTC', $4::timestamp AT TIME ZONE 'UTC', '[)'), $5::text::ltree, $6)
               RETURNING id, service_id, name,
                         lower(date_range) AT TIME ZONE 'UTC' as "date_start!",
                         upper(date_range) AT TIME ZONE 'UTC' as "date_end!",
                         assign_tag::text as "assign_tag!",
                         parent_segment_id"#,
            service_id,
            name,
            date_start,
            date_end,
            assign_tag,
            parent_segment_id as Option<Uuid>,
        )
            .fetch_one(ex)
            .await
            .map_err(map_sqlx_error)
    }

    pub async fn delete<'e, E>(
        ex: E,
        owner_id: Uuid,
        service_id: Uuid,
        segment_id: Uuid,
    ) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let result = sqlx::query!(
            r#"DELETE FROM segmentation_tagging_services sts
               USING tagging_services ts
               WHERE sts.id = $1
                 AND sts.service_id = $2
                 AND ts.id = $2
                 AND ts.owner_id = $3"#,
            segment_id,
            service_id,
            owner_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(result.rows_affected() > 0)
    }
}
