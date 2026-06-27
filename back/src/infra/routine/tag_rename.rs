//! Tag-rename cascade [`Routine`] (edge case §7).
//!
//! Renaming a tag must update all stored tag records on affected pictures plus segment/hierarchy/
//! share configurations. Trigger-only (no sweep), so — like the old `TaskQueue` — a trigger lost to
//! a crash is lost; making it durable (a DB outbox + a re-deriving sweep) is a noted follow-up.

use crate::infra::routine::Routine;
use sqlx::PgPool;
use uuid::Uuid;

/// Payload **and** dedup key. Identical renames in flight collapse to one rerun (idempotent).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TagRenameInput {
    pub user_id: Uuid,
    pub old_tag: String,
    pub new_tag: String,
}

/// Renames a tag across tags, shares, segmentation configs, and hierarchies.
pub struct TagRename {
    #[allow(dead_code)]
    db: PgPool,
    concurrency: usize,
}

impl TagRename {
    pub fn new(db: PgPool, concurrency: usize) -> Self {
        Self { db, concurrency }
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
        tracing::info!(%user_id, %old_tag, %new_tag, "tag rename");
        todo!("implement tag rename across tags, shares, segmentation configs, hierarchies, ...")
    }
}
