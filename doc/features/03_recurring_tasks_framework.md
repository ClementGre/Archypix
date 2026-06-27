# Recurring Background-Task Framework

> **Superseded by [`17_unified_routine_framework.md`](17_unified_routine_framework.md).** This
> document describes the shipped v1 (`RecurringTask` + `Scheduler`). Feature 17 generalises it into
> a single keyed `Routine` framework that also absorbs `infra/tasks.rs`, `infra/pipeline.rs`'s
> per-user scheduler, and `infra/exif_drain.rs`. Kept for history.

## 1. Overview

The backend runs several **periodic** background loops. Today each one is hand-rolled:

- `infra/job_watchdog.rs` — `loop { sleep(interval); reset_stale }`.
- `infra/pipeline.rs` — the tagging-pipeline loop bakes a poll fallback into its
  `select!` (a `sleep(poll_interval)` arm that re-runs `recovery_sweep`), plus a one-shot
  `recovery_sweep` at startup.

Two more periodic needs are imminent:

- **Job cleanup** — prune terminal (`completed` / `failed`) job rows after a retention
  window (default 30 days) so the `jobs` table does not grow without bound. (Every upload
  already creates a `gen_thumbnail` job; EXIF/visual edits add more.)
- (Already exists, to be migrated) the **pipeline recovery sweep**.

This feature introduces **one small framework in `infra`** — a `RecurringTask` trait plus
a `Scheduler` — and migrates the three periodic behaviours onto it. It is **purely a
refactor + one new task (job cleanup)**: no externally observable behaviour changes except
that terminal jobs are now pruned.

### Non-goals / scope boundaries

- This is **not** the one-shot `infra/tasks.rs` `TaskQueue` (ad-hoc, event-triggered work
  like tag rename / unannounce). Leave `TaskQueue` untouched; the two coexist. The
  distinction: `TaskQueue` = "do this once, now, off the request path"; `Scheduler` = "do
  this every N seconds, forever."
- The **tagging-pipeline loop itself stays** (it is event-driven via `PipelineWaker`).
  Only its *poll fallback* and *startup sweep* move into the framework.

## 2. Design

### 2.1 New module: `infra/scheduler.rs`

Register it in `infra.rs` (`pub mod scheduler;`). Follow the repo convention: a
`scheduler.rs` file (no `mod.rs`).

```rust
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tracing::{debug, error, info};

/// A unit of periodic background work. One implementor per behaviour; each carries its
/// own dependencies (db pool, config values, wake handles, …).
///
/// Implementors run **serially with themselves** (the next tick starts only after the
/// previous one returns) and **concurrently with other tasks** (each gets its own
/// spawned loop). A failing tick is logged and never aborts the loop.
#[async_trait::async_trait]
pub trait RecurringTask: Send + Sync + 'static {
    /// Stable, lower-snake name for logs/metrics, e.g. "job_watchdog".
    fn name(&self) -> &'static str;

    /// Delay between the end of one tick and the start of the next.
    fn interval(&self) -> Duration;

    /// When `true`, run one tick immediately at startup before the first interval sleep.
    /// (The pipeline recovery sweep needs this; the watchdog and cleanup do not.)
    fn run_on_startup(&self) -> bool {
        false
    }

    /// Execute one iteration. Errors are logged, not propagated.
    async fn tick(&self) -> anyhow::Result<()>;
}
```

> `async-trait = "0.1"` is already a dependency (`back/Cargo.toml`) and is the
> established pattern for async traits here (`infra/s3.rs`, `infra/redis.rs`) — use it for
> consistency. No `Cargo.toml` change needed.

### 2.2 The scheduler

