//! Generic background-work runtime — the **Routine framework** (feature 17, lifted to `common` in
//! feature 23 §8).
//!
//! A [`Routine`] is a named unit of background work triggerable three ways: recurrently (every
//! [`interval`](Routine::interval)), at startup ([`run_on_startup`](Routine::run_on_startup)), and
//! manually ([`RoutineHandle::trigger`]). Every trigger carries an [`Input`](Routine::Input); a
//! dedup [`Key`](Routine::Key) is *derived* from it. Equal keys never run concurrently — while a key
//! is running a new trigger sets a **rerun** flag (storing the latest input), and the runtime
//! re-runs once at the end. A debounce window coalesces a burst *before* the first run.
//!
//! [`interval`](Routine::interval) and [`debounce`](Routine::debounce) are read **live** (each tick /
//! each schedule decision) so a routine backed by a runtime-config snapshot picks up a changed value
//! after the current wait ends, without a re-spawn (feature 23 §4.4). `concurrency` and
//! `run_on_startup` are read once at [`spawn`].
//!
//! **Durability.** Triggers are in-memory (`mpsc`); a crash drops queued triggers. A routine needing
//! crash-safety must provide a [`sweep`](Routine::sweep) that re-derives its outstanding work from
//! persistent state. See `doc/features/17_unified_routine_framework.md`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Semaphore, mpsc, watch};
use tracing::{Instrument, error, info};
use uuid::Uuid;

// ── Trait ─────────────────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait Routine: Send + Sync + 'static {
    /// The run payload, carried by every trigger. `Default` only serves the default [`sweep`](Self::sweep)
    /// and the dashboard's manual trigger ([`TriggerAny`]).
    type Input: Clone + std::fmt::Debug + Default + Send + Sync + 'static;

    /// Dedup key, *derived* from the input via [`key`](Self::key). Equal keys never run concurrently.
    type Key: Clone + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static;

    /// Stable lower-snake name for logs/metrics, e.g. `"pipeline"`, `"job_cleanup"`.
    fn name(&self) -> &'static str;

    /// Derive the dedup key from the input. Simple routines set `type Key = Self::Input` and return
    /// `input.clone()` (identity).
    fn key(input: &Self::Input) -> Self::Key;

    /// Recurring cadence; `None` = trigger-only (no periodic sweep). Read live each tick.
    fn interval(&self) -> Option<Duration> {
        None
    }

    /// Run one sweep at startup before the first interval sleep. Read once at [`spawn`].
    fn run_on_startup(&self) -> bool {
        false
    }

    /// Debounce window for [`trigger_debounced`](RoutineHandle::trigger_debounced); `ZERO` disables.
    /// Read live at each schedule decision.
    fn debounce(&self) -> Duration {
        Duration::ZERO
    }

    /// Max runs in flight across *distinct* keys. Per-key is always serial. Read once at [`spawn`].
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
/// routines, anywhere.
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

/// Type-erased "trigger with the default input", so the admin dashboard can trigger any routine by
/// name without knowing its `Input` type (feature 23 §8.2 refinement).
pub trait TriggerAny: Send + Sync {
    fn trigger_default(&self);
}

impl<I: Default + Send + Sync + 'static> TriggerAny for RoutineHandle<I> {
    fn trigger_default(&self) {
        self.trigger(I::default());
    }
}

// ── Monitoring ──────────────────────────────────────────────────────────────────

/// Live status of a routine, updated by the runtime and read by the admin API (feature 23 §5.2).
#[derive(Clone, Default)]
pub struct RoutineStatus {
    inner: Arc<Mutex<StatusInner>>,
}

#[derive(Default, Clone)]
struct StatusInner {
    last_started_at: Option<i64>,
    last_finished_at: Option<i64>,
    last_error: Option<String>,
    in_flight: usize,
    total_runs: u64,
}

/// A point-in-time snapshot of a [`RoutineStatus`] (Unix-second timestamps).
#[derive(Clone)]
pub struct RoutineStatusSnapshot {
    pub last_started_at: Option<i64>,
    pub last_finished_at: Option<i64>,
    pub last_error: Option<String>,
    pub in_flight: usize,
    pub total_runs: u64,
}

impl RoutineStatus {
    pub fn snapshot(&self) -> RoutineStatusSnapshot {
        let g = self.inner.lock().expect("routine status mutex poisoned");
        RoutineStatusSnapshot {
            last_started_at: g.last_started_at,
            last_finished_at: g.last_finished_at,
            last_error: g.last_error.clone(),
            in_flight: g.in_flight,
            total_runs: g.total_runs,
        }
    }

