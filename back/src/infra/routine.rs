//! Generic background-work runtime — the **Routine framework** (feature 17).
//!
//! A [`Routine`] is a named unit of background work triggerable three ways: recurrently (every
//! [`interval`](Routine::interval)), at startup ([`run_on_startup`](Routine::run_on_startup)), and
//! manually ([`RoutineHandle::trigger`]). Every trigger carries an [`Input`](Routine::Input); a
//! dedup [`Key`](Routine::Key) is *derived* from it. Equal keys never run concurrently — while a key
//! is running a new trigger sets a **rerun** flag (storing the latest input), and the runtime
//! re-runs once at the end. A debounce window coalesces a burst *before* the first run.
//!
//! This is the per-key debounce/coalesce/rerun scheduler that used to live in `infra/pipeline.rs`,
//! lifted into one generic runtime; the pipeline, exif drain, job watchdog/cleanup, purge sweep, and
//! the one-shot tag-rename / unannounce tasks are all `Routine`s now.
//!
//! **Durability.** Triggers are in-memory (`mpsc`); a crash drops queued triggers. A routine needing
//! crash-safety must provide a [`sweep`](Routine::sweep) that re-derives its outstanding work from
//! persistent state (the pipeline, drain, and sweeps do; tag-rename/unannounce are best-effort, as
//! before). See `doc/features/17_unified_routine_framework.md`.
//!
//! The concrete routines live in submodules alongside this framework core.

pub mod exif_drain;
pub mod job_watchdog;
pub mod pipeline;
pub mod purge_sweep;
pub mod storage_reconcile;
pub mod tag_rename;
pub mod unannounce;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Semaphore, mpsc, watch};
use tracing::{Instrument, error, info};
use uuid::Uuid;

// ── Trait ─────────────────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait Routine: Send + Sync + 'static {
    /// The run payload, carried by every trigger. `Default` only serves the default [`sweep`](Self::sweep).
    type Input: Clone + std::fmt::Debug + Default + Send + Sync + 'static;

    /// Dedup key, *derived* from the input via [`key`](Self::key). Equal keys never run concurrently.
    type Key: Clone + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static;

    /// Stable lower-snake name for logs/metrics, e.g. `"pipeline"`, `"job_cleanup"`.
    fn name(&self) -> &'static str;

    /// Derive the dedup key from the input. Simple routines set `type Key = Self::Input` and return
    /// `input.clone()` (identity).
    fn key(input: &Self::Input) -> Self::Key;

    /// Recurring cadence; `None` = trigger-only (no periodic sweep).
    fn interval(&self) -> Option<Duration> {
        None
    }

    /// Run one sweep at startup before the first interval sleep.
    fn run_on_startup(&self) -> bool {
        false
    }

    /// Debounce window for [`trigger_debounced`](RoutineHandle::trigger_debounced); `ZERO` disables.
    fn debounce(&self) -> Duration {
        Duration::ZERO
    }

    /// Max runs in flight across *distinct* keys. Per-key is always serial.
    fn concurrency(&self) -> usize {
        1
    }

    /// The periodic/startup action: enqueue the inputs that need a run. The default triggers the
    /// `Default` input once (correct for `()`-keyed routines). Keyed routines override to *enumerate*
    /// the work from the DB and trigger each.
    async fn sweep(&self, h: &RoutineHandle<Self::Input>) -> anyhow::Result<()> {
        h.trigger(Self::Input::default());
        Ok(())
    }

    /// Execute one run for `input`. Errors are logged, not propagated.
    async fn run(&self, input: Self::Input) -> anyhow::Result<()>;
}

// ── Handle ────────────────────────────────────────────────────────────────────

/// Cheaply-cloneable trigger handle. Stored in `AppState`; call from request handlers, other
/// routines, anywhere. Unifies the old `PipelineWaker`, `ExifDrainWaker`, and `TaskQueue`.
#[derive(Clone)]
pub struct RoutineHandle<I> {
    tx: mpsc::UnboundedSender<(I, bool)>, // bool = debounced
}

