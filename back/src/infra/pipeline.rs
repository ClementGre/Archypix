//! Tagging pipeline background loop.
//!
//! The pipeline evaluates enabled tagging services against dirty pictures and
//! applies the resulting tag assignments, then diffs share coverage against the
//! `share_announcements` tracking table to announce/unannounce shared pictures.
//! A picture is dirty when:
//! - Its `last_pipeline_run_at` is NULL (never processed), or
//! - Its `last_pipeline_run_at` is older than any enabled service's `last_invalidated_at`.
//!
//! # Wake model
//! Producers call [`PipelineWaker::wake`] with the **id of the user whose pictures or shares
//! changed** (not necessarily the request caller). The wake is an `mpsc<Uuid>` message consumed by
//! the loop's per-user scheduler. A configurable poll interval provides a recovery sweep for
//! crash/lost-wake recovery, so a missed wake is only a latency issue, never a correctness one.
//!
//! # Concurrency
//! Per-user runs are serialized (one worker per `user_id` at a time — concurrent runs for the same
//! user would race on its tag reconcile and tracking writes) and parallel across users, bounded by
//! `PIPELINE_CONCURRENCY`. Wakes that arrive while a user is running are coalesced into a single
//! re-run. See `doc/features/02_pipeline_announcement_robustness.md` §7.

pub mod announcement;
pub mod dedup;
pub mod evaluation;

use crate::clients::federation::FederationClient;
use crate::infra::config::Config;
use crate::infra::error::AppError;
use crate::infra::redis::Cache;
use crate::repository::dedup::DedupRepository;
use crate::repository::pipeline::PipelineRepository;
use crate::repository::share_announcement::ShareAnnouncementRepository;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Semaphore, mpsc};
use tracing::Instrument;
use uuid::Uuid;

/// Borrowed dependencies for a single per-user pipeline run. Delivery is now inline (the pipeline
/// announces/unannounces itself rather than enqueuing tasks), so a run needs the federation client,
/// the cache (for same-backend resolution via `find_local_user_id`), and the waker (to wake
/// same-backend recipients after local registration).
pub struct PipelineRun<'a> {
    pub db: &'a PgPool,
    pub federation: &'a FederationClient,
    pub cache: &'a dyn Cache,
    pub config: &'a Config,
    pub waker: &'a PipelineWaker,
}

// ── Waker ───────────────────────────────────────────────────────────────────

/// Cheaply-cloneable handle for waking the pipeline for a specific user. Clone this into
/// `AppState` and the task runner; call [`wake`](Self::wake) after any event that creates dirty
/// pictures or share work for that user (ingest, tag edit, service config change, share accept,
/// same-backend (un)announce, …).
///
/// Each wake carries a `debounce` flag (the `bool` in the channel item). Interactive events (tag
/// edit, service edit, upload, share lifecycle) use [`wake`](Self::wake) and start a run promptly;
/// worker-driven events that arrive as per-picture bursts (thumbnail/EXIF reconcile completion) use
/// [`wake_debounced`](Self::wake_debounced) so the scheduler can collapse the burst into one run.
#[derive(Clone)]
pub struct PipelineWaker {
    tx: mpsc::UnboundedSender<(Uuid, bool)>,
}

impl PipelineWaker {
    /// Wake the pipeline for `user_id` promptly (no debounce). Silently no-ops if the loop has shut
    /// down — a missed wake is recovered by the poll sweep.
    pub fn wake(&self, user_id: Uuid) {
        let _ = self.tx.send((user_id, false));
    }

    /// Wake the pipeline for `user_id` through the debounce window (`PIPELINE_DEBOUNCE_MS`). Used by
    /// worker-completion paths whose wakes arrive one-per-picture, so a burst collapses into a single
    /// run instead of one run per picture. An interactive `wake` arriving during the window promotes
    /// it to run immediately.
    pub fn wake_debounced(&self, user_id: Uuid) {
        let _ = self.tx.send((user_id, true));
    }

    /// A waker not attached to any loop; its wakes are discarded. For tests and standalone calls.
    pub fn disconnected() -> Self {
        let (tx, _rx) = mpsc::unbounded_channel();
        PipelineWaker { tx }
    }
}

/// Build the waker and the receiver consumed by [`create`]. Splitting construction lets `main` wire
/// the waker into the `TaskQueue` (which wakes recipients after same-backend delivery) before the
/// loop future is built, breaking the waker ↔ task_queue cycle.
pub fn channel() -> (PipelineWaker, mpsc::UnboundedReceiver<(Uuid, bool)>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (PipelineWaker { tx }, rx)
}

// ── Loop ─────────────────────────────────────────────────────────────────────

/// Per-user run state held by the scheduler.
struct RunState {
    phase: Phase,
    /// A wake arrived while a run was in flight → run once more after it completes.
    rerun: bool,
    /// At least one wake (the triggering one or a `rerun`) was interactive (non-debounced), so the
    /// run must not wait in the debounce window.
    immediate: bool,
}

