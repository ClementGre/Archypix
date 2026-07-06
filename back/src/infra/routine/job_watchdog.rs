//! Job-table maintenance [`Routine`]s: the stale-`processing` watchdog and the terminal-row cleanup.
//!
//! - [`JobWatchdogRoutine`] periodically resets jobs stuck in `processing` (a worker that crashed,
//!   was OOM-killed, or lost connectivity after claiming a job). Without recovery those jobs would
//!   stay in `processing` forever. It calls [`JobRepository::reset_stale`], which resets eligible
//!   jobs to `pending` (or to `failed` if their retry budget is exhausted).
//! - [`JobCleanupRoutine`] prunes terminal (`completed` / `failed`) job rows older than a retention
//!   window so the `jobs` table does not grow without bound (every upload creates a `gen_thumbnail`
//!   job; EXIF/visual edits add more).
//!
//! Both are `()`-keyed sweep-only routines (`infra::routine`): no manual trigger, the default sweep
//! runs `run(())` on each interval tick.

use crate::infra::routine::Routine;
use crate::infra::settings::keys;
use crate::repository::job::JobRepository;
use archypix_common::settings::Settings;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

/// Periodically resets jobs stuck in `processing` back to `pending` (or `failed`).
pub struct JobWatchdogRoutine {
    db: PgPool,
    settings: Arc<Settings>,
}

impl JobWatchdogRoutine {
    pub fn new(db: PgPool, settings: Arc<Settings>) -> Self {
        Self { db, settings }
    }
}

#[async_trait::async_trait]
impl Routine for JobWatchdogRoutine {
    type Input = ();
    type Key = ();

    fn name(&self) -> &'static str {
        "job_watchdog"
    }

    fn key(_input: &()) {}

    fn interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(
            self.settings.get(keys::JOB_WATCHDOG_INTERVAL_SECS),
        ))
    }

    async fn run(&self, _input: ()) -> anyhow::Result<()> {
        let timeout_secs = self.settings.get(keys::JOB_PROCESSING_TIMEOUT_SECS);
        let n = JobRepository::reset_stale(&self.db, timeout_secs).await?;
        if n > 0 {
            info!(reset = n, "job watchdog: reset stale jobs");
        }
        Ok(())
    }
}

/// Periodically deletes terminal job rows older than `retention_secs`.
pub struct JobCleanupRoutine {
    db: PgPool,
    settings: Arc<Settings>,
}

impl JobCleanupRoutine {
    pub fn new(db: PgPool, settings: Arc<Settings>) -> Self {
        Self { db, settings }
    }
}

#[async_trait::async_trait]
impl Routine for JobCleanupRoutine {
    type Input = ();
    type Key = ();

    fn name(&self) -> &'static str {
        "job_cleanup"
    }

    fn key(_input: &()) {}

    fn interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(
            self.settings.get(keys::JOB_CLEANUP_INTERVAL_SECS),
        ))
    }

    async fn run(&self, _input: ()) -> anyhow::Result<()> {
        let retention_secs = self.settings.get(keys::JOB_RETENTION_SECS);
        let n = JobRepository::delete_terminal_older_than(&self.db, retention_secs).await?;
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

        let settings = crate::infra::settings::test_settings_with(&[]);
        let task = JobCleanupRoutine::new(db.clone(), settings);
        task.run(()).await.unwrap();

        let remaining: i64 = sqlx::query_scalar!("SELECT COUNT(*) FROM jobs")
            .fetch_one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(remaining, 2, "only the old completed job should be deleted");
    }
}
