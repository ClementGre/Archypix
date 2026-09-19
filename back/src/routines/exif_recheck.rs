//! Admin EXIF recheck sweep [`Routine`] (feature 33 §8).
//!
//! An allowlist bump, an engine upgrade or a tool outage leaves a stored verdict stale. The SQL that
//! finds those rows is trivial; the hazard is enqueue rate — a matching sweep can be the whole
//! library, and flipping a million rows while enqueuing a million jobs is an incident, not a
//! migration. Running it as a routine with a bounded batch per tick gives backpressure and
//! resumability for free. Trigger-only: the admin endpoint is the only producer.
//!
//! Rows holding unsynced DB edits (`pending`, `pending_job_creation`, `write_failed`) are never in
//! scope — a re-extraction makes the file authoritative and would discard them (31 §5).

use crate::domain::routine::{ExifRecheckInput, RecheckScope};
use crate::infra::settings::keys;
use crate::routines::{Routine, RoutineHandle};
use archypix_common::settings::Settings;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

/// Wake handle for the EXIF recheck sweep.
pub type ExifRecheckHandle = RoutineHandle<ExifRecheckInput>;

/// Re-extracts EXIF for pictures whose stored verdict may be stale.
pub struct ExifRecheckRoutine {
    db: PgPool,
    settings: Arc<Settings>,
}

impl ExifRecheckRoutine {
    pub fn new(db: PgPool, settings: Arc<Settings>) -> Self {
        Self { db, settings }
    }
}

#[async_trait::async_trait]
impl Routine for ExifRecheckRoutine {
    type Input = ExifRecheckInput;
    /// One sweep per scope at a time; a second request for the same scope reruns after it.
    type Key = RecheckScope;

    fn name(&self) -> &'static str {
        "exif_recheck"
    }

    fn key(input: &ExifRecheckInput) -> RecheckScope {
        input.scope
    }

    /// Drain in bounded batches until a pass enqueues fewer than `batch`. The sweep id is minted
    /// here, so the idempotency key is idempotent within this sweep and never against a previous one.
    async fn run(&self, input: ExifRecheckInput) -> anyhow::Result<()> {
        let batch = self.settings.get(keys::EXIF_RECHECK_BATCH);
        let sweep_id = Uuid::new_v4();
        let mimes = (!input.mime_types.is_empty()).then_some(input.mime_types.as_slice());
        let mut total = 0usize;
        loop {
            let n = crate::services::jobs::recheck_exif_batch(
                &self.db,
                input.scope,
                mimes,
                sweep_id,
                batch,
            )
            .await?;
            total += n;
            if (n as i64) < batch {
                break;
            }
        }
        if total > 0 {
            info!(scope = ?input.scope, enqueued = total, "exif recheck sweep enqueued re-extractions");
        }
        Ok(())
    }
}
