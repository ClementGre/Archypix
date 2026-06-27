# Unified Routine Framework

## 1. Overview

The backend runs background work through **four** overlapping, hand-rolled mechanisms:

| Mechanism                              | File                  | Keyed?      | Recurring      | Startup   | Debounce  | Dedup + rerun     | Manual trigger |
|----------------------------------------|-----------------------|-------------|----------------|-----------|-----------|-------------------|----------------|
| `Scheduler` / `RecurringTask`          | `infra/scheduler.rs`  | no (global) | ✅ interval     | ✅ flag    | ❌         | ❌                 | ❌              |
| `TaskQueue` / `InternalTask`           | `infra/tasks.rs`      | no          | ❌              | ❌         | ❌         | ❌                 | ✅ `enqueue`    |
| `PipelineWaker` + per-user `Scheduler` | `infra/pipeline.rs`   | ✅ per-user  | via sweep task | via sweep | ✅ per-key | ✅                 | ✅ `wake`       |
| `ExifDrainWaker`                       | `infra/exif_drain.rs` | no          | ✅ interval     | implicit  | ❌         | (global, trivial) | ✅ `wake`       |

The tagging pipeline is the **superset**: every other mechanism is the pipeline with a degenerate
key (`()`) and/or features switched off. This feature lifts the pipeline's per-key
debounce/coalesce/rerun scheduler (`infra/pipeline.rs` §`Scheduler`) into **one generic runtime**,
the **Routine framework**, and migrates all four mechanisms onto it.

"Routine" is the chosen name — `Task` collides with `InternalTask` and worker `Job`s; the legacy
`InternalTask` enum is merged away here, but `Routine` keeps it unambiguous.

### 1.1 What a Routine is

A **Routine** is a named unit of background work that can be triggered three ways:

1. **Recurrently** — every `interval()`, if set.
2. **On startup** — one sweep before the first interval, if `run_on_startup()`.
3. **Manually** — `handle.trigger(input)` / `handle.trigger_debounced(input)` from anywhere.

Triggers carry an **input** (the run payload). A **dedup key** is *derived* from the input
(`Routine::key`). Two triggers whose keys are equal never run concurrently: while a key is running, a
new trigger sets a **rerun** flag (storing the latest input), and the runtime re-runs once at the end
— the same debounce-by-self mechanism the pipeline uses today, now generic. A debounce window
(configurable per routine) coalesces bursts *before* the first run.

### 1.2 Non-goals

- **Durable queue.** Triggers are in-memory (`mpsc`); a crash drops queued triggers. Correctness must
  come from the recurring `sweep` re-deriving outstanding work from the DB (§4). Routines with no
  sweep (e.g. tag rename) are best-effort — this matches the status quo and is **not** changed here.
- **Generic retry/backoff.** `rerun` re-runs because new triggers arrived, never because a run
  *failed*. Failure handling stays in each routine's own DB state (e.g. the pipeline's `errored`
  share status + backoff). The framework only logs failures and continues.
- **Cross-routine ordering or priority.** Routines are independent; within a key, the only guarantee
  is "at least one more run after the latest trigger."

## 2. Design

### 2.1 The trait (`infra/routine.rs`)

```rust
#[async_trait::async_trait]
pub trait Routine: Send + Sync + 'static {
    /// The run payload, carried by every trigger. `Default` is used by the periodic/startup
    /// sweep's default impl. For the common case Input == Key (e.g. a `Uuid` user id).
    type Input: Clone + std::fmt::Debug + Default + Send + Sync + 'static;

    /// Dedup key, *derived* from the input. Equal keys never run concurrently; a trigger whose key
    /// is already running stores its (latest) input as the pending rerun. Callers never pass a key
    /// directly — the framework computes it from the input.
    type Key: Clone + Eq + std::hash::Hash + Send + Sync + 'static;

    /// Stable lower-snake name for logs/metrics, e.g. "pipeline", "job_cleanup".
    fn name(&self) -> &'static str;

    /// Derive the dedup key from the input. Simple routines set `type Key = Self::Input` and return
    /// `input.clone()` (identity).
    fn key(input: &Self::Input) -> Self::Key;

    /// Recurring cadence; `None` = trigger-only (no periodic sweep).
    fn interval(&self) -> Option<std::time::Duration> { None }

    /// Run one sweep at startup before the first interval sleep.
    fn run_on_startup(&self) -> bool { false }

    /// Debounce window for `trigger_debounced`; `ZERO` disables (every trigger runs promptly).
    fn debounce(&self) -> std::time::Duration { std::time::Duration::ZERO }

    /// Max runs in flight across *distinct* keys. Per-key is always serial regardless. `()`-keyed
    /// routines are implicitly 1 (only one key exists); the pipeline uses `PIPELINE_CONCURRENCY`.
    fn concurrency(&self) -> usize { 1 }

    /// The periodic/startup action: enqueue the inputs that need a run. The default triggers the
    /// `Default` input once (correct for `()`-keyed routines). Keyed routines override to
    /// *enumerate* the work from the DB (e.g. all users with dirty pictures) and trigger each.
    async fn sweep(&self, h: &RoutineHandle<Self::Input>) -> anyhow::Result<()> {
        h.trigger(Self::Input::default());
        Ok(())
    }

    /// Execute one run for `input`. Errors are logged, not propagated.
    async fn run(&self, input: Self::Input) -> anyhow::Result<()>;
}
```