impl<I> RoutineHandle<I> {
    /// Trigger promptly (no debounce). Silently no-ops if the runtime has shut down — recovered by
    /// the next sweep.
    pub fn trigger(&self, input: I) {
        let _ = self.tx.send((input, false));
    }

    /// Trigger through the debounce window. An interactive [`trigger`](Self::trigger) arriving
    /// mid-window promotes the run to start immediately.
    pub fn trigger_debounced(&self, input: I) {
        let _ = self.tx.send((input, true));
    }

    /// A handle attached to no runtime; triggers are discarded. For tests and standalone calls.
    pub fn disconnected() -> Self {
        let (tx, _rx) = mpsc::unbounded_channel();
        Self { tx }
    }
}

// ── Runtime ───────────────────────────────────────────────────────────────────

/// Per-key run state held by the scheduler.
struct RunState<I> {
    phase: Phase,
    /// A trigger arrived while a run was in flight → run once more after it completes.
    rerun: bool,
    /// At least one trigger (the current or a `rerun`) was interactive (non-debounced).
    immediate: bool,
    /// Latest input to use for the next run (last-write-wins). Always present while the entry lives.
    next_input: I,
}

enum Phase {
    /// A debounce window is open: the run has not started, triggers are coalesced until the timer fires.
    Pending,
    /// A worker is running (or queued on the semaphore) for this key.
    Running,
}

impl<I> RunState<I> {
    fn running(input: I) -> Self {
        Self {
            phase: Phase::Running,
            rerun: false,
            immediate: false,
            next_input: input,
        }
    }
    fn pending(input: I) -> Self {
        Self {
            phase: Phase::Pending,
            rerun: false,
            immediate: false,
            next_input: input,
        }
    }
}

/// What [`Scheduler::schedule`] / the post-run settle decided, performed after the lock is released.
enum Action {
    None,
    Run,
    Timer,
}

struct Scheduler<R: Routine> {
    routine: Arc<R>,
    sem: Arc<Semaphore>,
    state: Mutex<HashMap<R::Key, RunState<R::Input>>>,
    debounce: Duration,
}