```rust
pub struct Scheduler {
    tasks: Vec<Arc<dyn RecurringTask>>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self { tasks: Vec::new() }
    }

    /// Register a task. Chainable.
    pub fn register(&mut self, task: Arc<dyn RecurringTask>) -> &mut Self {
        self.tasks.push(task);
        self
    }

    /// Spawn one loop per registered task. Returns a future that resolves when all loops
    /// have stopped (after `shutdown` fires). Spawn it with `tokio::spawn`.
    ///
    /// `shutdown`: a `watch` receiver; when its value flips to `true`, every loop breaks
    /// after its current tick. Pass a never-firing receiver to run until process exit.
    pub fn run(self, shutdown: watch::Receiver<bool>) -> impl std::future::Future<Output=()> {
        async move {
            let mut handles = Vec::new();
            for task in self.tasks {
                let sd = shutdown.clone();
                handles.push(tokio::spawn(run_task(task, sd)));
            }
            for h in handles {
                let _ = h.await;
            }
        }
    }
}

async fn run_task(task: Arc<dyn RecurringTask>, mut shutdown: watch::Receiver<bool>) {
    let name = task.name();
    let interval = task.interval();
    info!(task = name, interval_secs = interval.as_secs(), "recurring task started");

    if task.run_on_startup() {
        run_once(&task).await;
    }

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() { break; }
            }
            _ = tokio::time::sleep(interval) => {
                run_once(&task).await;
            }
        }
    }
    info!(task = name, "recurring task stopped");
}

async fn run_once(task: &Arc<dyn RecurringTask>) {
    let start = Instant::now();
    match task.tick().await {
        Ok(()) => debug!(task = task.name(), elapsed_ms = start.elapsed().as_millis() as u64, "tick ok"),
        Err(e) => error!(task = task.name(), error = ?e, "tick failed"),
    }
}
```

**Shutdown.** Use `tokio::sync::watch::<bool>` (no new dependency — `watch` is part of
`tokio`). `main` holds the `Sender`; today nothing triggers graceful shutdown, so `main`
may simply create the channel, pass the receiver, and never send — preserving current
"run until process exit" behaviour. Wiring the sender to a signal handler is a later,
optional improvement and out of scope here.

**Overlap safety.** Because each loop `await`s `tick()` to completion before sleeping
again, a slow tick can never overlap itself; it just delays the next run. This matches
the current watchdog/pipeline behaviour.

## 3. The three tasks

Each lives next to its domain logic (not all crammed into `scheduler.rs`). Keep the
existing repository/sweep functions; the task types are thin adapters.

### 3.1 `JobWatchdogTask` (migrate)

Replaces `infra/job_watchdog.rs::run`. Put the task struct in `infra/job_watchdog.rs`
(keep the file; drop the bespoke `run` loop).

```rust
pub struct JobWatchdogTask {
    db: PgPool,
    timeout_secs: i64,
    interval: Duration,
}
// name() = "job_watchdog"; interval() = self.interval; run_on_startup() = false
// tick(): JobRepository::reset_stale(&self.db, self.timeout_secs).await?; log count.
```

Behaviour identical to today (`reset_stale`, `job_processing_timeout_secs`,
`job_watchdog_interval_secs`). The "reset N stale jobs" info log moves into `tick`.

### 3.2 `JobCleanupTask` (new)

Lives in `infra/job_watchdog.rs` (or a new `infra/job_maintenance.rs` if preferred —
keep watchdog + cleanup together since both are job-table maintenance).

```rust
pub struct JobCleanupTask {
    db: PgPool,
    retention_secs: i64,    // default 30 days
    interval: Duration,     // default 24 h
}
// name() = "job_cleanup"; run_on_startup() = false
// tick(): JobRepository::delete_terminal_older_than(&self.db, retention_secs).await?; log count.
```

**New repository method** `repository/job.rs`:

```rust
/// Delete terminal jobs (`completed` / `failed`) whose `completed_at` is older than
/// `retention_secs`. Never touches `pending` / `processing`. Returns rows deleted.
pub async fn delete_terminal_older_than(db: &PgPool, retention_secs: i64) -> Result<u64, AppError> {
    let res = sqlx::query!(
        r#"DELETE FROM jobs
           WHERE status IN ('completed', 'failed')
             AND completed_at IS NOT NULL
             AND completed_at < (now() AT TIME ZONE 'utc') - make_interval(secs => $1)"#,
        retention_secs as f64
    )
        .execute(db)
        .await
        .map_err(map_sqlx_error)?;
    Ok(res.rows_affected())
}
```