> `async-trait` is already a dependency and the established pattern (`infra/s3.rs`, `infra/redis.rs`).

**Why `sweep`, not "run with `Default` input".** The naïve reading of "the input impls `Default` for
the initial run" breaks for keyed routines: the pipeline's recurrence is *not* "run for the default
user" — it enumerates **every dirty user** and triggers each. A `Default` user id (nil `Uuid`) would
be a footgun. `sweep` reconciles the two: the periodic action is "enqueue the inputs needing work,"
whose default impl is `trigger(Default)` (right for `()`), and which keyed routines override to
enumerate. `Default` on `Input` therefore only exists to serve the default `sweep` — it is never used
by the pipeline.

**Why a derived key (not a separate wake parameter).** The dedup key is always derivable from the
call's parameters, so callers pass only the input; `Routine::key` projects it to the key. Where there
is no extra payload (pipeline: `Input = Key = Uuid`), `key` is identity. Where the input is richer
than the key, coalescing keeps the **latest** input for the pending rerun (last-write-wins). Because
every routine today **re-derives its actual work from the DB inside `run`** (the pipeline recomputes
dirty pictures; the drain re-queries `pending_job_creation`), no information is lost when triggers
coalesce — the input is at most a hint. A routine that genuinely cannot re-derive from the DB must
encode everything it needs in `Input` and tolerate last-write-wins.

### 2.2 The handle

```rust
/// Cheaply-cloneable trigger handle. Stored in `AppState`; call from request handlers, other
/// routines, the task runner, anywhere.
#[derive(Clone)]
pub struct RoutineHandle<I> {
    tx: mpsc::UnboundedSender<(I, bool)>
} // bool = debounced

impl<I> RoutineHandle<I> {
    /// Trigger promptly (no debounce). Silently no-ops if the runtime has shut down — recovered by
    /// the next sweep.
    pub fn trigger(&self, input: I);
    /// Trigger through the debounce window. An interactive `trigger` arriving mid-window promotes the
    /// run to start immediately.
    pub fn trigger_debounced(&self, input: I);
    /// A handle attached to no runtime; wakes are discarded. For tests and standalone calls.
    pub fn disconnected() -> Self;
}
```

This unifies `PipelineWaker::{wake, wake_debounced, disconnected}` and `ExifDrainWaker::{wake,
disconnected}` and `TaskQueue::enqueue` into one type.

### 2.3 The runtime

```rust
/// Spawn the runtime onto the current Tokio runtime; return the trigger handle + the runtime's
/// `JoinHandle`. `main` keeps the join handles to drain in-flight runs on shutdown. The pipeline's
/// handle ↔ runtime cycle is broken by wiring its own handle into it (interior mutability) right
/// after `spawn` returns, before the runtime issues its first run.
pub fn spawn<R: Routine>(routine: R, shutdown: watch::Receiver<bool>)
                         -> (RoutineHandle<R::Input>, tokio::task::JoinHandle<()>);
```

The runtime future:

- Holds `Arc<R>`, a `Semaphore(concurrency())`, and `Mutex<HashMap<R::Key, RunState<R::Input>>>` —
  the per-key scheduler lifted **verbatim** from `infra/pipeline.rs` (`Phase::{Pending, Running}`,
  `rerun`, `immediate`, the debounce timer, the post-run settle). The only change is that the map is
  keyed on `R::Key` and `RunState` stores the latest `R::Input` for a pending rerun.