impl<R: Routine> Scheduler<R> {
    /// Ensure a run is (or will be) scheduled for `input`'s key, coalescing concurrent triggers.
    fn schedule(self: &Arc<Self>, input: R::Input, debounced: bool) {
        let key = R::key(&input);
        let immediate = !debounced || self.debounce.is_zero();
        let action = {
            let mut map = self.state.lock().expect("routine scheduler mutex poisoned");
            match map.get_mut(&key) {
                Some(s) => {
                    s.next_input = input;
                    match s.phase {
                        // Debounce window open: an interactive trigger promotes it; a debounced one coalesces.
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
                }
                None if immediate => {
                    map.insert(key.clone(), RunState::running(input));
                    Action::Run
                }
                None => {
                    map.insert(key.clone(), RunState::pending(input));
                    Action::Timer
                }
            }
        };
        match action {
            Action::Run => self.spawn_run(key),
            Action::Timer => self.spawn_debounce_timer(key),
            Action::None => {}
        }
    }

    /// Sleep the debounce window, then flip `Pending → Running` and start the run. A stale timer
    /// (the entry was already promoted or cleared) no-ops.
    fn spawn_debounce_timer(self: &Arc<Self>, key: R::Key) {
        let this = Arc::clone(self);
        let delay = this.debounce;
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            {
                let mut map = this.state.lock().expect("routine scheduler mutex poisoned");
                match map.get_mut(&key) {
                    Some(s) if matches!(s.phase, Phase::Pending) => s.phase = Phase::Running,
                    _ => return, // promoted/cleared by another path — let it own the run
                }
            }
            this.spawn_run(key);
        });
    }

    /// Run the routine for `key`, then settle: on a pending `rerun`, loop immediately when it was
    /// interactive (or debouncing is off), else re-open the debounce window; otherwise clear the entry.
    fn spawn_run(self: &Arc<Self>, key: R::Key) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let permit = this
                    .sem
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("routine semaphore closed");
                let input = {
                    let map = this.state.lock().expect("routine scheduler mutex poisoned");
                    match map.get(&key) {
                        Some(s) => s.next_input.clone(),
                        None => return, // shouldn't happen — entry lives until the run settles
                    }
                };
                let run_id = Uuid::new_v4();
                let span = tracing::info_span!(
                    "routine_run",
                    routine = this.routine.name(),
                    key = ?key,
                    run_id = %run_id
                );
                if let Err(e) = this.routine.run(input).instrument(span).await {
                    error!(routine = this.routine.name(), error = ?e, "routine run failed");
                }
                drop(permit);

                let action = {
                    let mut map = this.state.lock().expect("routine scheduler mutex poisoned");
                    match map.get_mut(&key) {
                        Some(s) if s.rerun => {
                            let run_now = s.immediate || this.debounce.is_zero();
                            s.rerun = false;
                            s.immediate = false;
                            if run_now {
                                s.phase = Phase::Running;
                                Action::Run
                            } else {
                                s.phase = Phase::Pending;
                                Action::Timer
                            }
                        }
                        _ => {
                            map.remove(&key);
                            Action::None
                        }
                    }
                };
                match action {
                    Action::Run => continue,
                    Action::Timer => {
                        this.spawn_debounce_timer(key);
                        break;
                    }
                    Action::None => break,
                }
            }
        });
    }

    /// Run the periodic/startup sweep, logging (never propagating) failures.
    async fn run_sweep(self: &Arc<Self>, handle: &RoutineHandle<R::Input>) {
        let run_id = Uuid::new_v4();
        let span =
            tracing::info_span!("routine_sweep", routine = self.routine.name(), run_id = %run_id);
        if let Err(e) = self.routine.sweep(handle).instrument(span).await {
            error!(routine = self.routine.name(), error = ?e, "routine sweep failed");
        }
    }
}

/// Spawn the routine's runtime onto the current Tokio runtime and return its trigger handle plus the
/// runtime's [`JoinHandle`](tokio::task::JoinHandle). `main` keeps the join handles and, after
/// flipping `shutdown`, awaits them to drain in-flight runs before exiting.
///
/// The only handle ↔ runtime cycle (the pipeline waking recipients via its own handle) is broken by
/// storing the handle inside the routine via interior mutability right after this returns, before the
/// spawned runtime can issue its first run.
pub fn spawn<R: Routine>(
    routine: R,
    shutdown: watch::Receiver<bool>,
) -> (RoutineHandle<R::Input>, tokio::task::JoinHandle<()>) {
    let (tx, rx) = mpsc::unbounded_channel::<(R::Input, bool)>();
    let handle = RoutineHandle { tx };
    let runtime_handle = handle.clone();
    let join = tokio::spawn(run_routine(Arc::new(routine), runtime_handle, rx, shutdown));
    (handle, join)
}

