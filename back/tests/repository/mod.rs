//! Repository-layer DB suite — one binary, one module per repository.
//!
//! These were `#[cfg(test)]` modules inside `back/src/repository/*.rs`, but they are `#[sqlx::test]`
//! integration tests against a real database and touch only the public API, so they belong here
//! where they can share `common` instead of re-seeding by hand.

#[path = "../common/mod.rs"]
mod common;

mod hierarchy;
mod job;
mod share;
mod share_announcement;
mod tag;
mod tag_metadata;

pub(crate) static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
