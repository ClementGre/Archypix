//! Resolver background routines (feature 23 §8.3), on the shared `common::routine` framework.
//!
//! - [`StaleBackendPrune`] marks a backend unreachable once its stored delegation token is past
//!   expiry (threshold = the delegation TTL).
//! - [`InviteCleanup`] deletes expired / fully-consumed invites.
//! - [`MappingReconcile`] reconciles `username → backend` mappings against each backend's authoritative
//!   user list (feature 24) — fixing drift the push protocol misses (deleted / moved users).

use crate::clients::BackendClient;
use crate::config::{Config, setting_keys as sk};
use crate::repository;
use archypix_common::routine::Routine;
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tracing::{info, warn};

pub struct StaleBackendPrune {
    pub db: PgPool,
    pub config: Config,
}

#[async_trait::async_trait]
impl Routine for StaleBackendPrune {
    type Input = ();
    type Key = ();
    fn name(&self) -> &'static str {
        "stale_backend_prune"
    }
    fn key(_: &()) {}
    fn interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(
            self.config.get(sk::STALE_PRUNE_INTERVAL_SECS),
        ))
    }
    fn run_on_startup(&self) -> bool {
        true
    }
    async fn run(&self, _: ()) -> anyhow::Result<()> {
        let n = repository::prune_stale(&self.db).await?;
        if n > 0 {
            info!(
                pruned = n,
                "stale-backend prune: marked backends unreachable"
            );
        }
        Ok(())
    }
}

pub struct InviteCleanup {
    pub db: PgPool,
    pub config: Config,
}

#[async_trait::async_trait]
impl Routine for InviteCleanup {
    type Input = ();
    type Key = ();
    fn name(&self) -> &'static str {
        "invite_cleanup"
    }
    fn key(_: &()) {}
    fn interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(
            self.config.get(sk::INVITE_CLEANUP_INTERVAL_SECS),
        ))
    }
    async fn run(&self, _: ()) -> anyhow::Result<()> {
        let n = repository::cleanup_invites(&self.db).await?;
        if n > 0 {
            info!(
                deleted = n,
                "invite cleanup: removed expired/exhausted invites"
            );
        }
        Ok(())
    }
}

/// Reconcile `username → backend` mappings against each backend's authoritative user list (feature 24).
///
/// The push protocol (`/api/update` on register) keeps mappings *fresh* but misses **deletions** and
/// out-of-band moves. This routine closes that gap: it queries each **reachable** backend's
/// `/api/admin/users` (delegation replay), then
/// - **adds/fixes** a mapping when a backend authoritatively claims a username the resolver has wrong;
/// - **prunes** a mapping only when its own backend was reachable+queried and the user is confirmed
///   absent from *every* reachable backend (a user on an unreachable backend is never pruned — we can't
///   tell, so we keep it).
///
/// Cache coherence is left to the moka TTL: a moved user's cached URL self-heals within the TTL, which
/// is fine for a slow drift-correcting safety net.
pub struct MappingReconcile {
    pub db: PgPool,
    pub config: Config,
    pub backends: BackendClient,
}

#[async_trait::async_trait]
impl Routine for MappingReconcile {
    type Input = ();
    type Key = ();
    fn name(&self) -> &'static str {
        "mapping_reconcile"
    }
    fn key(_: &()) {}
    fn interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(
            self.config.get(sk::MAPPING_RECONCILE_INTERVAL_SECS),
        ))
    }
    fn run_on_startup(&self) -> bool {
        true
    }

    async fn run(&self, _: ()) -> anyhow::Result<()> {
        let backends = repository::list_backends(&self.db).await?;

        // Query each reachable backend's authoritative usernames. A backend we fail to reach is simply
        // not part of this pass (its mappings are left untouched).
        let mut per_backend: HashMap<String, HashSet<String>> = HashMap::new();
        for b in backends.iter().filter(|b| b.reachable) {
            match self.backends.get_json(&b.back_domain, "/api/admin/users").await {
                Ok(serde_json::Value::Array(arr)) => {
                    let users: HashSet<String> = arr
                        .iter()
                        .filter_map(|u| u.get("username").and_then(|v| v.as_str()).map(String::from))
                        .collect();
                    per_backend.insert(b.back_domain.clone(), users);
                }
                Ok(_) => warn!(back_domain = %b.back_domain, "mapping reconcile: unexpected /users shape"),
                Err(e) => warn!(back_domain = %b.back_domain, error = %e, "mapping reconcile: user list fetch failed, skipping"),
            }
        }
        if per_backend.is_empty() {
            return Ok(());
        }

        // Desired: username → the (first) queried backend that claims it.
        let mut desired: HashMap<String, String> = HashMap::new();
        for (bd, users) in &per_backend {
            for u in users {
                desired.entry(u.clone()).or_insert_with(|| bd.clone());
            }
        }

        let current: HashMap<String, String> =
            repository::list_mappings(&self.db).await?.into_iter().collect();

        // Add/fix mappings that differ from the authoritative claim.
        let (mut added, mut moved) = (0u64, 0u64);
        for (u, bd) in &desired {
            if current.get(u) != Some(bd) {
                repository::upsert_mapping(&self.db, u, bd).await?;
                if current.contains_key(u) {
                    moved += 1
                } else {
                    added += 1
                }
            }
        }

        // Prune mappings whose backend was reachable + queried and no reachable backend claims the user.
        let to_delete: Vec<String> = current
            .iter()
            .filter(|(u, bd)| !desired.contains_key(*u) && per_backend.contains_key(*bd))
            .map(|(u, _)| u.clone())
            .collect();
        let removed = repository::delete_mappings(&self.db, &to_delete).await?;

        if added + moved + removed > 0 {
            info!(added, moved, removed, "mapping reconcile: adjusted user mappings");
        }
        Ok(())
    }
}
