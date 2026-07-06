use crate::domain::job::{Job, JobConfig, JobStatus, JobType};
use crate::infra::observability;
use archypix_common::error::{map_sqlx_error, AppError};
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;

pub struct JobRepository;

impl JobRepository {
    /// Atomically claim the next pending job matching any of `job_types`.
    ///
    /// Generates a fresh `claim_token` UUID for this claim; the worker must echo
    /// it back in `complete` / `fail` to prevent stale workers from corrupting
    /// re-claimed jobs.
    #[tracing::instrument(skip(db, job_types))]
    pub async fn claim_next(
        db: &PgPool,
        worker_id: &str,
        job_types: &[JobType],
    ) -> Result<Option<Job>, AppError> {
        let mut tx = db.begin().await.map_err(map_sqlx_error)?;

        let job_id: Option<Uuid> = if job_types.is_empty() {
            sqlx::query_scalar!(
                "SELECT id FROM jobs WHERE status = 'pending' \
                 ORDER BY created_at LIMIT 1 FOR UPDATE SKIP LOCKED"
            )
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx_error)?
        } else {
            let type_strs: Vec<String> = job_types.iter().map(|t| t.to_string()).collect();
            sqlx::query_scalar!(
                "SELECT id FROM jobs WHERE status = 'pending' \
                 AND job_type::text = ANY($1) \
                 ORDER BY created_at LIMIT 1 FOR UPDATE SKIP LOCKED",
                &type_strs as &[String],
            )
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx_error)?
        };

        let Some(job_id) = job_id else {
            tx.rollback().await.ok();
            return Ok(None);
        };

        let claim_token = Uuid::new_v4();

        let job = sqlx::query_as!(
            Job,
            r#"UPDATE jobs
               SET status      = 'processing',
                   started_at  = (now() AT TIME ZONE 'utc'),
                   claimed_by  = $2,
                   claim_token = $3
               WHERE id = $1
               RETURNING
                   id, owner_id,
                   job_type    AS "job_type: JobType",
                   status      AS "status: JobStatus",
                   config      AS "config: _",
                   result      AS "result: _",
                   error_message,
                   retry_count, max_retries,
                   idempotency_key,
                   picture_id, claimed_by, claim_token,
                   trace_context AS "trace_context: _",
                   created_at, started_at, completed_at"#,
            job_id,
            worker_id,
            claim_token,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(Some(job))
    }

    /// Mark a job as completed and store the result JSON.
    ///
    /// Returns `None` when the job is not in `processing` state or the
    /// `claim_token` does not match — this prevents stale workers (reset by the
    /// watchdog) from overwriting results of a re-claimed job.
    #[tracing::instrument(skip(ex, result), fields(job_id = %job_id))]
    pub async fn complete<'e, E>(
        ex: E,
        job_id: Uuid,
        claim_token: Uuid,
        result: serde_json::Value,
    ) -> Result<Option<Job>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            Job,
            r#"UPDATE jobs
               SET status       = 'completed',
                   completed_at = (now() AT TIME ZONE 'utc'),
                   result       = $3
               WHERE id         = $1
                 AND claim_token = $2
                 AND status     = 'processing'
               RETURNING
                   id, owner_id,
                   job_type    AS "job_type: JobType",
                   status      AS "status: JobStatus",
                   config      AS "config: _",
                   result      AS "result: _",
                   error_message,
                   retry_count, max_retries,
                   idempotency_key,
                   picture_id, claimed_by, claim_token,
                   trace_context AS "trace_context: _",
                   created_at, started_at, completed_at"#,
            job_id,
            claim_token,
            result as serde_json::Value,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Mark a job as failed.
    ///
    /// Returns `None` when the `claim_token` does not match or the job is no
    /// longer in `processing` state (same guard as `complete`).
    ///
    /// When `permanent` is `true`, the job transitions directly to `failed`
    /// regardless of remaining retries.  When `false`, the retry counter is
    /// checked: if retries remain the job resets to `pending`.
    #[tracing::instrument(skip(ex), fields(job_id = %job_id))]
    pub async fn fail<'e, E>(
        ex: E,
        job_id: Uuid,
        claim_token: Uuid,
        error: &str,
        permanent: bool,
    ) -> Result<Option<Job>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            Job,
            r#"UPDATE jobs
               SET status        = CASE
                                       WHEN $4 OR retry_count + 1 >= max_retries
                                           THEN 'failed'::job_status
                                       ELSE 'pending'::job_status
                                   END,
                   retry_count   = retry_count + 1,
                   error_message = $3,
                   started_at    = CASE
                                       WHEN $4 OR retry_count + 1 >= max_retries THEN started_at
                                       ELSE NULL
                                   END,
                   claimed_by    = NULL,
                   claim_token   = NULL,
                   completed_at  = CASE
                                       WHEN $4 OR retry_count + 1 >= max_retries
                                           THEN (now() AT TIME ZONE 'utc')
                                       ELSE NULL
                                   END
               WHERE id          = $1
                 AND claim_token = $2
                 AND status      = 'processing'
               RETURNING
                   id, owner_id,
                   job_type    AS "job_type: JobType",
                   status      AS "status: JobStatus",
                   config      AS "config: _",
                   result      AS "result: _",
                   error_message,
                   retry_count, max_retries,
                   idempotency_key,
                   picture_id, claimed_by, claim_token,
                   trace_context AS "trace_context: _",
                   created_at, started_at, completed_at"#,
            job_id,
            claim_token,
            error,
            permanent,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Enqueue a new job. Captures the current OTel trace context so the worker can link back.
    ///
    /// `job_type` is derived from `config` via `JobConfig::job_type()` so the
    /// DB column and the JSONB discriminant can never disagree.
    /// Idempotency conflict returns `AppError::Conflict`.
    #[tracing::instrument(skip(ex, config), fields(owner_id = %owner_id))]
    pub async fn create<'e, E>(
        ex: E,
        owner_id: Uuid,
        picture_id: Option<Uuid>,
        config: &JobConfig,
        idempotency_key: Option<&str>,
    ) -> Result<Job, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let job_type = config.job_type();
        let config_value = serde_json::to_value(config)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;

        let ctx_map = observability::inject_context();
        let trace_context: Option<serde_json::Value> = if ctx_map.is_empty() {
            None
        } else {
            serde_json::to_value(&ctx_map).ok()
        };

        sqlx::query_as!(
            Job,
            r#"INSERT INTO jobs (owner_id, job_type, picture_id, config, idempotency_key, trace_context)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING
                   id, owner_id,
                   job_type    AS "job_type: JobType",
                   status      AS "status: JobStatus",
                   config      AS "config: _",
                   result      AS "result: _",
                   error_message,
                   retry_count, max_retries,
                   idempotency_key,
                   picture_id, claimed_by, claim_token,
                   trace_context AS "trace_context: _",
                   created_at, started_at, completed_at"#,
            owner_id,
            job_type as JobType,
            picture_id,
            config_value as serde_json::Value,
            idempotency_key,
            trace_context as Option<serde_json::Value>,
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Find the in-flight (`pending` / `processing`) `edit_picture` job for a picture, if any.
    /// At most one can exist (enforced by `uq_edit_picture_inflight`). Drives the §5 concurrency
    /// rule: fold into a `pending` job, or defer enqueue past a `processing` one.
    #[tracing::instrument(skip(ex), fields(picture_id = %picture_id))]
    pub async fn find_inflight_edit<'e, E>(ex: E, picture_id: Uuid) -> Result<Option<Job>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            Job,
            r#"SELECT id, owner_id,
                      job_type    AS "job_type: JobType",
                      status      AS "status: JobStatus",
                      config      AS "config: _",
                      result      AS "result: _",
                      error_message,
                      retry_count, max_retries,
                      idempotency_key,
                      picture_id, claimed_by, claim_token,
                      trace_context AS "trace_context: _",
                      created_at, started_at, completed_at
               FROM   jobs
               WHERE  picture_id = $1
                 AND  job_type = 'edit_picture'
                 AND  status IN ('pending', 'processing')
               LIMIT 1"#,
            picture_id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Replace the `config` JSONB of a still-`pending` job (the fold path). Only updates while the
    /// job is `pending` so a job that started processing mid-fold is not silently mutated.
    #[tracing::instrument(skip(ex, config), fields(job_id = %job_id))]
    pub async fn update_config_if_pending<'e, E>(
        ex: E,
        job_id: Uuid,
        config: &JobConfig,
    ) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let config_value = serde_json::to_value(config)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        let res = sqlx::query!(
            "UPDATE jobs SET config = $2 WHERE id = $1 AND status = 'pending'",
            job_id,
            config_value as serde_json::Value,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected() > 0)
    }

    #[tracing::instrument(skip(ex), fields(job_id = %id))]
    pub async fn find_by_id<'e, E>(ex: E, id: Uuid) -> Result<Option<Job>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            Job,
            r#"SELECT id, owner_id,
                      job_type    AS "job_type: JobType",
                      status      AS "status: JobStatus",
                      config      AS "config: _",
                      result      AS "result: _",
                      error_message,
                      retry_count, max_retries,
                      idempotency_key,
                      picture_id, claimed_by, claim_token,
                      trace_context AS "trace_context: _",
                      created_at, started_at, completed_at
               FROM   jobs
               WHERE  id = $1"#,
            id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(db), fields(picture_id = %picture_id, owner_id = %owner_id))]
    pub async fn list_by_picture(
        db: &PgPool,
        picture_id: Uuid,
        owner_id: Uuid,
    ) -> Result<Vec<Job>, AppError> {
        sqlx::query_as!(
            Job,
            r#"SELECT id, owner_id,
                      job_type    AS "job_type: JobType",
                      status      AS "status: JobStatus",
                      config      AS "config: _",
                      result      AS "result: _",
                      error_message,
                      retry_count, max_retries,
                      idempotency_key,
                      picture_id, claimed_by, claim_token,
                      trace_context AS "trace_context: _",
                      created_at, started_at, completed_at
               FROM   jobs
               WHERE  picture_id = $1
                 AND  owner_id   = $2
               ORDER BY created_at DESC"#,
            picture_id,
            owner_id,
        )
        .fetch_all(db)
        .await
        .map_err(map_sqlx_error)
    }

    /// Reset jobs stuck in `processing` for longer than `timeout_secs`.
    ///
    /// Clears `claimed_by` and `claim_token` so a fresh worker gets a new token
    /// when it re-claims the job — preventing the original (late) worker from
    /// completing the retried run.
    #[tracing::instrument(skip(db))]
    pub async fn reset_stale(db: &PgPool, timeout_secs: i64) -> Result<u64, AppError> {
        let result = sqlx::query!(
            r#"UPDATE jobs
               SET status        = CASE
                                       WHEN retry_count + 1 < max_retries THEN 'pending'::job_status
                                       ELSE 'failed'::job_status
                                   END,
                   retry_count   = retry_count + 1,
                   error_message = 'Worker timed out without reporting a result',
                   claimed_by    = NULL,
                   claim_token   = NULL,
                   started_at    = CASE
                                       WHEN retry_count + 1 < max_retries THEN NULL
                                       ELSE started_at
                                   END,
                   completed_at  = CASE
                                       WHEN retry_count + 1 < max_retries THEN NULL
                                       ELSE (now() AT TIME ZONE 'utc')
                                   END
               WHERE status     = 'processing'
                 AND started_at < (now() AT TIME ZONE 'utc') - ($1 * INTERVAL '1 second')"#,
            timeout_secs as f64,
        )
        .execute(db)
        .await
        .map_err(map_sqlx_error)?;
        Ok(result.rows_affected())
    }

    /// Delete terminal jobs (`completed` / `failed`) whose `completed_at` is older than
    /// `retention_secs`. Never touches `pending` / `processing`. Returns rows deleted.
    ///
    /// Both `completed` and permanently-`failed` jobs set `completed_at` (see [`Self::fail`] /
    /// [`Self::reset_stale`]), so it is the correct retention anchor; rows with a NULL
    /// `completed_at` fall back to `created_at`.
    #[tracing::instrument(skip(db))]
    pub async fn delete_terminal_older_than(
        db: &PgPool,
        retention_secs: i64,
    ) -> Result<u64, AppError> {
        let res = sqlx::query!(
            r#"DELETE FROM jobs
               WHERE status IN ('completed', 'failed')
                 AND COALESCE(completed_at, created_at)
                     < (now() AT TIME ZONE 'utc') - make_interval(secs => $1)"#,
            retention_secs as f64
        )
        .execute(db)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::job::{JobConfig, JobStatus, JobType};
    use archypix_common::job::GenThumbnailConfig;
    use sqlx::PgPool;
    use uuid::Uuid;

    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

    async fn seed_user(db: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO users (id, username, email, display_name) VALUES ($1, $2, $3, $4)",
            id,
            format!("testuser_{}", id.to_string().split('-').next().unwrap()),
            format!("{}@test.com", id),
            "Test User",
        )
        .execute(db)
        .await
        .unwrap();
        id
    }

    async fn seed_job(db: &PgPool, owner_id: Uuid) -> Job {
        let config = JobConfig::GenThumbnail(GenThumbnailConfig {
            picture_id: Uuid::new_v4(),
            is_initial: true,
        });
        JobRepository::create(db, owner_id, None, &config, None)
            .await
            .unwrap()
    }

    // ── claim_next ────────────────────────────────────────────────────────────

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn claim_next_returns_none_when_empty(db: PgPool) {
        let result = JobRepository::claim_next(&db, "worker1", &[JobType::GenThumbnail])
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn claim_next_marks_job_processing(db: PgPool) {
        let owner = seed_user(&db).await;
        let created = seed_job(&db, owner).await;

        let claimed = JobRepository::claim_next(&db, "worker1", &[])
            .await
            .unwrap()
            .expect("should find a job");

        assert_eq!(claimed.id, created.id);
        assert_eq!(claimed.status, JobStatus::Processing);
        assert!(claimed.claim_token.is_some());
        assert_eq!(claimed.claimed_by.as_deref(), Some("worker1"));
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn claim_next_respects_job_type_filter(db: PgPool) {
        let owner = seed_user(&db).await;
        seed_job(&db, owner).await; // gen_thumbnail

        // Claim only edit_picture — should find nothing
        let result = JobRepository::claim_next(&db, "worker1", &[JobType::EditPicture])
            .await
            .unwrap();
        assert!(result.is_none());

        // Claim any — should find the gen_thumbnail
        let result = JobRepository::claim_next(&db, "worker1", &[JobType::GenThumbnail])
            .await
            .unwrap();
        assert!(result.is_some());
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn claimed_job_is_not_double_claimed(db: PgPool) {
        let owner = seed_user(&db).await;
        seed_job(&db, owner).await;

        JobRepository::claim_next(&db, "worker1", &[])
            .await
            .unwrap();

        // Second claim should find nothing (job is now processing)
        let second = JobRepository::claim_next(&db, "worker2", &[])
            .await
            .unwrap();
        assert!(second.is_none());
    }

    // ── complete ──────────────────────────────────────────────────────────────

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn complete_with_correct_token_marks_completed(db: PgPool) {
        let owner = seed_user(&db).await;
        seed_job(&db, owner).await;

        let claimed = JobRepository::claim_next(&db, "worker1", &[])
            .await
            .unwrap()
            .unwrap();
        let token = claimed.claim_token.unwrap();

        let completed = JobRepository::complete(&db, claimed.id, token, serde_json::json!({}))
            .await
            .unwrap();
        assert!(completed.is_some());
        assert_eq!(completed.unwrap().status, JobStatus::Completed);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn complete_with_wrong_token_returns_none(db: PgPool) {
        let owner = seed_user(&db).await;
        seed_job(&db, owner).await;

        let claimed = JobRepository::claim_next(&db, "worker1", &[])
            .await
            .unwrap()
            .unwrap();

        let wrong_token = Uuid::new_v4();
        let result = JobRepository::complete(&db, claimed.id, wrong_token, serde_json::json!({}))
            .await
            .unwrap();
        assert!(result.is_none(), "wrong claim_token must be rejected");

        // Job must still be in processing state
        let job = JobRepository::find_by_id(&db, claimed.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.status, JobStatus::Processing);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn complete_on_already_completed_job_is_rejected(db: PgPool) {
        let owner = seed_user(&db).await;
        seed_job(&db, owner).await;

        let claimed = JobRepository::claim_next(&db, "worker1", &[])
            .await
            .unwrap()
            .unwrap();
        let token = claimed.claim_token.unwrap();

        // First completion
        JobRepository::complete(&db, claimed.id, token, serde_json::json!({}))
            .await
            .unwrap();

        // Second completion with same token — status is no longer 'processing'
        let result = JobRepository::complete(&db, claimed.id, token, serde_json::json!({}))
            .await
            .unwrap();
        assert!(result.is_none());
    }

    // ── fail ──────────────────────────────────────────────────────────────────

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn fail_with_correct_token_and_retries_resets_to_pending(db: PgPool) {
        let owner = seed_user(&db).await;
        seed_job(&db, owner).await; // default max_retries = 3

        let claimed = JobRepository::claim_next(&db, "worker1", &[])
            .await
            .unwrap()
            .unwrap();
        let token = claimed.claim_token.unwrap();

        let failed = JobRepository::fail(&db, claimed.id, token, "transient error", false)
            .await
            .unwrap();
        assert!(failed.is_some());

        let job = JobRepository::find_by_id(&db, claimed.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.status, JobStatus::Pending, "should reset to pending");
        assert_eq!(job.retry_count, 1);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn fail_permanent_skips_retry(db: PgPool) {
        let owner = seed_user(&db).await;
        seed_job(&db, owner).await;

        let claimed = JobRepository::claim_next(&db, "worker1", &[])
            .await
            .unwrap()
            .unwrap();
        let token = claimed.claim_token.unwrap();

        JobRepository::fail(&db, claimed.id, token, "permanent error", true)
            .await
            .unwrap();

        let job = JobRepository::find_by_id(&db, claimed.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.status, JobStatus::Failed);
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn fail_with_wrong_token_is_rejected(db: PgPool) {
        let owner = seed_user(&db).await;
        seed_job(&db, owner).await;

        let claimed = JobRepository::claim_next(&db, "worker1", &[])
            .await
            .unwrap()
            .unwrap();

        let result = JobRepository::fail(&db, claimed.id, Uuid::new_v4(), "error", false)
            .await
            .unwrap();
        assert!(result.is_none());
    }

    // ── idempotency key ───────────────────────────────────────────────────────

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn duplicate_idempotency_key_returns_conflict(db: PgPool) {
        let owner = seed_user(&db).await;
        let config = JobConfig::GenThumbnail(GenThumbnailConfig {
            picture_id: Uuid::new_v4(),
            is_initial: true,
        });
        JobRepository::create(&db, owner, None, &config, Some("unique-key"))
            .await
            .unwrap();

        let result = JobRepository::create(&db, owner, None, &config, Some("unique-key")).await;
        assert!(
            matches!(result, Err(AppError::Conflict(_))),
            "second insert with same idempotency key should conflict"
        );
    }
}
