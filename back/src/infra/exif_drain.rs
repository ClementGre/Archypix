//! Deferred-EXIF-job drain (feature 14 §5).
//!
//! A batch EXIF edit applies a single set-based UPDATE that stamps `exif_sync_status =
//! 'pending_job_creation'` instead of enumerating-then-creating one `edit_picture` job per picture.
//! This drain — mirroring the tagging pipeline's dirty-then-drain pattern — picks those rows up,
//! creates the reconcile jobs in batches, and flips them to `pending`.
//!
//! Like the pipeline, the drain is **event-driven with a poll fallback**: a batch edit wakes it
//! immediately via [`ExifDrainWaker`], and a short interval is the crash/lost-wake recovery
//! backstop, so a missed wake is only a latency issue, never a correctness one.

use crate::infra::error::AppError;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tracing::{debug, error};

/// Cheaply-cloneable handle for waking the EXIF-job drain. Clone into `AppState`; call
/// [`wake`](Self::wake) after a batch EXIF edit stamps new `pending_job_creation` rows.
#[derive(Clone)]
pub struct ExifDrainWaker {
    notify: Arc<Notify>,
}

impl ExifDrainWaker {
    /// Wake the drain to run a pass promptly. A missed wake is recovered by the poll interval.
    pub fn wake(&self) {
        self.notify.notify_one();
    }

    /// A waker not attached to any loop; its wakes are discarded. For tests and standalone calls.
    pub fn disconnected() -> Self {
        Self {
            notify: Arc::new(Notify::new()),
        }
    }
}

/// Build the waker and the drain loop future. The loop drains on each wake and on each interval
/// tick; spawn the future with `tokio::spawn`.
pub fn create(
    db: PgPool,
    interval: Duration,
    batch: i64,
) -> (ExifDrainWaker, impl Future<Output = ()>) {
    let notify = Arc::new(Notify::new());
    let waker = ExifDrainWaker {
        notify: notify.clone(),
    };
    let loop_fut = async move {
        loop {
            if let Err(e) = drain_until_empty(&db, batch).await {
                error!(error = ?e, "exif drain pass failed");
            }
            tokio::select! {
                _ = notify.notified() => {}
                _ = tokio::time::sleep(interval) => {}
            }
        }
    };
    (waker, loop_fut)
}

/// Drain repeatedly until a pass creates no jobs (a single wake may cover more than one batch worth
/// of `pending_job_creation` rows).
async fn drain_until_empty(db: &PgPool, batch: i64) -> Result<(), AppError> {
    loop {
        let created = crate::services::jobs::create_deferred_exif_jobs(db, batch).await?;
        if created > 0 {
            debug!(created, "exif drain created reconcile jobs");
        }
        if (created as i64) < batch {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_waker_wake_is_a_noop() {
        // A disconnected waker (tests / standalone calls) drops its wakes without panicking.
        ExifDrainWaker::disconnected().wake();
    }
}
