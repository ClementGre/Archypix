use crate::domain::job::{JobStatus, JobType};
use crate::domain::share::ShareStatus;
use crate::infra::error::{AppError, map_sqlx_error};
use chrono::NaiveDateTime;
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

pub struct AdminRepository;

// ── Raw query result types ────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct JobCountRow {
    status: JobStatus,
    count: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct ShareStatusCount {
    status: ShareStatus,
    count: i64,
}

/// Minimal job projection for admin views (no config/result JSONB).
#[derive(Debug, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct AdminJob {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub owner_username: String,
    pub job_type: JobType,
    pub status: JobStatus,
    pub retry_count: i32,
    pub max_retries: i32,
    pub error_message: Option<String>,
    pub picture_id: Option<Uuid>,
    pub claimed_by: Option<String>,
    pub created_at: NaiveDateTime,
    pub started_at: Option<NaiveDateTime>,
    pub completed_at: Option<NaiveDateTime>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct InstanceStats {
    pub user_count: i64,
    pub owned_picture_count: i64,
    pub received_picture_count: i64,
    pub total_storage_bytes: i64,
    pub job_counts: HashMap<String, i64>,
    pub errored_share_count: i64,
    pub pending_first_announcement_count: i64,
    pub dirty_picture_count: i64,
    pub last_worker_activity_at: Option<NaiveDateTime>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct UserStats {
    pub owned_picture_count: i64,
    pub received_picture_count: i64,
    /// Billed total (maintained counter, feature 22).
    pub storage_bytes: i64,
    pub quota_bytes: Option<i64>,
    pub originals_bytes: i64,
    pub originals_trashed_bytes: i64,
    pub versions_bytes: i64,
    pub versions_trashed_bytes: i64,
    pub usage_ratio: Option<f64>,
    pub job_counts: HashMap<String, i64>,
    pub outgoing_share_counts: HashMap<String, i64>,
    pub incoming_share_counts: HashMap<String, i64>,
    pub dirty_picture_count: i64,
    pub errored_share_count: i64,
}

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct UserWithStorage {
    pub id: Uuid,
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub is_admin: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    /// Billed total (maintained counter, feature 22) — originals + versions, live + trashed.
    pub storage_bytes: i64,
    pub quota_bytes: Option<i64>,
    pub originals_bytes: i64,
    pub originals_trashed_bytes: i64,
    pub versions_bytes: i64,
    pub versions_trashed_bytes: i64,
}

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct ErroredShare {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub owner_username: String,
    pub tag_path: String,
    pub recipient_username: String,
    pub recipient_instance: String,
    pub next_retry_at: Option<NaiveDateTime>,
    pub last_error_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, serde::Serialize)]
pub struct FederationInstance {
    pub instance: String,
    pub outgoing_share_count: i64,
    pub incoming_share_count: i64,
    pub errored_share_count: i64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ConsistencyStats {
    /// Owned pictures with exif_sync_status = 'pending' but no active edit_picture job.
    pub stuck_exif_pending_count: i64,
    /// Owned, non-deleted pictures with no thumbnails, ingested more than 30 minutes ago.
    pub pictures_without_thumbnail_count: i64,
    /// SharedTagMappingService rows flagged as broken (their IncomingShare was revoked).
    pub broken_mapping_count: i64,
}

fn counts_to_map(rows: Vec<impl StatusRow>) -> HashMap<String, i64> {
    rows.into_iter()
        .map(|r| (r.status_key(), r.count_val()))
        .collect()
}

trait StatusRow {
    fn status_key(&self) -> String;
    fn count_val(&self) -> i64;
}

impl StatusRow for JobCountRow {
    fn status_key(&self) -> String {
        serde_json::to_value(&self.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    }
    fn count_val(&self) -> i64 {
        self.count
    }
}

impl StatusRow for ShareStatusCount {
    fn status_key(&self) -> String {
        serde_json::to_value(&self.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    }
    fn count_val(&self) -> i64 {
        self.count
    }
}

impl AdminRepository {
    // ── Instance-wide stats ───────────────────────────────────────────────────

    #[tracing::instrument(skip(db))]
    pub async fn instance_stats(db: &PgPool) -> Result<InstanceStats, AppError> {
        let user_count: i64 = sqlx::query_scalar!("SELECT COUNT(*) AS \"count!\" FROM users")
            .fetch_one(db)
            .await
            .map_err(map_sqlx_error)?;

        let owned_picture_count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM pictures WHERE owner_username IS NULL AND deleted_at IS NULL"
        )
            .fetch_one(db)
            .await
            .map_err(map_sqlx_error)?;

