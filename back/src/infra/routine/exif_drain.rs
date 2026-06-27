//! Deferred-EXIF-job drain [`Routine`] (feature 14 §5).
//!
//! A batch EXIF edit applies a single set-based UPDATE that stamps `exif_sync_status =
//! 'pending_job_creation'` instead of enumerating-then-creating one `edit_picture` job per picture.
//! This drain picks those rows up, creates the reconcile jobs in batches, and flips them to
//! `pending`. It is a `()`-keyed [`Routine`] (`infra::routine`): a batch edit triggers it via the
//! `RoutineHandle<()>`, and a short interval (with a startup sweep) is the crash/lost-wake recovery
//! backstop. Each `run` drains until empty, so a single trigger covers an arbitrary backlog.

use crate::infra::routine::{Routine, RoutineHandle};
use sqlx::PgPool;
use std::time::Duration;
use tracing::debug;

/// Wake handle for the EXIF-job drain — `trigger(())` after a batch EXIF edit stamps new
/// `pending_job_creation` rows.
pub type ExifDrainHandle = RoutineHandle<()>;

/// Turns `pending_job_creation` rows (stamped by batch EXIF edits) into `edit_picture` reconcile jobs.
pub struct ExifDrain {
    db: PgPool,
    interval: Duration,
    batch: i64,
}

impl ExifDrain {
    pub fn new(db: PgPool, interval: Duration, batch: i64) -> Self {
        Self {
            db,
            interval,
            batch,
        }
    }
}

#[async_trait::async_trait]
impl Routine for ExifDrain {
    type Input = ();
    type Key = ();

    fn name(&self) -> &'static str {
        "exif_drain"
    }

    fn key(_input: &()) {}

    fn interval(&self) -> Option<Duration> {
        Some(self.interval)
    }

    fn run_on_startup(&self) -> bool {
        true
    }

    /// Drain repeatedly until a pass creates fewer than `batch` jobs (a single trigger may cover more
    /// than one batch worth of `pending_job_creation` rows).
    async fn run(&self, _input: ()) -> anyhow::Result<()> {
        loop {
            let created =
                crate::services::jobs::create_deferred_exif_jobs(&self.db, self.batch).await?;
            if created > 0 {
                debug!(created, "exif drain created reconcile jobs");
            }
            if (created as i64) < self.batch {
                break;
            }
        }
        Ok(())
    }
}