enum Phase {
    /// A debounce window is open: the run has not started, wakes are being coalesced until the timer
    /// fires. Only reachable when `pipeline_debounce_ms > 0` and every wake so far was debounced.
    Pending,
    /// A worker is running (or queued on the semaphore) for this user.
    Running,
}

impl RunState {
    fn running() -> Self {
        Self {
            phase: Phase::Running,
            rerun: false,
            immediate: false,
        }
    }
    fn pending() -> Self {
        Self {
            phase: Phase::Pending,
            rerun: false,
            immediate: false,
        }
    }
}

/// What [`Scheduler::schedule`] / the post-run settle decided to do, performed after the state lock
/// is released (spawning under the lock would hold a std mutex across an await point).
enum Action {
    None,
    Run,
    Timer,
}

/// Shared context handed to each per-user worker. Holds owned dependencies; each run borrows them
/// into a [`PipelineRun`].
struct Scheduler {
    db: PgPool,
    federation: FederationClient,
    cache: Arc<dyn Cache>,
    config: Config,
    waker: PipelineWaker,
    sem: Arc<Semaphore>,
    state: Arc<Mutex<HashMap<Uuid, RunState>>>,
}

impl Scheduler {
    /// Ensure a run is (or will be) scheduled for `user_id`, coalescing concurrent wakes.
    ///
    /// `debounce` requests the wait window; it is honoured only when `pipeline_debounce_ms > 0`. An
    /// interactive (non-debounced) wake never waits: from idle it starts a run immediately, and one
    /// arriving during an open debounce window **promotes** it to run now. Debounced wakes from idle
    /// open the window so a per-picture worker-completion burst collapses into a single run.
    fn schedule(self: &Arc<Self>, user_id: Uuid, debounce: bool) {
        let immediate = !debounce || self.config.pipeline_debounce_ms == 0;
        let action = {
            let mut map = self
                .state
                .lock()
                .expect("pipeline scheduler mutex poisoned");
            if let Some(s) = map.get_mut(&user_id) {
                match s.phase {
                    // Debounce window open: an interactive wake promotes it; a debounced one coalesces.
                    Phase::Pending if immediate => {
                        s.phase = Phase::Running;
                        Action::Run
                    }
                    Phase::Pending => Action::None,
                    // A run is in flight — request a single re-run after it, tracking its urgency.
                    Phase::Running => {
                        s.rerun = true;
                        if immediate {
                            s.immediate = true;
                        }
                        Action::None
                    }
                }
            } else if immediate {
                map.insert(user_id, RunState::running());
                Action::Run
            } else {
                map.insert(user_id, RunState::pending());
                Action::Timer
            }
        };
        match action {
            Action::Run => self.spawn_run(user_id),
            Action::Timer => self.spawn_debounce_timer(user_id),
            Action::None => {}
        }
    }

    /// Sleep the debounce window, then flip `Pending → Running` and start the run. A stale timer
    /// (the entry was already promoted to `Running` or cleared) no-ops; the active timer wins.
    fn spawn_debounce_timer(self: &Arc<Self>, user_id: Uuid) {
        let this = Arc::clone(self);
        let delay = Duration::from_millis(this.config.pipeline_debounce_ms);
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            {
                let mut map = this
                    .state
                    .lock()
                    .expect("pipeline scheduler mutex poisoned");
                match map.get_mut(&user_id) {
                    Some(s) if matches!(s.phase, Phase::Pending) => s.phase = Phase::Running,
                    _ => return, // promoted/cleared by another path — let it own the run
                }
            }
            this.spawn_run(user_id);
        });
    }

    /// Run the pipeline for `user_id`, then settle: on a pending `rerun`, loop immediately when it was
    /// interactive (or debouncing is off), else re-open the debounce window; otherwise clear the entry.
    fn spawn_run(self: &Arc<Self>, user_id: Uuid) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let permit = this
                    .sem
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("pipeline semaphore closed");
                let run = PipelineRun {
                    db: &this.db,
                    federation: &this.federation,
                    cache: this.cache.as_ref(),
                    config: &this.config,
                    waker: &this.waker,
                };
                let run_id = uuid::Uuid::new_v4();
                let span =
                    tracing::info_span!("pipeline_run", user_id = %user_id, run_id = %run_id);
                if let Err(e) = evaluation::run_for_user(&run, user_id)
                    .instrument(span)
                    .await
                {
                    tracing::error!(user_id = %user_id, error = ?e, "pipeline: failed for user");
                }
                drop(permit);

                let action = {
                    let mut map = this
                        .state
                        .lock()
                        .expect("pipeline scheduler mutex poisoned");
                    match map.get_mut(&user_id) {
                        Some(s) if s.rerun => {
                            let run_now = s.immediate || this.config.pipeline_debounce_ms == 0;
                            s.rerun = false;
                            s.immediate = false;
                            if run_now {
                                s.phase = Phase::Running; // loop again immediately
                                Action::Run
                            } else {
                                s.phase = Phase::Pending;
                                Action::Timer
                            }
                        }
                        _ => {
                            map.remove(&user_id);
                            Action::None
                        }
                    }
                };
                match action {
                    Action::Run => continue,
                    Action::Timer => {
                        this.spawn_debounce_timer(user_id);
                        break;
                    }
                    Action::None => break,
                }
            }
        });
    }
}

