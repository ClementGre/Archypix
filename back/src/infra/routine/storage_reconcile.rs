//! Storage-usage reconcile sweep (feature 22 §7).
//!
//! A sweep-only [`Routine`](crate::infra::routine::Routine): recompute every user's four billed
//! counters from scratch (a set of grouped `SUM … GROUP BY` scans, not per-object work), overwrite
//! `user_storage`, and refresh the `storage:committed:*` Redis mirror. This is the drift safety net
//! that lets the trigger-maintained fast counter be trusted.

use crate::infra::redis::{Cache, RedisKey};
use crate::infra::routine::Routine;
use crate::infra::settings::keys;
use crate::repository::user_storage::UserStorageRepository;
use archypix_common::error::AppError;
use archypix_common::settings::Settings;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

pub struct StorageReconcileRoutine {
    db: PgPool,
    cache: Arc<dyn Cache>,
    settings: Arc<Settings>,
}

impl StorageReconcileRoutine {
    pub fn new(db: PgPool, cache: Arc<dyn Cache>, settings: Arc<Settings>) -> Self {
        Self {
            db,
            cache,
            settings,
        }
    }
}

#[async_trait::async_trait]
impl Routine for StorageReconcileRoutine {
    type Input = ();
    type Key = ();

    fn name(&self) -> &'static str {
        "storage_reconcile"
    }

    fn key(_input: &()) {}

    fn interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(
            self.settings.get(keys::STORAGE_RECONCILE_INTERVAL_SECS),
        ))
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
