//! Resolver heartbeat (feature 23 §3.2, §8.2).
//!
//! A startup + interval [`Routine`](crate::infra::routine::Routine): mint a fresh backend-signed
//! `ResolverDelegation` token, gather fleet metrics, and push both to the resolver. The resolver
//! stores the token (replaying it on every call it makes to this backend) and the metrics (for its
//! placement strategies + overview). A missed heartbeat self-heals on the next tick; the delegation
//! TTL is > the interval so the resolver always holds a live token. Only spawned when
//! `use_resolver = true`.

use crate::clients::resolver::{HeartbeatMetrics, ResolverClient};
use crate::infra::routine::Routine;
use crate::infra::settings::keys;
use crate::repository::admin::AdminRepository;
use archypix_common::settings::Settings;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;

pub struct ResolverHeartbeatRoutine {
    db: PgPool,
    resolver: ResolverClient,
    settings: Arc<Settings>,
}

impl ResolverHeartbeatRoutine {
    pub fn new(db: PgPool, resolver: ResolverClient, settings: Arc<Settings>) -> Self {
        Self {
            db,
            resolver,
            settings,
        }
    }
}

#[async_trait::async_trait]
impl Routine for ResolverHeartbeatRoutine {
    type Input = ();
    type Key = ();

    fn name(&self) -> &'static str {
        "resolver_heartbeat"
    }

    fn key(_input: &()) {}

    fn interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(
            self.settings.get(keys::RESOLVER_HEARTBEAT_INTERVAL_SECS),
        ))
    }

    fn run_on_startup(&self) -> bool {
        true
    }

    async fn run(&self, _input: ()) -> anyhow::Result<()> {
        let (user_count, picture_count, storage_bytes) =
            AdminRepository::fleet_metrics(&self.db).await?;
        self.resolver
            .heartbeat(HeartbeatMetrics {
                user_count,
                picture_count,
                storage_bytes,
                healthy: true,
            })
            .await?;
        Ok(())
    }
}
