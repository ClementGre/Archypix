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

