//! Best-effort downstream-unannounce [`Routine`] (the revocation-cascade tail).
//!
//! Emitted by `cleanup_incoming_share` and the purge sweep: removes specific pictures from a share
//! recipient (same-backend directly against the DB, cross-instance via the recipient's federation
//! endpoint). Trigger-only (no sweep) — exactly the old `TaskQueue` best-effort behaviour: a trigger
//! lost to a crash is lost. The pipeline handles all other (un)announcement inline.

use crate::clients::federation::FederationClient;
use crate::infra::config::Config;
use crate::infra::routine::{Routine, RoutineHandle};
use sqlx::PgPool;
use uuid::Uuid;

/// Payload **and** dedup key. Two distinct unannounces (different fields) both run; two identical
/// ones in flight collapse to a single rerun (idempotent).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct UnannounceInput {
    pub outgoing_share_id: Uuid,
    pub sender_username: String,
    pub recipient_username: String,
    pub recipient_instance: String,
    /// Announce ids (recipient's `remote_picture_id`) of the pictures to remove.
    pub picture_ids: Vec<String>,
    pub is_same_backend: bool,
}

/// Delivers `UnannounceInput`s. Holds the pipeline handle so a same-backend unregister can wake the
/// recipient's pipeline.
pub struct Unannounce {
    db: PgPool,
    federation: FederationClient,
    config: Config,
    pipeline: RoutineHandle<Uuid>,
}

impl Unannounce {
    pub fn new(
        db: PgPool,
        federation: FederationClient,
        config: Config,
        pipeline: RoutineHandle<Uuid>,
    ) -> Self {
        Self {
            db,
            federation,
            config,
            pipeline,
        }
    }
}

#[async_trait::async_trait]
impl Routine for Unannounce {
    type Input = UnannounceInput;
    type Key = UnannounceInput;

    fn name(&self) -> &'static str {
        "unannounce"
    }

    fn key(input: &UnannounceInput) -> UnannounceInput {
        input.clone()
    }

    fn concurrency(&self) -> usize {
        self.config.task_queue_concurrency
    }

    async fn run(&self, input: UnannounceInput) -> anyhow::Result<()> {
        crate::services::shares::deliver_unannounce(
            &self.db,
            &self.federation,
            &self.config,
            &self.pipeline,
            input,
        )
        .await?;
        Ok(())
    }
}