Notes:

- `completed_at` is set for both `completed` and permanently-`failed` jobs (the `fail`
  path sets it via the existing `CASE`), so it is the correct retention anchor. Verify
  against `repository/job.rs::fail` while implementing; if some failed rows can have a
  NULL `completed_at`, fall back to `created_at` for those.
- For a large backlog, delete in batches (`… AND id IN (SELECT id … LIMIT 10000)` looped)
  to avoid one giant transaction / lock. The first cleanup after deploying on an existing
  instance may remove many rows.
- `make_interval(secs => …)` keeps the cutoff in SQL; remember `cargo sqlx prepare` after
  adding the query (per coding guidelines).

### 3.3 `PipelineRecoverySweepTask` (migrate out of the pipeline loop)

Lives in `infra/pipeline.rs` (reuses `PipelineRepository::find_users_with_dirty_pictures`
and the `PipelineWaker`).

```rust
pub struct PipelineRecoverySweepTask {
    db: PgPool,
    waker: PipelineWaker,
    interval: Duration,     // = pipeline_poll_interval_secs
}
// name() = "pipeline_recovery_sweep"; run_on_startup() = true
// tick(): for user_id in find_users_with_dirty_pictures(&db): waker.wake(user_id);
```

This **replaces** both the startup `recovery_sweep(&db, &scheduler)` call and the
`_ = sleep(poll_interval)` arm inside `infra/pipeline.rs::run`. The sweep no longer needs
the pipeline's internal `Scheduler`; it simply pushes user-ids into the existing
`PipelineWaker` mpsc, and the pipeline loop's `rx.recv()` arm schedules them as usual.

Consequences for `infra/pipeline.rs`:

- `run` / `create` drop the `poll_interval: Duration` parameter and the poll `select!`
  arm; the loop becomes purely event-driven (`rx.recv()` only, breaking on channel close).
- The `recovery_sweep` free function stays but is now called only by the task (or its body
  inlines into the task). Keep `find_users_with_dirty_pictures` in the repository.
- `run_once_for_user` (test helper) is unaffected.

## 4. Wiring in `main.rs`

Replace the three separate `tokio::spawn`s (watchdog + pipeline poll arm) with one
scheduler. Order: build `pipeline_waker`/`pipeline_rx` first (unchanged), build the
scheduler, then spawn the pipeline loop (now without `poll_interval`).

```rust
let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

let mut scheduler = infra::scheduler::Scheduler::new();
scheduler
.register(Arc::new(infra::job_watchdog::JobWatchdogTask::new(
db.clone(),
config.job_processing_timeout_secs,
Duration::from_secs(config.job_watchdog_interval_secs),
)))
.register(Arc::new(infra::job_watchdog::JobCleanupTask::new(
db.clone(),
config.job_retention_secs,
Duration::from_secs(config.job_cleanup_interval_secs),
)))
.register(Arc::new(infra::pipeline::PipelineRecoverySweepTask::new(
db.clone(),
pipeline_waker.clone(),
Duration::from_secs(config.pipeline_poll_interval_secs),
)));
tokio::spawn(scheduler.run(shutdown_rx));

// Pipeline loop — now event-driven only (no poll_interval).
tokio::spawn(infra::pipeline::create(
db.clone(),
pipeline_rx,
config.clone(),
config.pipeline_concurrency,
federation.clone(),
cache.clone(),
pipeline_waker.clone(),
));
```

Keep `shutdown_tx` alive (e.g. store it, or `std::mem::forget`-equivalent by binding it
for the lifetime of `main`); dropping it would close the channel. Since graceful shutdown
is out of scope, simply binding it in `main` is enough.

## 5. Config (`infra/config.rs` + `back/.env.example`)

Add two knobs; keep the existing ones (now consumed by the new tasks):