async fn run_routine<R: Routine>(
    routine: Arc<R>,
    handle: RoutineHandle<R::Input>,
    mut rx: mpsc::UnboundedReceiver<(R::Input, bool)>,
    shutdown: watch::Receiver<bool>,
) {
    let name = routine.name();
    info!(routine = name, "routine started");

    let scheduler = Arc::new(Scheduler {
        sem: Arc::new(Semaphore::new(routine.concurrency().max(1))),
        state: Mutex::new(HashMap::new()),
        debounce: routine.debounce(),
        routine: routine.clone(),
    });

    let mut children = Vec::new();

    // Recurrence loop: startup sweep (if requested) then one sweep per interval tick.
    if let Some(interval) = routine.interval() {
        let sched = scheduler.clone();
        let handle = handle.clone();
        let mut sd = shutdown.clone();
        let startup = routine.run_on_startup();
        children.push(tokio::spawn(async move {
            if startup {
                sched.run_sweep(&handle).await;
            }
            let mut ticker = tokio::time::interval(interval);
            // A slow sweep must not make the next tick fire immediately (burst); delay it instead,
            // matching the old "sleep(interval) after each run" cadence.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await; // consume the immediate first tick
            loop {
                tokio::select! {
                    _ = sd.changed() => { if *sd.borrow() { break; } }
                    _ = ticker.tick() => { sched.run_sweep(&handle).await; }
                }
            }
        }));
    }

    // Trigger loop: schedule each manual trigger.
    {
        let sched = scheduler.clone();
        let mut sd = shutdown.clone();
        children.push(tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = sd.changed() => { if *sd.borrow() { break; } }
                    msg = rx.recv() => match msg {
                        Some((input, debounced)) => sched.schedule(input, debounced),
                        None => break, // all handles dropped — process shutting down
                    }
                }
            }
        }));
    }

    for c in children {
        let _ = c.await;
    }

    // Graceful drain: the loops above no longer schedule new work, so acquiring every permit blocks
    // until all in-flight runs have released theirs.
    let permits = routine.concurrency().max(1) as u32;
    let _ = scheduler.sem.acquire_many(permits).await;

    info!(routine = name, "routine stopped");
}

