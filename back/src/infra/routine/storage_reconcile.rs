//! Storage-usage reconcile sweep (feature 22 §7).
//!
//! A sweep-only [`Routine`](crate::infra::routine::Routine): recompute every user's four billed
//! counters from scratch (a set of grouped `SUM … GROUP BY` scans, not per-object work), overwrite
//! `user_storage`, and refresh the `storage:committed:*` Redis mirror. This is the drift safety net
//! that lets the trigger-maintained fast counter be trusted.

use crate::infra::error::AppError;
use crate::infra::redis::{Cache, RedisKey};
use crate::infra::routine::Routine;
use crate::repository::user_storage::UserStorageRepository;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

pub struct StorageReconcileTask {
    db: PgPool,
    cache: Arc<dyn Cache>,
    interval: Duration,
}

impl StorageReconcileTask {
    pub fn new(db: PgPool, cache: Arc<dyn Cache>, interval: Duration) -> Self {
        Self {
            db,
            cache,
            interval,
        }
    }
}

#[async_trait::async_trait]
impl Routine for StorageReconcileTask {
    type Input = ();
    type Key = ();

    fn name(&self) -> &'static str {
        "storage_reconcile"
    }

    fn key(_input: &()) {}

    fn interval(&self) -> Option<Duration> {
        Some(self.interval)
    }

    async fn run(&self, _input: ()) -> anyhow::Result<()> {
        let totals = UserStorageRepository::reconcile_all(&self.db).await?;
        // Refresh the cached committed mirror to match the recomputed truth.
        for (user_id, billed) in &totals {
            let _: Result<(), AppError> = self
                .cache
                .set_str_ex(
                    RedisKey::StorageCommitted(*user_id),
                    &billed.to_string(),
                    3600,
                )
                .await;
        }
        info!(
            users = totals.len(),
            "storage reconcile: recomputed usage counters"
        );
        Ok(())
    }
}