| Field                       | Env var                     | Default          | Meaning                                                      |
|-----------------------------|-----------------------------|------------------|--------------------------------------------------------------|
| `job_retention_secs`        | `JOB_RETENTION_SECS`        | `2592000` (30 d) | Age after `completed_at` at which terminal jobs are deleted. |
| `job_cleanup_interval_secs` | `JOB_CLEANUP_INTERVAL_SECS` | `86400` (24 h)   | How often the cleanup task runs.                             |

Existing, unchanged in meaning: `job_processing_timeout_secs`,
`job_watchdog_interval_secs`, `pipeline_poll_interval_secs` (the last now drives
`PipelineRecoverySweepTask`). Add the two new vars to `back/.env.example` with comments,
and to the `Config::test_default()` (or equivalent) used by tests (mirror the existing
defaults block around `config.rs:294`).

## 6. Testing

- **Framework** (`scheduler.rs` unit test): a `CountingTask` with a short interval and an
  `AtomicUsize`; assert it ticks ≥1 time within a `tokio::time` window, that
  `run_on_startup` causes an immediate tick, and that flipping the `watch` to `true` stops
  the loop. Use `tokio::time::pause`/`advance` for determinism.
- **`delete_terminal_older_than`** (repository integration test, `#[sqlx::test]`): insert
  a `completed` job with an old `completed_at`, a recent `completed` job, a `failed` old
  job, and a `pending` job; run; assert only the two old terminal rows are deleted.
- **`JobCleanupTask::tick`**: thin; covered by the repository test plus one task-level test
  asserting `tick()` returns `Ok` and deletes.
- **Pipeline recovery sweep**: reuse the existing pipeline test harness — mark a picture
  dirty, run `PipelineRecoverySweepTask::tick`, assert the user id is delivered through the
  waker (or that a subsequent pipeline pass processes it). Confirm the pipeline loop still
  recovers dirty users now that its internal poll arm is gone.
- **Watchdog**: keep/move the existing `reset_stale` coverage; add a task-level smoke test.

## 7. Documentation updates

- `doc/03_BACKEND_ARCHITECTURE.md` — in the module-layout block, replace the standalone
  `job_watchdog.rs` line with the scheduler + tasks; note the pipeline loop is now
  event-driven and its poll fallback is a registered recurring task. Update the AppState /
  startup description if it lists the watchdog spawn.
- `doc/02_INFRASTRUCTURE_DESIGN.md` — if it describes background loops / the watchdog,
  update to reference the unified scheduler and mention job retention (30 d default).
- `back/.env.example` — the two new vars (§5).
- **Consistency sweep (carryover):** while touching storage/worker docs, verify no doc
  still claims the `pictures` bucket is *immutable after upload* — it is overwritten in
  place on edit, with the prior file copied to the `versions` bucket. `back/.env.example`
  was already corrected; check `02_INFRASTRUCTURE_DESIGN.md`, `03_BACKEND_ARCHITECTURE.md`,
  `04_WORKER_ARCHITECTURE.md`, and any S3 module comments for the same stale claim and fix
  them to: pictures = current/latest file (mutable); versions = previous versions +
  preserved original.

## 8. Work breakdown

- [ ] `infra/scheduler.rs`: `RecurringTask` trait + `Scheduler` + `run_task`/`run_once`;
  register in `infra.rs`. (`async-trait` already present — no `Cargo.toml` change.)
- [ ] Migrate `JobWatchdogTask`; delete the old `job_watchdog::run` loop.
- [ ] `JobRepository::delete_terminal_older_than` + `cargo sqlx prepare`; `JobCleanupTask`.
- [ ] `PipelineRecoverySweepTask`; strip the poll arm + startup sweep + `poll_interval`
  param from `infra/pipeline.rs`.
- [ ] Config: `job_retention_secs`, `job_cleanup_interval_secs` (+ `.env.example`, test
  defaults).
- [ ] Rewire `main.rs` (scheduler + `watch` shutdown channel; pipeline `create` signature).
- [ ] Tests (§6).
- [ ] Docs (§7), including the immutable-bucket consistency sweep.