    fn mark_started(&self) {
        let mut g = self.inner.lock().expect("routine status mutex poisoned");
        g.in_flight += 1;
        g.last_started_at = Some(now_secs());
    }

    fn mark_finished(&self, error: Option<String>) {
        let mut g = self.inner.lock().expect("routine status mutex poisoned");
        g.in_flight = g.in_flight.saturating_sub(1);
        g.last_finished_at = Some(now_secs());
        g.total_runs += 1;
        g.last_error = error;
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ── Runtime ───────────────────────────────────────────────────────────────────

struct RunState<I> {
    phase: Phase,
    rerun: bool,
    immediate: bool,
    next_input: I,
}

enum Phase {
    Pending,
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

enum Action {
    None,
    Run,
    Timer,
}

struct Scheduler<R: Routine> {
    routine: Arc<R>,
    sem: Arc<Semaphore>,
    state: Mutex<HashMap<R::Key, RunState<R::Input>>>,
    status: RoutineStatus,
}

impl<R: Routine> Scheduler<R> {
    /// Current debounce window — read live so a runtime-config change takes effect immediately.
    fn debounce(&self) -> Duration {
        self.routine.debounce()
    }

    /// Ensure a run is (or will be) scheduled for `input`'s key, coalescing concurrent triggers.
    fn schedule(self: &Arc<Self>, input: R::Input, debounced: bool) {
        let key = R::key(&input);
        let immediate = !debounced || self.debounce().is_zero();
        let action = {
            let mut map = self.state.lock().expect("routine scheduler mutex poisoned");
            match map.get_mut(&key) {
                Some(s) => {
                    s.next_input = input;
                    match s.phase {
                        Phase::Pending if immediate => {
                            s.phase = Phase::Running;
                            Action::Run
                        }
                        Phase::Pending => Action::None,
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

    fn spawn_debounce_timer(self: &Arc<Self>, key: R::Key) {
        let this = Arc::clone(self);
        let delay = this.debounce();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            {
                let mut map = this.state.lock().expect("routine scheduler mutex poisoned");
                match map.get_mut(&key) {
                    Some(s) if matches!(s.phase, Phase::Pending) => s.phase = Phase::Running,
                    _ => return,
                }
            }
            this.spawn_run(key);
        });
    }

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
                        None => return,
                    }
                };
                let run_id = Uuid::new_v4();
                let span = tracing::info_span!(
                    "routine_run",
                    routine = this.routine.name(),
                    key = ?key,
                    run_id = %run_id
                );
                this.status.mark_started();
                let result = this.routine.run(input).instrument(span).await;
                let err_msg = match &result {
                    Ok(()) => None,
                    Err(e) => {
                        error!(routine = this.routine.name(), error = ?e, "routine run failed");
                        Some(format!("{e:#}"))
                    }
                };
                this.status.mark_finished(err_msg);
                drop(permit);

                let action = {
                    let mut map = this.state.lock().expect("routine scheduler mutex poisoned");
                    match map.get_mut(&key) {
                        Some(s) if s.rerun => {
                            let run_now = s.immediate || this.debounce().is_zero();
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

    async fn run_sweep(self: &Arc<Self>, handle: &RoutineHandle<R::Input>) {
        let run_id = Uuid::new_v4();
        let span =
            tracing::info_span!("routine_sweep", routine = self.routine.name(), run_id = %run_id);
        if let Err(e) = self.routine.sweep(handle).instrument(span).await {
            error!(routine = self.routine.name(), error = ?e, "routine sweep failed");
        }
    }
}

/// Spawn the routine's runtime and return its trigger handle plus the runtime's
/// [`JoinHandle`](tokio::task::JoinHandle). Uses a throwaway [`RoutineStatus`]; use
/// [`spawn_with_status`] to keep a status handle for monitoring.
pub fn spawn<R: Routine>(
    routine: R,
    shutdown: watch::Receiver<bool>,
) -> (RoutineHandle<R::Input>, tokio::task::JoinHandle<()>) {
    let (handle, _status, join) = spawn_with_status(routine, RoutineStatus::default(), shutdown);
    (handle, join)
}

/// Like [`spawn`] but threads a caller-owned [`RoutineStatus`] so the admin API can read last-run /
/// in-flight / last-error state (feature 23 §5.2).
pub fn spawn_with_status<R: Routine>(
    routine: R,
    status: RoutineStatus,
    shutdown: watch::Receiver<bool>,
) -> (
    RoutineHandle<R::Input>,
    RoutineStatus,
    tokio::task::JoinHandle<()>,
) {
    let (tx, rx) = mpsc::unbounded_channel::<(R::Input, bool)>();
    let handle = RoutineHandle { tx };
    let runtime_handle = handle.clone();
    let join = tokio::spawn(run_routine(
        Arc::new(routine),
        runtime_handle,
        rx,
        shutdown,
        status.clone(),
    ));
    (handle, status, join)
}

/// Resolve only on a real shutdown (`shutdown` set to `true`). If every sender is dropped (the
/// channel closes, e.g. in tests), park **forever** so a `select!` racing this against a timer always
/// falls through to the timer instead of busy-looping on the closed channel's immediate `Err`.
async fn wait_for_shutdown(sd: &mut watch::Receiver<bool>) {
    loop {
        if *sd.borrow_and_update() {
            return;
        }
        if sd.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

async fn run_routine<R: Routine>(
    routine: Arc<R>,
    handle: RoutineHandle<R::Input>,
    mut rx: mpsc::UnboundedReceiver<(R::Input, bool)>,
    shutdown: watch::Receiver<bool>,
    status: RoutineStatus,
) {
    let name = routine.name();
    info!(routine = name, "routine started");

    let scheduler = Arc::new(Scheduler {
        sem: Arc::new(Semaphore::new(routine.concurrency().max(1))),
        state: Mutex::new(HashMap::new()),
        status,
        routine: routine.clone(),
    });

    let mut children = Vec::new();

    // Recurrence loop: startup sweep (if requested) then one sweep per interval tick. The interval is
    // read fresh each iteration so a runtime-config change takes effect after the current wait.
    if routine.interval().is_some() {
        let sched = scheduler.clone();
        let handle = handle.clone();
        let mut sd = shutdown.clone();
        let routine = routine.clone();
        let startup = routine.run_on_startup();
        children.push(tokio::spawn(async move {
            if startup {
                sched.run_sweep(&handle).await;
            }
            // The interval is re-read each iteration so a runtime-config change takes effect after
            // the current wait, without a re-spawn (feature 23 §4.4).
            loop {
                let delay = routine.interval().unwrap_or(Duration::from_secs(3600));
                tokio::select! {
                    _ = wait_for_shutdown(&mut sd) => break,
                    _ = tokio::time::sleep(delay) => { sched.run_sweep(&handle).await; }
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
                    _ = wait_for_shutdown(&mut sd) => break,
                    msg = rx.recv() => match msg {
                        Some((input, debounced)) => sched.schedule(input, debounced),
                        None => break,
                    }
                }
            }
        }));
    }

    for c in children {
        let _ = c.await;
    }

    // Graceful drain: acquiring every permit blocks until all in-flight runs release theirs.
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
        h.trigger((1, 0));
        tokio::time::sleep(Duration::from_millis(5)).await;
        h.trigger((1, 0));
        h.trigger((1, 0));
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert_eq!(
            runs.load(Ordering::SeqCst),
            2,
            "initial run + exactly one coalesced rerun"
        );
    }

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
        tokio::time::sleep(Duration::from_millis(70)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

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

    #[tokio::test]
    async fn interactive_promotes_open_window() {
        let runs = Arc::new(AtomicUsize::new(0));
        let (h, _join) = spawn(
            CountingRoutine {
                runs: runs.clone(),
                debounce: Duration::from_secs(3600),
                ..Default::default()
            },
            never_shutdown(),
        );
        h.trigger_debounced((1, 0));
        tokio::time::sleep(Duration::from_millis(5)).await;
        h.trigger((1, 0));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            runs.load(Ordering::SeqCst),
            1,
            "promoted, did not wait the window"
        );
    }

    #[tokio::test]
    async fn status_records_runs() {
        let runs = Arc::new(AtomicUsize::new(0));
        let (h, status, _join) = spawn_with_status(
            CountingRoutine {
                runs: runs.clone(),
                ..Default::default()
            },
            RoutineStatus::default(),
            never_shutdown(),
        );
        h.trigger((1, 0));
        tokio::time::sleep(Duration::from_millis(30)).await;
        let snap = status.snapshot();
        assert_eq!(snap.total_runs, 1);
        assert_eq!(snap.in_flight, 0);
        assert!(snap.last_error.is_none());
    }

    #[tokio::test]
    async fn disconnected_handle_trigger_is_a_noop() {
        RoutineHandle::<()>::disconnected().trigger(());
        RoutineHandle::<()>::disconnected().trigger_debounced(());
    }

    #[tokio::test]
    async fn trigger_any_uses_default_input() {
        let runs = Arc::new(AtomicUsize::new(0));
        let (h, _join) = spawn(
            CountingRoutine {
                runs: runs.clone(),
                ..Default::default()
            },
            never_shutdown(),
        );
        let erased: &dyn TriggerAny = &h;
        erased.trigger_default();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }
}
