//! Small framework for **periodic** background work.
//!
//! Each periodic behaviour (job watchdog, job cleanup, pipeline recovery sweep) implements
//! [`RecurringTask`] and is registered on a [`Scheduler`], which spawns one loop per task.
//!
//! This is **not** the one-shot [`crate::infra::tasks::TaskQueue`] (ad-hoc, event-triggered work).
//! The two coexist: `TaskQueue` = "do this once, now"; `Scheduler` = "do this every N seconds".

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tracing::{Instrument, debug, error, info};

/// A unit of periodic background work. One implementor per behaviour; each carries its own
/// dependencies (db pool, config values, wake handles, …).
///
/// Implementors run **serially with themselves** (the next tick starts only after the previous one
/// returns) and **concurrently with other tasks** (each gets its own spawned loop). A failing tick
/// is logged and never aborts the loop.
#[async_trait::async_trait]
pub trait RecurringTask: Send + Sync + 'static {
    /// Stable, lower-snake name for logs/metrics, e.g. `"job_watchdog"`.
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

/// Registry of periodic tasks. Spawns one loop per task via [`run`](Self::run).
#[derive(Default)]
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

    /// Spawn one loop per registered task. Returns a future that resolves when all loops have
    /// stopped (after `shutdown` fires). Spawn it with `tokio::spawn`.
    ///
    /// `shutdown`: a `watch` receiver; when its value flips to `true`, every loop breaks after its
    /// current tick. Pass a never-firing receiver to run until process exit.
    pub fn run(self, shutdown: watch::Receiver<bool>) -> impl Future<Output = ()> {
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
    info!(
        task = name,
        interval_secs = interval.as_secs(),
        "recurring task started"
    );

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
    let run_id = uuid::Uuid::new_v4();
    let span = tracing::info_span!("recurring_task", task = task.name(), run_id = %run_id);
    let start = Instant::now();
    match task.tick().instrument(span).await {
        Ok(()) => debug!(
            task = task.name(),
            elapsed_ms = start.elapsed().as_millis() as u64,
            "tick ok"
        ),
        Err(e) => error!(task = task.name(), error = ?e, "tick failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingTask {
        counter: Arc<AtomicUsize>,
        interval: Duration,
        startup: bool,
    }

    #[async_trait::async_trait]
    impl RecurringTask for CountingTask {
        fn name(&self) -> &'static str {
            "counting"
        }
        fn interval(&self) -> Duration {
            self.interval
        }
        fn run_on_startup(&self) -> bool {
            self.startup
        }
        async fn tick(&self) -> anyhow::Result<()> {
            self.counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn ticks_on_interval() {
        let counter = Arc::new(AtomicUsize::new(0));
        let task = Arc::new(CountingTask {
            counter: counter.clone(),
            interval: Duration::from_millis(5),
            startup: false,
        });
        let (_tx, rx) = watch::channel(false);
        tokio::spawn(run_task(task, rx));

        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(
            counter.load(Ordering::SeqCst) >= 2,
            "expected several ticks within the window"
        );
    }

    #[tokio::test]
    async fn run_on_startup_ticks_immediately() {
        let counter = Arc::new(AtomicUsize::new(0));
        let task = Arc::new(CountingTask {
            counter: counter.clone(),
            interval: Duration::from_secs(3600),
            startup: true,
        });
        let (_tx, rx) = watch::channel(false);
        tokio::spawn(run_task(task, rx));

        // Only the startup tick should have fired (interval is an hour).
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn shutdown_stops_the_loop() {
        let counter = Arc::new(AtomicUsize::new(0));
        let task = Arc::new(CountingTask {
            counter: counter.clone(),
            interval: Duration::from_secs(3600),
            startup: false,
        });
        let (tx, rx) = watch::channel(false);
        let handle = tokio::spawn(run_task(task, rx));

        tx.send(true).unwrap();
        // The loop should observe the shutdown and finish.
        handle.await.unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }
}