/// Run one `routine.run(input)` to completion, bypassing the runtime. For tests and standalone calls.
#[allow(dead_code)]
pub async fn run_once<R: Routine>(routine: &R, input: R::Input) -> anyhow::Result<()> {
    routine.run(input).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A routine counting its runs and recording the inputs it ran with. `key_of` projects the input
    /// to the dedup key so tests can exercise both identity-keyed and richer-input routines.
    struct CountingRoutine {
        runs: Arc<AtomicUsize>,
        last_input: Arc<Mutex<u64>>,
        interval: Option<Duration>,
        startup: bool,
        debounce: Duration,
        concurrency: usize,
        run_delay: Duration,
    }

    impl Default for CountingRoutine {
        fn default() -> Self {
            Self {
                runs: Arc::new(AtomicUsize::new(0)),
                last_input: Arc::new(Mutex::new(0)),
                interval: None,
                startup: false,
                debounce: Duration::ZERO,
                concurrency: 4,
                run_delay: Duration::ZERO,
            }
        }
    }

    #[async_trait::async_trait]
    impl Routine for CountingRoutine {
        // (key, payload): key dedups; payload is recorded so last-write-wins is observable.
        type Input = (u64, u64);
        type Key = u64;

        fn name(&self) -> &'static str {
            "counting"
        }
        fn key(input: &Self::Input) -> u64 {
            input.0
        }
        fn interval(&self) -> Option<Duration> {
            self.interval
        }
        fn run_on_startup(&self) -> bool {
            self.startup
        }
        fn debounce(&self) -> Duration {
            self.debounce
        }
        fn concurrency(&self) -> usize {
            self.concurrency
        }
        async fn run(&self, input: Self::Input) -> anyhow::Result<()> {
            if !self.run_delay.is_zero() {
                tokio::time::sleep(self.run_delay).await;
            }
            self.runs.fetch_add(1, Ordering::SeqCst);
            *self.last_input.lock().unwrap() = input.1;
            Ok(())
        }
    }

    fn never_shutdown() -> watch::Receiver<bool> {
        watch::channel(false).1
    }

    // (a) recurring tick on interval
    #[tokio::test]
    async fn ticks_on_interval() {
        let runs = Arc::new(AtomicUsize::new(0));
        let (_h, _join) = spawn(
            CountingRoutine {
                runs: runs.clone(),
                interval: Some(Duration::from_millis(5)),
                ..Default::default()
            },
            never_shutdown(),
        );
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(runs.load(Ordering::SeqCst) >= 2, "expected several sweeps");
    }

    // (b) run_on_startup runs one sweep immediately
    #[tokio::test]
    async fn run_on_startup_sweeps_immediately() {
        let runs = Arc::new(AtomicUsize::new(0));
        let (_h, _join) = spawn(
            CountingRoutine {
                runs: runs.clone(),
                interval: Some(Duration::from_secs(3600)),
                startup: true,
                ..Default::default()
            },
            never_shutdown(),
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    // (c) shutdown stops the loop
    #[tokio::test]
    async fn shutdown_stops_the_loop() {
        let runs = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = watch::channel(false);
        let (_h, join) = spawn(
            CountingRoutine {
                runs: runs.clone(),
                interval: Some(Duration::from_secs(3600)),
                ..Default::default()
            },
            rx,
        );
        tx.send(true).unwrap();
        join.await.unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 0);
    }

    // (d) two triggers with the same key while running ⇒ exactly one rerun
    #[tokio::test]
    async fn same_key_coalesces_to_one_rerun() {
        let runs = Arc::new(AtomicUsize::new(0));
        let (h, _join) = spawn(
            CountingRoutine {
                runs: runs.clone(),
                run_delay: Duration::from_millis(30),
                ..Default::default()
            },
            never_shutdown(),
        );
        h.trigger((1, 0)); // starts run #1
        tokio::time::sleep(Duration::from_millis(5)).await;
        h.trigger((1, 0)); // run in flight → rerun
        h.trigger((1, 0)); // coalesced into the same rerun
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert_eq!(
            runs.load(Ordering::SeqCst),
            2,
            "initial run + exactly one coalesced rerun"
        );
    }

    // (e) two distinct keys run concurrently
    #[tokio::test]
    async fn distinct_keys_run_concurrently() {
        let runs = Arc::new(AtomicUsize::new(0));
        let (h, _join) = spawn(
            CountingRoutine {
                runs: runs.clone(),
                run_delay: Duration::from_millis(40),
                concurrency: 4,
                ..Default::default()
            },
            never_shutdown(),
        );
        h.trigger((1, 0));
        h.trigger((2, 0));
        // If serialized, only one would be done at 40ms+ε; concurrent ⇒ both done by ~60ms.
        tokio::time::sleep(Duration::from_millis(70)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    // (f) debounced burst collapses to one run; (h) last-write-wins on the reran input
    #[tokio::test]
    async fn debounced_burst_collapses_last_write_wins() {
        let runs = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(Mutex::new(0u64));
        let (h, _join) = spawn(
            CountingRoutine {
                runs: runs.clone(),
                last_input: last.clone(),
                debounce: Duration::from_millis(30),
                ..Default::default()
            },
            never_shutdown(),
        );
        h.trigger_debounced((1, 100));
        h.trigger_debounced((1, 200));
        h.trigger_debounced((1, 300));
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1, "burst collapsed to one run");
        assert_eq!(*last.lock().unwrap(), 300, "last write wins");
    }

    // (g) an interactive trigger promotes an open debounce window
    #[tokio::test]
    async fn interactive_promotes_open_window() {
        let runs = Arc::new(AtomicUsize::new(0));
        let (h, _join) = spawn(
            CountingRoutine {
                runs: runs.clone(),
                debounce: Duration::from_secs(3600), // window would never fire on its own
                ..Default::default()
            },
            never_shutdown(),
        );
        h.trigger_debounced((1, 0)); // opens the (very long) window
        tokio::time::sleep(Duration::from_millis(5)).await;
        h.trigger((1, 0)); // interactive → promote now
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            runs.load(Ordering::SeqCst),
            1,
            "promoted, did not wait the window"
        );
    }

    #[tokio::test]
    async fn disconnected_handle_trigger_is_a_noop() {
        RoutineHandle::<()>::disconnected().trigger(());
        RoutineHandle::<()>::disconnected().trigger_debounced(());
    }
}