/// Spawn the pipeline loop as a Tokio task.
///
/// Returns a future that runs forever (until the process exits). Spawn it with `tokio::spawn`.
/// `rx` comes from [`channel`]; the matching [`PipelineWaker`] is what producers call.
///
/// The loop is purely event-driven: the recovery/poll fallback now lives in
/// [`PipelineRecoverySweepTask`], a [`RecurringTask`] that pushes dirty users back through the
/// waker.
pub fn create(
    db: PgPool,
    rx: mpsc::UnboundedReceiver<(Uuid, bool)>,
    config: Config,
    concurrency: usize,
    federation: FederationClient,
    cache: Arc<dyn Cache>,
    waker: PipelineWaker,
) -> impl Future<Output = ()> {
    async move { run(db, rx, config, concurrency, federation, cache, waker).await }
}

async fn run(
    db: PgPool,
    mut rx: mpsc::UnboundedReceiver<(Uuid, bool)>,
    config: Config,
    concurrency: usize,
    federation: FederationClient,
    cache: Arc<dyn Cache>,
    waker: PipelineWaker,
) {
    tracing::info!(concurrency, "tagging pipeline loop started");

    let scheduler = Arc::new(Scheduler {
        db,
        federation,
        cache,
        config,
        waker,
        sem: Arc::new(Semaphore::new(concurrency.max(1))),
        state: Arc::new(Mutex::new(HashMap::new())),
    });

    loop {
        match rx.recv().await {
            Some((user_id, debounce)) => scheduler.schedule(user_id, debounce),
            None => break, // all wakers dropped — process shutting down
        }
    }

    tracing::info!("tagging pipeline loop stopped");
}

/// Recovery/poll fallback for the pipeline: periodically (and once at startup) re-wakes every user
/// that currently has dirty pictures or a share awaiting (re)announcement. Covers crash/lost-wake
/// recovery, so a missed wake is only a latency issue, never a correctness one.
pub struct PipelineRecoverySweepTask {
    db: PgPool,
    waker: PipelineWaker,
    interval: Duration,
}

impl PipelineRecoverySweepTask {
    pub fn new(db: PgPool, waker: PipelineWaker, interval: Duration) -> Self {
        Self {
            db,
            waker,
            interval,
        }
    }
}

#[async_trait::async_trait]
impl crate::infra::scheduler::RecurringTask for PipelineRecoverySweepTask {
    fn name(&self) -> &'static str {
        "pipeline_recovery_sweep"
    }

    fn interval(&self) -> Duration {
        self.interval
    }

    fn run_on_startup(&self) -> bool {
        true
    }

    #[tracing::instrument(skip(self))]
    async fn tick(&self) -> anyhow::Result<()> {
        // Announce-stale backstop (D): mark dirty any picture whose last announce trails the row
        // (e.g. a worker-completion fast-path wake that lost the race against the first announce).
        // Done before the dirty-user scan so these owners are then picked up by it.
        let stale = ShareAnnouncementRepository::find_stale_announcement_pictures(&self.db).await?;
        if !stale.is_empty() {
            PipelineRepository::invalidate(&self.db, &stale).await?;
        }

        let mut users = PipelineRepository::find_users_with_dirty_pictures(&self.db).await?;
        // Content-dedup backstop (feature 11 §5.2): users whose groups need a promotion/collapse but
        // may have missed their event-driven wake (e.g. a lost cross-instance owner-purge unannounce).
        let dedup_users = DedupRepository::find_users_needing_reconcile(&self.db).await?;
        users.extend(dedup_users);
        users.sort_unstable();
        users.dedup();
        for user_id in users {
            self.waker.wake(user_id);
        }
        Ok(())
    }
}

/// Used for testing only. Runs one full pipeline pass for a user with inline delivery.
pub async fn run_once_for_user(
    db: &PgPool,
    federation: &FederationClient,
    cache: &dyn Cache,
    config: &Config,
    waker: &PipelineWaker,
    user_id: Uuid,
) -> Result<(), AppError> {
    let run = PipelineRun {
        db,
        federation,
        cache,
        config,
        waker,
    };
    evaluation::run_for_user(&run, user_id).await
}
