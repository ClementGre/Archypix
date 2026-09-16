use crate::domain::job::{Job, JobConfig, JobStatus, JobType};
use crate::infra::observability;
use archypix_common::error::{AppError, map_sqlx_error};
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;

/// One job the watchdog took off a dead worker, with the status it landed in (`pending` when
/// retries remain, `failed` when the budget is exhausted).
pub struct StaleReset {
    pub job_id: Uuid,
    pub picture_id: Option<Uuid>,
    pub job_type: JobType,
    pub status: JobStatus,
    /// The job's raw config, so a caller can tell *what* the reset job was doing without a second
    /// query (e.g. whether a `gen_thumbnail` was an extraction).
    pub config: serde_json::Value,
}

/// Outcome of [`JobRepository::create_idempotent`]: the live job for the key, and whether this call
/// is the one that enqueued it (`false` means an equivalent job was already in flight).
pub struct IdempotentJob {
    pub job: Job,
    pub created: bool,
}

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
    /// `job_type` is derived from `config` via `JobConfig::job_type()` so the DB column and the
    /// JSONB discriminant can never disagree.
    #[tracing::instrument(skip(ex, config), fields(owner_id = %owner_id))]
    pub async fn create<'e, E>(
        ex: E,
        owner_id: Uuid,
        picture_id: Option<Uuid>,
        config: &JobConfig,
    ) -> Result<Job, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        Self::insert(ex, owner_id, picture_id, config, None)
            .await
            .map(|c| c.job)
    }

    /// Enqueue a job under an idempotency key: at most one **live** (`pending` / `processing`) job
    /// per `(owner_id, idempotency_key)`, enforced by `uq_jobs_idempotency_live`. A call that loses
    /// the race returns the job already in flight rather than an error, so a retry is a no-op.
    ///
    /// The key is scoped to liveness, not to all time: once the job reaches a terminal status the
    /// key is free again, so it says "don't duplicate work that is still queued", never "this work
    /// may only ever happen once". Encode in the key whatever makes two enqueues *the same work*
    /// (e.g. the file hash an extraction will read) — owner scoping is the index's job, not the
    /// key's.
    #[tracing::instrument(skip(ex, config), fields(owner_id = %owner_id))]
    pub async fn create_idempotent<'e, E>(
        ex: E,
        owner_id: Uuid,
        picture_id: Option<Uuid>,
        config: &JobConfig,
        idempotency_key: &str,
    ) -> Result<IdempotentJob, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        Self::insert(ex, owner_id, picture_id, config, Some(idempotency_key)).await
    }

    async fn insert<'e, E>(
        ex: E,
        owner_id: Uuid,
        picture_id: Option<Uuid>,
        config: &JobConfig,
        idempotency_key: Option<&str>,
    ) -> Result<IdempotentJob, AppError>
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

        // The id is minted here rather than by the column default so the returned row identifies
        // which branch ran. `DO UPDATE` on a no-op assignment rather than `DO NOTHING`: it makes
        // `RETURNING` yield the conflicting row, which `DO NOTHING` plus a follow-up `SELECT`
        // cannot do race-free (the select runs on a snapshot taken before the winner committed).
        let id = Uuid::new_v4();
        let job = sqlx::query_as!(
            Job,
            r#"INSERT INTO jobs (id, owner_id, job_type, picture_id, config, idempotency_key, trace_context)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (owner_id, idempotency_key)
                   WHERE idempotency_key IS NOT NULL AND status IN ('pending', 'processing')
                   DO UPDATE SET idempotency_key = jobs.idempotency_key
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
            id,
            owner_id,
            job_type as JobType,
            picture_id,
            config_value as serde_json::Value,
            idempotency_key,
            trace_context as Option<serde_json::Value>,
        )
        .fetch_one(ex)
        .await
            .map_err(map_sqlx_error)?;

        Ok(IdempotentJob {
            created: job.id == id,
            job,
        })
    }

    /// Whether a `gen_thumbnail` job is in flight (`pending` / `processing`) for a picture. The
    /// guard the re-extraction paths use in place of an idempotency key (feature 33 §8).
    #[tracing::instrument(skip(ex), fields(picture_id = %picture_id))]
    pub async fn has_inflight_thumbnail<'e, E>(ex: E, picture_id: Uuid) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let found = sqlx::query_scalar!(
            "SELECT 1 FROM jobs
              WHERE picture_id = $1 AND job_type = 'gen_thumbnail'
                AND status IN ('pending', 'processing')
              LIMIT 1",
            picture_id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(found.is_some())
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

    /// Replace a job's `config` JSONB. Used to persist the EXIF target bound at claim-time
    /// (feature 31 §3.3), so the completion handler sees the state the file was brought to.
    #[tracing::instrument(skip(ex, config), fields(job_id = %job_id))]
    pub async fn update_config<'e, E>(
        ex: E,
        job_id: Uuid,
        config: &JobConfig,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let config_value = serde_json::to_value(config)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        sqlx::query!(
            "UPDATE jobs SET config = $2 WHERE id = $1",
            job_id,
            config_value as serde_json::Value,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
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
    pub async fn reset_stale(db: &PgPool, timeout_secs: i64) -> Result<Vec<StaleReset>, AppError> {
        let rows = sqlx::query!(
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
                 AND started_at < (now() AT TIME ZONE 'utc') - ($1 * INTERVAL '1 second')
               RETURNING id,
                         picture_id,
                         job_type AS "job_type: JobType",
                         status   AS "status: JobStatus",
                         config   AS "config: serde_json::Value""#,
            timeout_secs as f64,
        )
        .fetch_all(db)
        .await
        .map_err(map_sqlx_error)?;
        Ok(rows
            .into_iter()
            .map(|r| StaleReset {
                job_id: r.id,
                picture_id: r.picture_id,
                job_type: r.job_type,
                status: r.status,
                config: r.config,
            })
            .collect())
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
        JobRepository::create(db, owner_id, None, &config)
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

    fn thumb_config() -> JobConfig {
        JobConfig::GenThumbnail(GenThumbnailConfig {
            picture_id: Uuid::new_v4(),
            is_initial: true,
        })
    }

    /// The point of the mechanism: a retry is a no-op that hands back the job already in flight,
    /// not a 409.
    #[sqlx::test(migrator = "MIGRATOR")]
    async fn duplicate_idempotency_key_returns_the_live_job(db: PgPool) {
        let owner = seed_user(&db).await;
        let config = thumb_config();

        let first = JobRepository::create_idempotent(&db, owner, None, &config, "unique-key")
            .await
            .unwrap();
        assert!(first.created);

        let second = JobRepository::create_idempotent(&db, owner, None, &config, "unique-key")
            .await
            .unwrap();
        assert!(!second.created, "the retry must not enqueue a second job");
        assert_eq!(second.job.id, first.job.id);
    }

    /// Liveness scoping: the key guards the queue, not all of history. A terminal job releases it,
    /// so re-enqueuing no longer waits on `JobCleanupRoutine`'s retention window.
    #[sqlx::test(migrator = "MIGRATOR")]
    async fn terminal_job_frees_its_idempotency_key(db: PgPool) {
        let owner = seed_user(&db).await;
        let config = thumb_config();

        let first = JobRepository::create_idempotent(&db, owner, None, &config, "unique-key")
            .await
            .unwrap();
        let claimed = JobRepository::claim_next(&db, "worker1", &[])
            .await
            .unwrap()
            .unwrap();

        // Still live while `processing`.
        let during = JobRepository::create_idempotent(&db, owner, None, &config, "unique-key")
            .await
            .unwrap();
        assert!(!during.created);
        assert_eq!(during.job.id, first.job.id);

        JobRepository::complete(
            &db,
            claimed.id,
            claimed.claim_token.unwrap(),
            serde_json::json!({}),
        )
        .await
        .unwrap();

        let after = JobRepository::create_idempotent(&db, owner, None, &config, "unique-key")
            .await
            .unwrap();
        assert!(after.created, "a terminal job must release its key");
        assert_ne!(after.job.id, first.job.id);
    }

    /// Owner scoping comes from the index, so a key no longer has to carry the owner in its text.
    #[sqlx::test(migrator = "MIGRATOR")]
    async fn idempotency_key_is_scoped_per_owner(db: PgPool) {
        let alice = seed_user(&db).await;
        let bob = seed_user(&db).await;
        let config = thumb_config();

        let a = JobRepository::create_idempotent(&db, alice, None, &config, "shared-key")
            .await
            .unwrap();
        let b = JobRepository::create_idempotent(&db, bob, None, &config, "shared-key")
            .await
            .unwrap();
        assert!(a.created && b.created);
        assert_ne!(a.job.id, b.job.id);
    }

    /// Unkeyed enqueues are never deduplicated against each other.
    #[sqlx::test(migrator = "MIGRATOR")]
    async fn create_without_a_key_always_inserts(db: PgPool) {
        let owner = seed_user(&db).await;
        let config = thumb_config();

        let first = JobRepository::create(&db, owner, None, &config)
            .await
            .unwrap();
        let second = JobRepository::create(&db, owner, None, &config)
            .await
            .unwrap();
        assert_ne!(first.id, second.id);
        assert!(first.idempotency_key.is_none());
    }
}
