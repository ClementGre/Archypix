//! Tag-rename cascade [`Routine`] (edge case §7).
//!
//! Renaming a tag is a real search-and-replace across everything a user owns: manual picture tags,
//! outgoing-share tags, tagging-service gates + config (SharedTagMapping included), and hierarchy
//! configs. Changed services are invalidated and covered pictures marked dirty, then the pipeline is
//! woken to re-derive service tags and re-announce shares under the renamed tag. Trigger-only (no
//! sweep), so — like the old `TaskQueue` — a trigger lost to a crash is lost; making it durable (a
//! DB outbox + a re-deriving sweep) is a noted follow-up. The work itself lives in
//! [`crate::services::tags::cascade_rename`].

use crate::domain::tag::TagPath;
use crate::infra::routine::{Routine, RoutineHandle};
use sqlx::PgPool;
use uuid::Uuid;

/// Payload **and** dedup key. Identical renames in flight collapse to one rerun (idempotent).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TagRenameInput {
    pub user_id: Uuid,
    /// ltree form (dot-separated), already validated non-reserved by the endpoint.
    pub old_tag: String,
    pub new_tag: String,
}

/// Renames a tag across tags, shares, tagging-service configs, and hierarchies, then wakes the
/// pipeline so the re-tag + re-announce work runs.
pub struct TagRename {
    db: PgPool,
    pipeline: RoutineHandle<Uuid>,
    concurrency: usize,
}

impl TagRename {
    pub fn new(db: PgPool, pipeline: RoutineHandle<Uuid>, concurrency: usize) -> Self {
        Self {
            db,
            pipeline,
            concurrency,
        }
    }
}

#[async_trait::async_trait]
impl Routine for TagRename {
    type Input = TagRenameInput;
    type Key = TagRenameInput;

    fn name(&self) -> &'static str {
        "tag_rename"
    }

    fn key(input: &TagRenameInput) -> TagRenameInput {
        input.clone()
    }

    fn concurrency(&self) -> usize {
        self.concurrency
    }

    async fn run(&self, input: TagRenameInput) -> anyhow::Result<()> {
        let TagRenameInput {
            user_id,
            old_tag,
            new_tag,
        } = input;
        let old = TagPath::from_ltree(old_tag);
        let new = TagPath::from_ltree(new_tag);
        let outcome = crate::services::tags::cascade_rename(&self.db, user_id, &old, &new).await?;
        tracing::info!(
            %user_id, old = %old, new = %new,
            tags_renamed = outcome.tags_renamed,
            services_changed = outcome.services_changed,
            shares_renamed = outcome.shares_renamed,
            hierarchies_changed = outcome.hierarchies_changed,
            pictures_invalidated = outcome.pictures_invalidated,
            "tag rename cascade"
        );
        if outcome.needs_pipeline_wake() {
            self.pipeline.trigger(user_id);
        }
        Ok(())
    }
}
