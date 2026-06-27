//! Job-table maintenance [`Routine`]s: the stale-`processing` watchdog and the terminal-row cleanup.
//!
//! - [`JobWatchdogTask`] periodically resets jobs stuck in `processing` (a worker that crashed,
//!   was OOM-killed, or lost connectivity after claiming a job). Without recovery those jobs would
//!   stay in `processing` forever. It calls [`JobRepository::reset_stale`], which resets eligible
//!   jobs to `pending` (or to `failed` if their retry budget is exhausted).
//! - [`JobCleanupTask`] prunes terminal (`completed` / `failed`) job rows older than a retention
//!   window so the `jobs` table does not grow without bound (every upload creates a `gen_thumbnail`
//!   job; EXIF/visual edits add more).
//!
//! Both are `()`-keyed sweep-only routines (`infra::routine`): no manual trigger, the default sweep
//! runs `run(())` on each interval tick.

use crate::infra::routine::Routine;
use crate::repository::job::JobRepository;
use sqlx::PgPool;
use std::time::Duration;
use tracing::info;

/// Periodically resets jobs stuck in `processing` back to `pending` (or `failed`).
pub struct JobWatchdogTask {
    db: PgPool,
    timeout_secs: i64,
    interval: Duration,
}

impl JobWatchdogTask {
    pub fn new(db: PgPool, timeout_secs: i64, interval: Duration) -> Self {
        Self {
            db,
            timeout_secs,
            interval,
        }
    }
}

#[async_trait::async_trait]
impl Routine for JobWatchdogTask {
    type Input = ();
    type Key = ();

    fn name(&self) -> &'static str {
        "job_watchdog"
    }

    fn key(_input: &()) {}

    fn interval(&self) -> Option<Duration> {
        Some(self.interval)
    }

    async fn run(&self, _input: ()) -> anyhow::Result<()> {
        let n = JobRepository::reset_stale(&self.db, self.timeout_secs).await?;
        if n > 0 {
            info!(reset = n, "job watchdog: reset stale jobs");
        }
        Ok(())
    }
}

/// Periodically deletes terminal job rows older than `retention_secs`.
pub struct JobCleanupTask {
    db: PgPool,
    retention_secs: i64,
    interval: Duration,
}

impl JobCleanupTask {
    pub fn new(db: PgPool, retention_secs: i64, interval: Duration) -> Self {
        Self {
            db,
            retention_secs,
            interval,
        }
    }
}

#[async_trait::async_trait]
impl Routine for JobCleanupTask {
    type Input = ();
    type Key = ();

    fn name(&self) -> &'static str {
        "job_cleanup"
    }

    fn key(_input: &()) {}

    fn interval(&self) -> Option<Duration> {
        Some(self.interval)
    }

    async fn run(&self, _input: ()) -> anyhow::Result<()> {
        let n = JobRepository::delete_terminal_older_than(&self.db, self.retention_secs).await?;
        if n > 0 {
            info!(deleted = n, "job cleanup: pruned terminal jobs");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn cleanup_task_tick_deletes_old_terminal_jobs(db: PgPool) {
        let user_id = uuid::Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO users (id, username, email, display_name) VALUES ($1, $2, $3, $4)",
            user_id,
            "cleanup_user",
            "cleanup@test.com",
            "Cleanup User",
        )
        .execute(&db)
        .await
        .unwrap();

        // Old completed job — should be pruned.
        sqlx::query!(
            "INSERT INTO jobs (owner_id, job_type, status, completed_at)
             VALUES ($1, 'gen_thumbnail', 'completed', (now() AT TIME ZONE 'utc') - INTERVAL '40 days')",
            user_id,
        )
            .execute(&db)
            .await
            .unwrap();
        // Recent completed job — should remain.
        sqlx::query!(
            "INSERT INTO jobs (owner_id, job_type, status, completed_at)
             VALUES ($1, 'gen_thumbnail', 'completed', (now() AT TIME ZONE 'utc'))",
            user_id,
        )
        .execute(&db)
        .await
        .unwrap();
        // Pending job — never touched.
        sqlx::query!(
            "INSERT INTO jobs (owner_id, job_type, status) VALUES ($1, 'gen_thumbnail', 'pending')",
            user_id,
        )
        .execute(&db)
        .await
        .unwrap();

        let task = JobCleanupTask::new(db.clone(), 2_592_000, Duration::from_secs(86_400));
        task.run(()).await.unwrap();

        let remaining: i64 = sqlx::query_scalar!("SELECT COUNT(*) FROM jobs")
            .fetch_one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(remaining, 2, "only the old completed job should be deleted");
    }
}
