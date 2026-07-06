//! Tagging pipeline [`Routine`].
//!
//! The pipeline evaluates enabled tagging services against dirty pictures and applies the resulting
//! tag assignments, then diffs share coverage against the `share_announcements` tracking table to
//! announce/unannounce shared pictures. A picture is dirty when:
//! - Its `last_pipeline_run_at` is NULL (never processed), or
//! - Its `last_pipeline_run_at` is older than any enabled service's `last_invalidated_at`.
//!
//! It is a [`Routine`] (`infra::routine`): `Input = Key = Uuid` (the user whose pictures/shares
//! changed). Per-user runs are serialized; parallel across users up to `PIPELINE_CONCURRENCY`.
//! Triggers arriving while a user is running coalesce into a single re-run. The [`sweep`] is the
//! recovery/poll fallback (and startup pass): it re-derives every user with dirty pictures or a
//! share awaiting (re)announcement, so a missed trigger is only a latency issue. See
//! `doc/features/02_pipeline_announcement_robustness.md` and `doc/features/17_unified_routine_framework.md`.

pub mod announcement;
pub mod dedup;
pub mod evaluation;

use crate::clients::federation::FederationClient;
use crate::infra::redis::Cache;
use crate::infra::routine::{Routine, RoutineHandle};
use crate::infra::settings::keys;
use crate::repository::dedup::DedupRepository;
use crate::repository::pipeline::PipelineRepository;
use crate::repository::share_announcement::ShareAnnouncementRepository;
use archypix_common::error::AppError;
use archypix_common::settings::Settings;
use sqlx::PgPool;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use uuid::Uuid;

/// Borrowed dependencies for a single per-user pipeline run. Delivery is inline (the pipeline
/// announces/unannounces itself), so a run needs the federation client, the cache (for same-backend
/// resolution via `find_local_user_id`), and the pipeline handle (to wake same-backend recipients
/// after local registration).
pub struct PipelineRun<'a> {
    pub db: &'a PgPool,
    pub federation: &'a FederationClient,
    pub cache: &'a dyn Cache,
    pub settings: &'a Arc<Settings>,
    pub waker: &'a RoutineHandle<Uuid>,
}

// ── Routine ───────────────────────────────────────────────────────────────────

/// The tagging pipeline as a routine. Holds its own handle (wired after [`spawn`](crate::infra::routine::spawn)
/// via [`set_handle`](Self::set_handle)) so a run can wake same-backend recipients.
pub struct PipelineRoutine {
    db: PgPool,
    federation: FederationClient,
    cache: Arc<dyn Cache>,
    settings: Arc<Settings>,
    handle: Arc<OnceLock<RoutineHandle<Uuid>>>,
}

impl PipelineRoutine {
    pub fn new(
        db: PgPool,
        federation: FederationClient,
        cache: Arc<dyn Cache>,
        settings: Arc<Settings>,
    ) -> Self {
        Self {
            db,
            federation,
            cache,
            settings,
            handle: Arc::new(OnceLock::new()),
        }
    }

    /// A clonable cell shared with the routine: set it to the routine's own handle after `spawn`.
    pub fn handle_cell(&self) -> Arc<OnceLock<RoutineHandle<Uuid>>> {
        self.handle.clone()
    }
}

#[async_trait::async_trait]
impl Routine for PipelineRoutine {
    type Input = Uuid;
    type Key = Uuid;

    fn name(&self) -> &'static str {
        "pipeline"
    }

    fn key(input: &Uuid) -> Uuid {
        *input
    }

    fn interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(
            self.settings.get(keys::PIPELINE_POLL_INTERVAL_SECS),
        ))
    }

    fn run_on_startup(&self) -> bool {
        true
    }

    fn debounce(&self) -> Duration {
        Duration::from_millis(self.settings.get(keys::PIPELINE_DEBOUNCE_MS))
    }

    // Concurrency sizes the semaphore at spawn (restart-required setting).
    fn concurrency(&self) -> usize {
        self.settings.get(keys::PIPELINE_CONCURRENCY).max(1)
    }

    /// Recovery/poll sweep: re-mark announce-stale rows dirty, then trigger every user with dirty
    /// pictures or a share awaiting (re)announcement.
    async fn sweep(&self, h: &RoutineHandle<Uuid>) -> anyhow::Result<()> {
        // Announce-stale backstop (D): mark dirty any picture whose last announce trails the row
        // (e.g. a worker-completion fast-path wake that lost the race against the first announce).
        let stale = ShareAnnouncementRepository::find_stale_announcement_pictures(&self.db).await?;
        if !stale.is_empty() {
            PipelineRepository::invalidate(&self.db, &stale).await?;
        }

        let mut users = PipelineRepository::find_users_with_dirty_pictures(&self.db).await?;
        // Content-dedup backstop (feature 11 §5.2): users whose groups need a promotion/collapse but
        // may have missed their event-driven wake (e.g. a lost cross-instance owner-purge unannounce).
        users.extend(DedupRepository::find_users_needing_reconcile(&self.db).await?);
        users.sort_unstable();
        users.dedup();
        for user_id in users {
            h.trigger(user_id);
        }
        Ok(())
    }

    async fn run(&self, user_id: Uuid) -> anyhow::Result<()> {
        let handle = self
            .handle
            .get()
            .expect("pipeline handle not wired after spawn");
        let run = PipelineRun {
            db: &self.db,
            federation: &self.federation,
            cache: self.cache.as_ref(),
            settings: &self.settings,
            waker: handle,
        };
        evaluation::run_for_user(&run, user_id).await?;
        Ok(())
    }
}

/// Used for testing only. Runs one full pipeline pass for a user with inline delivery.
pub async fn run_once_for_user(
    db: &PgPool,
    federation: &FederationClient,
    cache: &dyn Cache,
    settings: &Arc<Settings>,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
) -> Result<(), AppError> {
    let run = PipelineRun {
        db,
        federation,
        cache,
        settings,
        waker,
    };
    evaluation::run_for_user(&run, user_id).await
}
