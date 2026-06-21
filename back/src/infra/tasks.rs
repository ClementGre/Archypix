//! In-process background task queue for lightweight DB-only and federation-delivery jobs.
//!
//! The tagging pipeline runs as a separate loop (`infra::pipeline`) with a
//! `Notify`-based wake model and configurable recovery polling interval.
//!
//! # Design
//! An unbounded `mpsc` channel decouples enqueue from execution.
//! A semaphore caps concurrency. Each task is spawned as a Tokio task
//! and holds a permit for its duration.

use crate::clients::federation::FederationClient;
use crate::infra::config::Config;
use crate::infra::pipeline::PipelineWaker;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::{Semaphore, mpsc};
use uuid::Uuid;

#[derive(Debug)]
pub enum InternalTask {
    /// Tag rename across tags, shares, segmentation configs, and hierarchies.
    TagRename {
        user_id: Uuid,
        old_tag: String,
        new_tag: String,
    },

    /// Unannounce specific pictures from a share recipient. Emitted by `cleanup_incoming_share`
    /// during revocation (best-effort downstream cascade); the pipeline handles all other
    /// (un)announcement inline.
    UnannounceSharedPictures {
        outgoing_share_id: Uuid,
        sender_username: String,
        recipient_username: String,
        recipient_instance: String,
        /// Announce ids (recipient's `remote_picture_id`) of the pictures to remove.
        picture_ids: Vec<String>,
        is_same_backend: bool,
    },
}

// ── Queue handle ──────────────────────────────────────────────────────────────

/// Cheaply-cloneable handle for submitting tasks to the in-process queue.
/// Clone this into `AppState`; call `enqueue` anywhere in the application.
#[derive(Clone)]
pub struct TaskQueue {
    sender: mpsc::UnboundedSender<InternalTask>,
}

impl TaskQueue {
    /// Submit a task. Returns immediately; execution is asynchronous.
    /// Errors silently if the runner has been dropped (should never happen in practice).
    pub fn enqueue(&self, task: InternalTask) {
        if self.sender.send(task).is_err() {
            tracing::error!("task queue: receiver dropped — task lost");
        }
    }
}

// ── Queue constructor ─────────────────────────────────────────────────────────

/// Returns a future that runs forever (until the process exits). Spawn it with `tokio::spawn`.
pub fn create(
    db: PgPool,
    federation: FederationClient,
    config: Config,
    pipeline_waker: PipelineWaker,
    concurrency: usize,
) -> (TaskQueue, impl Future<Output = ()>) {
    let (tx, rx) = mpsc::unbounded_channel::<InternalTask>();
    let runner = TaskRunner {
        db,
        federation,
        config,
        pipeline_waker,
        rx,
        sem: Arc::new(Semaphore::new(concurrency)),
    };
    (TaskQueue { sender: tx }, runner.run())
}

// ── Runner (private) ──────────────────────────────────────────────────────────

struct TaskRunner {
    db: PgPool,
    federation: FederationClient,
    config: Config,
    pipeline_waker: PipelineWaker,
    rx: mpsc::UnboundedReceiver<InternalTask>,
    sem: Arc<Semaphore>,
}

impl TaskRunner {
    async fn run(mut self) {
        tracing::info!("in-process task runner started");
        while let Some(task) = self.rx.recv().await {
            let permit = self
                .sem
                .clone()
                .acquire_owned()
                .await
                .expect("semaphore closed");
            let db = self.db.clone();
            let federation = self.federation.clone();
            let config = self.config.clone();
            let waker = self.pipeline_waker.clone();
            tokio::spawn(async move {
                execute_task(db, federation, config, waker, task).await;
                drop(permit);
            });
        }
        tracing::info!("in-process task runner stopped");
    }
}

async fn execute_task(
    db: PgPool,
    federation: FederationClient,
    config: Config,
    waker: PipelineWaker,
    task: InternalTask,
) {
    let task_name = match &task {
        InternalTask::TagRename { .. } => "tag_rename",
        InternalTask::UnannounceSharedPictures { .. } => "unannounce_shared_pictures",
    };
    let span = tracing::info_span!("background_task", task = task_name);
    let _enter = span.enter();
    match task {
        InternalTask::TagRename {
            user_id,
            ref old_tag,
            ref new_tag,
        } => {
            tracing::info!(
                user_id = %user_id,
                old_tag = %old_tag,
                new_tag = %new_tag,
                "in-process task: tag rename"
            );
            todo!(
                "implement tag rename across tags, shares, segmentation configs, hierarchies, ..."
            );
        }
        InternalTask::UnannounceSharedPictures { .. } => {
            if let Err(e) = crate::services::shares::deliver_unannounce_task(
                &db,
                &federation,
                &config,
                &waker,
                task,
            )
            .await
            {
                tracing::error!(error = ?e, "unannounce task failed");
            }
        }
    }
}