        let received_picture_count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM pictures WHERE owner_username IS NOT NULL AND deleted_at IS NULL"
        )
            .fetch_one(db)
            .await
            .map_err(map_sqlx_error)?;

        let total_storage_bytes: i64 = sqlx::query_scalar!(
            r#"SELECT COALESCE(SUM(originals_bytes + originals_trashed_bytes
                                   + versions_bytes + versions_trashed_bytes), 0)::BIGINT
                   AS "bytes!" FROM user_storage"#
        )
        .fetch_one(db)
        .await
        .map_err(map_sqlx_error)?;

        let job_count_rows = sqlx::query_as!(
            JobCountRow,
            r#"SELECT status AS "status: JobStatus", COUNT(*) AS "count!" FROM jobs GROUP BY status"#
        )
            .fetch_all(db)
            .await
            .map_err(map_sqlx_error)?;

        let errored_share_count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM outgoing_shares WHERE status = 'errored'::share_status"
        )
            .fetch_one(db)
            .await
            .map_err(map_sqlx_error)?;

        let pending_first_announcement_count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM outgoing_shares WHERE status = 'pending_first_announcement'::share_status"
        )
            .fetch_one(db)
            .await
            .map_err(map_sqlx_error)?;

        let dirty_picture_count: i64 = sqlx::query_scalar!(
            r#"SELECT COUNT(DISTINCT p.id) AS "count!"
               FROM pictures p
               WHERE p.deleted_at IS NULL
                 AND (
                   p.last_pipeline_run_at IS NULL
                   OR EXISTS (
                     SELECT 1 FROM tagging_services ts
                     WHERE ts.owner_id = p.local_user_id
                       AND ts.enabled = true
                       AND p.last_pipeline_run_at < ts.last_invalidated_at
                   )
                 )"#
        )
        .fetch_one(db)
        .await
        .map_err(map_sqlx_error)?;

        let last_worker_activity_at: Option<NaiveDateTime> =
            sqlx::query_scalar!("SELECT MAX(started_at) FROM jobs WHERE started_at IS NOT NULL")
                .fetch_one(db)
                .await
                .map_err(map_sqlx_error)?;

        Ok(InstanceStats {
            user_count,
            owned_picture_count,
            received_picture_count,
            total_storage_bytes,
            job_counts: counts_to_map(job_count_rows),
            errored_share_count,
            pending_first_announcement_count,
            dirty_picture_count,
            last_worker_activity_at,
        })
    }

    // ── Per-user stats ────────────────────────────────────────────────────────

    #[tracing::instrument(skip(db), fields(user_id = %user_id))]
    pub async fn user_stats(db: &PgPool, user_id: Uuid) -> Result<UserStats, AppError> {
        let owned_picture_count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM pictures \
             WHERE local_user_id = $1 AND owner_username IS NULL AND deleted_at IS NULL",
            user_id
        )
        .fetch_one(db)
        .await
        .map_err(map_sqlx_error)?;

        let received_picture_count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM pictures \
             WHERE local_user_id = $1 AND owner_username IS NOT NULL AND deleted_at IS NULL",
            user_id
        )
        .fetch_one(db)
        .await
        .map_err(map_sqlx_error)?;

        let storage =
            crate::repository::user_storage::UserStorageRepository::get(db, user_id).await?;
        let quota_bytes =
            crate::repository::user_storage::UserStorageRepository::get_quota(db, user_id).await?;
        let storage_bytes = storage.billed_total();
        let usage_ratio = quota_bytes
            .filter(|q| *q > 0)
            .map(|q| storage_bytes as f64 / q as f64);

        let job_count_rows = sqlx::query_as!(
            JobCountRow,
            r#"SELECT status AS "status: JobStatus", COUNT(*) AS "count!"
               FROM jobs WHERE owner_id = $1 GROUP BY status"#,
            user_id
        )
        .fetch_all(db)
        .await
        .map_err(map_sqlx_error)?;

        let outgoing_rows = sqlx::query_as!(
            ShareStatusCount,
            r#"SELECT status AS "status: ShareStatus", COUNT(*) AS "count!"
               FROM outgoing_shares WHERE owner_id = $1 GROUP BY status"#,
            user_id
        )
        .fetch_all(db)
        .await
        .map_err(map_sqlx_error)?;

        let incoming_rows = sqlx::query_as!(
            ShareStatusCount,
            r#"SELECT status AS "status: ShareStatus", COUNT(*) AS "count!"
               FROM incoming_shares WHERE recipient_id = $1 GROUP BY status"#,
            user_id
        )
        .fetch_all(db)
        .await
        .map_err(map_sqlx_error)?;

        let dirty_picture_count: i64 = sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!"
               FROM pictures p
               WHERE p.local_user_id = $1
                 AND p.deleted_at IS NULL
                 AND (
                   p.last_pipeline_run_at IS NULL
                   OR EXISTS (
                     SELECT 1 FROM tagging_services ts
                     WHERE ts.owner_id = $1
                       AND ts.enabled = true
                       AND p.last_pipeline_run_at < ts.last_invalidated_at
                   )
                 )"#,
            user_id
        )
        .fetch_one(db)
        .await
        .map_err(map_sqlx_error)?;

        let errored_share_count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" \
             FROM outgoing_shares WHERE owner_id = $1 AND status = 'errored'::share_status",
            user_id
        )
        .fetch_one(db)
        .await
        .map_err(map_sqlx_error)?;

        Ok(UserStats {
            owned_picture_count,
            received_picture_count,
            storage_bytes,
            quota_bytes,
            originals_bytes: storage.originals_bytes,
            originals_trashed_bytes: storage.originals_trashed_bytes,
            versions_bytes: storage.versions_bytes,
            versions_trashed_bytes: storage.versions_trashed_bytes,
            usage_ratio,
            job_counts: counts_to_map(job_count_rows),
            outgoing_share_counts: counts_to_map(outgoing_rows),
            incoming_share_counts: counts_to_map(incoming_rows),
            dirty_picture_count,
            errored_share_count,
        })
    }

    // ── User list with storage ────────────────────────────────────────────────

    #[tracing::instrument(skip(db))]
    pub async fn list_users_with_storage(db: &PgPool) -> Result<Vec<UserWithStorage>, AppError> {
        sqlx::query_as!(
            UserWithStorage,
            r#"SELECT u.id, u.username, u.email, u.display_name, u.is_admin,
                      u.created_at, u.updated_at, u.storage_quota_bytes AS quota_bytes,
                      COALESCE(us.originals_bytes, 0)         AS "originals_bytes!",
                      COALESCE(us.originals_trashed_bytes, 0) AS "originals_trashed_bytes!",
                      COALESCE(us.versions_bytes, 0)          AS "versions_bytes!",
                      COALESCE(us.versions_trashed_bytes, 0)  AS "versions_trashed_bytes!",
                      COALESCE(us.originals_bytes + us.originals_trashed_bytes
                               + us.versions_bytes + us.versions_trashed_bytes, 0)::BIGINT
                          AS "storage_bytes!"
               FROM users u
               LEFT JOIN user_storage us ON us.user_id = u.id
               ORDER BY u.created_at DESC"#
        )
        .fetch_all(db)
        .await
        .map_err(map_sqlx_error)
    }

    // ── Job list (dynamic filters) ────────────────────────────────────────────

    #[tracing::instrument(skip(db))]
    pub async fn list_jobs(
        db: &PgPool,
        status_filter: Option<JobStatus>,
        type_filter: Option<JobType>,
        user_id_filter: Option<Uuid>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AdminJob>, AppError> {
        let mut qb = sqlx::QueryBuilder::new(
            "SELECT j.id, j.owner_id, u.username AS owner_username, \
             j.job_type, j.status, j.retry_count, j.max_retries, \
             j.error_message, j.picture_id, j.claimed_by, \
             j.created_at, j.started_at, j.completed_at \
             FROM jobs j JOIN users u ON u.id = j.owner_id \
             WHERE 1=1",
        );

        if let Some(s) = status_filter {
            qb.push(" AND j.status = ");
            qb.push_bind(s);
        }
        if let Some(t) = type_filter {
            qb.push(" AND j.job_type = ");
            qb.push_bind(t);
        }
        if let Some(uid) = user_id_filter {
            qb.push(" AND j.owner_id = ");
            qb.push_bind(uid);
        }

        qb.push(" ORDER BY j.created_at DESC LIMIT ");
        qb.push_bind(limit);
        qb.push(" OFFSET ");
        qb.push_bind(offset);

        qb.build_query_as::<AdminJob>()
            .fetch_all(db)
            .await
            .map_err(map_sqlx_error)
    }

    // ── Stale jobs ────────────────────────────────────────────────────────────

    #[tracing::instrument(skip(db))]
    pub async fn list_stale_jobs(
        db: &PgPool,
        timeout_secs: i64,
    ) -> Result<Vec<AdminJob>, AppError> {
        sqlx::query_as!(
            AdminJob,
            r#"SELECT j.id, j.owner_id, u.username AS owner_username,
                      j.job_type AS "job_type: JobType",
                      j.status AS "status: JobStatus",
                      j.retry_count, j.max_retries, j.error_message,
                      j.picture_id, j.claimed_by, j.created_at, j.started_at, j.completed_at
               FROM jobs j
               JOIN users u ON u.id = j.owner_id
               WHERE j.status = 'processing'::job_status
                 AND j.started_at < (now() AT TIME ZONE 'utc') - ($1 * INTERVAL '1 second')
               ORDER BY j.started_at ASC"#,
            timeout_secs as f64,
        )
        .fetch_all(db)
        .await
        .map_err(map_sqlx_error)
    }

    // ── Job management (admin force ops) ─────────────────────────────────────

    /// Force-reset a job to pending regardless of current state. Clears claim info and retry count.
    /// Only applies to non-terminal jobs (pending/processing).
    #[tracing::instrument(skip(db), fields(job_id = %job_id))]
    pub async fn reset_job(db: &PgPool, job_id: Uuid) -> Result<Option<AdminJob>, AppError> {
        sqlx::query_as!(
            AdminJob,
            r#"WITH updated AS (
                 UPDATE jobs
                 SET status      = 'pending'::job_status,
                     claimed_by  = NULL,
                     claim_token = NULL,
                     started_at  = NULL,
                     retry_count = 0,
                     error_message = 'Reset by admin'
                 WHERE id = $1
                   AND status NOT IN ('completed'::job_status)
                 RETURNING *
               )
               SELECT u.id, u.owner_id, usr.username AS owner_username,
                      u.job_type AS "job_type: JobType",
                      u.status AS "status: JobStatus",
                      u.retry_count, u.max_retries, u.error_message,
                      u.picture_id, u.claimed_by, u.created_at, u.started_at, u.completed_at
               FROM updated u
               JOIN users usr ON usr.id = u.owner_id"#,
            job_id,
        )
        .fetch_optional(db)
        .await
        .map_err(map_sqlx_error)
    }

    /// Permanently fail a job (admin cancel). Applies to any non-terminal job.
    #[tracing::instrument(skip(db), fields(job_id = %job_id))]
    pub async fn cancel_job(db: &PgPool, job_id: Uuid) -> Result<Option<AdminJob>, AppError> {
        sqlx::query_as!(
            AdminJob,
            r#"WITH updated AS (
                 UPDATE jobs
                 SET status       = 'failed'::job_status,
                     completed_at = (now() AT TIME ZONE 'utc'),
                     claimed_by   = NULL,
                     claim_token  = NULL,
                     error_message = 'Cancelled by admin'
                 WHERE id = $1
                   AND status NOT IN ('completed'::job_status, 'failed'::job_status)
                 RETURNING *
               )
               SELECT u.id, u.owner_id, usr.username AS owner_username,
                      u.job_type AS "job_type: JobType",
                      u.status AS "status: JobStatus",
                      u.retry_count, u.max_retries, u.error_message,
                      u.picture_id, u.claimed_by, u.created_at, u.started_at, u.completed_at
               FROM updated u
               JOIN users usr ON usr.id = u.owner_id"#,
            job_id,
        )
        .fetch_optional(db)
        .await
        .map_err(map_sqlx_error)
    }

    // ── Errored shares (global) ───────────────────────────────────────────────

    #[tracing::instrument(skip(db))]
    pub async fn list_errored_shares(db: &PgPool) -> Result<Vec<ErroredShare>, AppError> {
        sqlx::query_as!(
            ErroredShare,
            r#"SELECT os.id, os.owner_id, u.username AS owner_username,
                      os.tag_path::text AS "tag_path!",
                      os.recipient_username, os.recipient_instance,
                      os.next_retry_at, os.last_error_at, os.created_at
               FROM outgoing_shares os
               JOIN users u ON u.id = os.owner_id
               WHERE os.status = 'errored'::share_status
               ORDER BY os.last_error_at DESC NULLS LAST"#
        )
        .fetch_all(db)
        .await
        .map_err(map_sqlx_error)
    }

    // ── Federation instances ──────────────────────────────────────────────────

    #[tracing::instrument(skip(db))]
    pub async fn list_federation_instances(
        db: &PgPool,
    ) -> Result<Vec<FederationInstance>, AppError> {
        // Aggregate outgoing shares per instance. No global_domain filter here: multiple backends
        // can share the same global_domain, so cross-backend shares would be wrongly excluded.
        let outgoing = sqlx::query!(
            r#"SELECT recipient_instance AS "instance!",
                      COUNT(*) AS "total!",
                      COUNT(*) FILTER (WHERE status = 'errored'::share_status) AS "errored!"
               FROM outgoing_shares
               GROUP BY recipient_instance"#,
        )
        .fetch_all(db)
        .await
        .map_err(map_sqlx_error)?;

        // Aggregate incoming shares per instance
        let incoming = sqlx::query!(
            r#"SELECT sender_instance AS "instance!",
                      COUNT(*) AS "total!"
               FROM incoming_shares
               GROUP BY sender_instance"#,
        )
        .fetch_all(db)
        .await
        .map_err(map_sqlx_error)?;

        // Merge in Rust
        let mut map: HashMap<String, FederationInstance> = HashMap::new();

        for row in outgoing {
            map.insert(
                row.instance.clone(),
                FederationInstance {
                    instance: row.instance,
                    outgoing_share_count: row.total,
                    incoming_share_count: 0,
                    errored_share_count: row.errored,
                },
            );
        }
        for row in incoming {
            map.entry(row.instance.clone())
                .and_modify(|e| e.incoming_share_count = row.total)
                .or_insert(FederationInstance {
                    instance: row.instance,
                    outgoing_share_count: 0,
                    incoming_share_count: row.total,
                    errored_share_count: 0,
                });
        }

        let mut instances: Vec<FederationInstance> = map.into_values().collect();
        instances.sort_by(|a, b| {
            let a_total = a.outgoing_share_count + a.incoming_share_count;
            let b_total = b.outgoing_share_count + b.incoming_share_count;
            b_total.cmp(&a_total)
        });
        Ok(instances)
    }

    // ── Consistency check ─────────────────────────────────────────────────────

    #[tracing::instrument(skip(db))]
    pub async fn consistency_stats(db: &PgPool) -> Result<ConsistencyStats, AppError> {
        let stuck_exif_pending_count: i64 = sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!"
               FROM pictures p
               WHERE p.exif_sync_status = 'pending'::picture_exif_sync_status
                 AND p.owner_username IS NULL
                 AND NOT EXISTS (
                   SELECT 1 FROM jobs j
                   WHERE j.picture_id = p.id
                     AND j.job_type = 'edit_picture'::job_type
                     AND j.status IN ('pending'::job_status, 'processing'::job_status)
                 )"#
        )
        .fetch_one(db)
        .await
        .map_err(map_sqlx_error)?;

        let pictures_without_thumbnail_count: i64 = sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!"
               FROM pictures p
               WHERE p.owner_username IS NULL
                 AND p.deleted_at IS NULL
                 AND p.thumbnails_generated_at IS NULL
                 AND p.ingested_at < (now() AT TIME ZONE 'utc') - INTERVAL '30 minutes'"#
        )
        .fetch_one(db)
        .await
        .map_err(map_sqlx_error)?;

        // Brokenness is derived (feature 20 §10.1): a shared_tag_mapping service is broken iff its
        // referenced incoming share is absent or not active.
        let broken_mapping_count: i64 = sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!"
               FROM tagging_services ts
               LEFT JOIN incoming_shares i ON (ts.config->>'incoming_share_id')::uuid = i.id
               WHERE ts.service_type = 'shared_tag_mapping'::service_type
                 AND (i.id IS NULL OR i.status <> 'active'::share_status)"#
        )
        .fetch_one(db)
        .await
        .map_err(map_sqlx_error)?;

        Ok(ConsistencyStats {
            stuck_exif_pending_count,
            pictures_without_thumbnail_count,
            broken_mapping_count,
        })
    }

    // ── Force-reconcile: clear share backoff ──────────────────────────────────

    /// Clear `next_retry_at` for an errored/pending_first_announcement share so the pipeline
    /// picks it up immediately on next wake. Returns the owner_id if the share was found and updated.
    #[tracing::instrument(skip(db), fields(share_id = %share_id))]
    pub async fn clear_share_backoff(
        db: &PgPool,
        share_id: Uuid,
    ) -> Result<Option<Uuid>, AppError> {
        sqlx::query_scalar!(
            r#"UPDATE outgoing_shares
               SET next_retry_at = NULL
               WHERE id = $1
                 AND status IN (
                   'errored'::share_status,
                   'pending_first_announcement'::share_status
                 )
               RETURNING owner_id"#,
            share_id,
        )
        .fetch_optional(db)
        .await
        .map_err(map_sqlx_error)
    }
}
