//! Hierarchy CRUD orchestration and the read resolver.
//!
//! The resolver ([`resolve`]) turns a `HierarchyConfig` plus the user's distinct tag paths into a
//! [`ResolvedDir`] tree (the synthetic root and its descendants). Each directory carries a
//! [`TagPredicate`] for its direct files (`browse`) and one for its subtree (counts / empty-dir
//! hiding). It is the single source of truth for both the webapp navigation and (later) WebDAV.
//!
//! See `doc/features/05_hierarchies.md` §5–6.

mod browse;
mod crud;
mod resolver;
mod tree;
mod webdav;

pub use browse::*;
pub use crud::*;
pub use resolver::*;
pub use tree::*;
pub use webdav::*;