- Drives an `mpsc::recv()` loop scheduling each `(input, debounced)` trigger.
- If `interval().is_some()`, also runs a recurrence loop that calls `sweep(&handle)` every interval
  and once at startup when `run_on_startup()`. Sweep-issued triggers are **non-debounced** (they must
  always run promptly — matches the pipeline's `wake`-in-sweep today).
- Breaks both loops when `shutdown` flips to `true`, then drains in-flight runs (acquires every
  semaphore permit) before the runtime future resolves. `main` wires graceful shutdown: a
  SIGINT/SIGTERM future drives `axum`'s `with_graceful_shutdown`; once the server stops, `main` flips
  `shutdown_tx` and awaits every routine's `JoinHandle`.

A run is wrapped in a span `routine_run { routine = name, key = ?key, run_id }` (requires
`Key: Debug` — add the bound, or format the input). Failures: `error!(routine, error, "run failed")`,
loop continues.

### 2.4 Registry / wiring

Routines have differently-typed handles, so they cannot be stored behind one `dyn`. Bundle them:

```rust
// state.rs
pub struct Routines {
    pub pipeline: RoutineHandle<Uuid>,
    pub exif_drain: RoutineHandle<()>,
    pub tag_rename: RoutineHandle<TagRenameInput>,
    pub unannounce: RoutineHandle<UnannounceInput>,
    // job_watchdog / job_cleanup / purge_sweep need no handle (sweep-only, never triggered manually)
}
```

`AppState` holds `routines: Routines`, replacing today's `pipeline_waker`, `exif_drain`, and
`task_queue` fields. `main` calls `spawn` for each routine (which spawns the runtime and returns the
handle + `JoinHandle`), collecting the handles into `Routines` and the join handles for graceful
shutdown.

## 3. Migrating each mechanism

| Routine       | `Input`                      | `Key`             | `interval`                    | startup | `debounce`             | `concurrency`              | `sweep`                                            |
|---------------|------------------------------|-------------------|-------------------------------|---------|------------------------|----------------------------|----------------------------------------------------|
| `Pipeline`    | `Uuid`                       | `Uuid` (identity) | `PIPELINE_POLL_INTERVAL_SECS` | yes     | `PIPELINE_DEBOUNCE_MS` | `PIPELINE_CONCURRENCY` (4) | enumerate dirty + dedup-needing users              |
| `ExifDrain`   | `()`                         | `()`              | `EXIF_DRAIN_INTERVAL_SECS`    | yes     | 0                      | 1                          | default (`trigger(())`) — `run` drains until empty |
| `JobWatchdog` | `()`                         | `()`              | `JOB_WATCHDOG_INTERVAL_SECS`  | no      | 0                      | 1                          | default                                            |
| `JobCleanup`  | `()`                         | `()`              | `JOB_CLEANUP_INTERVAL_SECS`   | no      | 0                      | 1                          | default                                            |
| `PurgeSweep`  | `()`                         | `()`              | `PURGE_SWEEP_INTERVAL_SECS`   | no      | 0                      | 1                          | default                                            |
| `TagRename`   | `{user_id, old, new}`        | same              | `None`                        | no      | 0                      | `TASK_QUEUE_CONCURRENCY`   | none (trigger-only)                                |
| `Unannounce`  | `{share_id, …, picture_ids}` | same              | `None`                        | no      | 0                      | `TASK_QUEUE_CONCURRENCY`   | none (trigger-only)                                |

Notes:

- **Pipeline.** `run(user_id)` = `evaluation::run_for_user`. The runtime *is* its current
  `Scheduler`, so this is near-mechanical. `sweep` reproduces `PipelineRecoverySweepTask::tick`:
  re-dirty announce-stale rows, then trigger every user from
  `find_users_with_dirty_pictures` ∪ `find_users_needing_reconcile`. The standalone
  `PipelineRecoverySweepTask` and the separate `PipelineWaker` both disappear into the routine.
- **ExifDrain.** `run` keeps `drain_until_empty`. The `interval`/`Notify` loop becomes the generic
  runtime; `ExifDrainWaker` becomes `RoutineHandle<()>`.
- **Job tasks + purge.** These are the current `RecurringTask` impls verbatim, now `Routine`s with
  `Input = ()` and the default `sweep` (the body of today's `tick` moves into `run`).
- **TagRename / Unannounce.** Each `InternalTask` variant becomes its own `Routine`. The variant's
  payload struct is the `Input` **and** the `Key` (`Eq + Hash` derive). `interval = None`,
  `run_on_startup = false` — pure manual-trigger. `key` is identity. The whole `InternalTask` enum
  and the `execute_task` dispatch in `infra/tasks.rs` are deleted; `TaskQueue::enqueue(InternalTask::X
  {..})` becomes `routines.x.trigger(XInput {..})`.

**Coalescing semantics for the merged one-shots.** Two *distinct* tag renames have distinct keys →
both run. Two *identical* renames in flight collapse to one rerun (idempotent — harmless/better than
today's double-run). A rename arriving while an identical one runs serialises behind it instead of
racing it. Acceptable; call it out in the doc/PR.

## 4. Durability caveat (decide explicitly)

`Pipeline`, `ExifDrain`, and the job/purge sweeps are crash-safe: their `sweep` re-derives all
outstanding work from the DB, so a dropped in-memory trigger is only a latency hit. `TagRename` and
`Unannounce` have **no sweep** — a trigger lost to a crash is lost (exactly today's `TaskQueue`
behaviour; `TagRename`'s `run` is still `todo!()`). This feature **preserves** that; making
tag-rename durable (a DB outbox row + a `sweep` that re-derives pending renames) is a noted follow-up,
out of scope. State the invariant in the code doc: *a routine needing crash-safety must provide a
`sweep` that re-derives its work from persistent state.*

## 5. Config

No new env vars and **no renames** — each routine reads its existing knobs in its constructor
(`PIPELINE_*`, `EXIF_DRAIN_*`, `JOB_*`, `PURGE_SWEEP_*`, `TASK_QUEUE_CONCURRENCY`). The framework
itself is config-free. `PIPELINE_DEBOUNCE_MS=0` (test default) keeps disabling debounce.

## 6. Testing

- **Framework unit tests** (`infra/routine.rs`): a `CountingRoutine<Key>` over `AtomicUsize` asserting
  (a) recurring tick on interval, (b) `run_on_startup` immediate sweep, (c) shutdown stops the loop
  (port the existing `scheduler.rs` tests), plus the keyed behaviours that only the pipeline covered
  today: (d) two triggers with the **same** key while running ⇒ exactly one rerun, (e) two **distinct**
  keys run concurrently up to `concurrency`, (f) debounced burst collapses to one run, (g) an
  interactive trigger promotes an open debounce window, (h) last-write-wins on the reran input.
  Use `tokio::time::pause`/`advance` for determinism.
- **Per-routine tests** stay as-is — they target `run`/`sweep`, which keep the existing bodies. Reuse
  the pipeline harness (mark dirty → `sweep` delivers the user → run reconciles). Generalise
  `pipeline::run_once_for_user` into a `routine::run_once(routine, input)` helper.
- `disconnected()` handle test (port from `exif_drain.rs`).

## 7. Migration order (de-risked)

1. `infra/routine.rs`: trait + handle + runtime + tests. Register in `infra.rs`.
2. Port the `()`-keyed routines first (`ExifDrain`, `JobWatchdog`, `JobCleanup`, `PurgeSweep`) —
   proves the default-`sweep` / `()`-key path.
3. Port `Pipeline` (`Key = Uuid`) — the runtime is literally its current scheduler; keep
   `evaluation::run_for_user`. Delete `PipelineWaker`, `PipelineRecoverySweepTask`, and the bespoke
   `Scheduler` in `infra/pipeline.rs`.
4. Port `TagRename` / `Unannounce`; delete the `InternalTask` enum, `TaskRunner`, and `execute_task`.
5. Delete `infra/scheduler.rs`, `infra/exif_drain.rs`'s waker, and `infra/tasks.rs`. Update
   `AppState` (`routines: Routines`) and `main.rs` (build handles → assemble → spawn runtimes).

## 8. Documentation updates

- `doc/03_BACKEND_ARCHITECTURE.md` — module-layout block: replace `scheduler.rs` / `tasks.rs` /
  `exif_drain.rs` / the pipeline's bespoke scheduler with `routine.rs` + the per-routine files; update
  the `AppState` struct (drop `task_queue`/`pipeline_waker`, add `routines`).
- `doc/02_INFRASTRUCTURE_DESIGN.md` — the backend "in-process task queue" + "recurring scheduler"
  bullets collapse into one "Routine framework" bullet.
- `doc/features/03_recurring_tasks_framework.md` — already marked superseded.
- Cross-references in features 02 (pipeline robustness), 11 (dedup reconciler), 14 (exif drain) that
  name `PipelineWaker` / `ExifDrainWaker` / `RecurringTask` — repoint to `RoutineHandle` / `Routine`.

## 9. Open questions / edge cases to confirm during implementation

- **`Key: Debug` bound** — needed for the run span. Add it, or format the input instead and drop the
  bound. (Pipeline `Uuid` and the payload structs all derive `Debug` already.)
- **Boilerplate for `Input == Key`** — Rust can't default an associated type, so simple routines write
  `type Key = Self::Input;` + identity `key`. If this proves noisy, add a `KeyedByValue` blanket-style
  helper macro. Not worth it up front.
- **Recurrence loop vs scheduler loop** — one spawned future with two internal child tasks (recv loop
    + interval loop), or `tokio::select!` in one loop. Either works; the child-task split is closer to
      the current code.
- **`concurrency()` for one-shots** — `TagRename`/`Unannounce` inherit the old `TASK_QUEUE_CONCURRENCY`
  (4) so unrelated renames still parallelise; per-key serialisation still prevents identical races.
