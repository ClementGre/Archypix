//! Resolver application state, mirroring the backend's `state.rs`.

use crate::clients::BackendClient;
use crate::config::{Config, setting_keys as sk};
use archypix_common::auth::JwtService;
use archypix_common::routine::{RoutineStatus, TriggerAny};
use moka::future::Cache;
use sqlx::PgPool;
use std::sync::Arc;

/// One monitored routine for the resolver Routines tab (feature 24): live status + a type-erased
/// "trigger now" handle. Tuning fields are discovered from the settings registry.
pub struct RoutineEntry {
    pub name: &'static str,
    pub status: RoutineStatus,
    pub trigger: Arc<dyn TriggerAny>,
}

/// Registry of the resolver's spawned routines, read by `GET /api/resolver-admin/routines`.
#[derive(Clone, Default)]
pub struct RoutineRegistry {
    pub entries: Arc<Vec<RoutineEntry>>,
}

/// Injected into every Axum handler via `State<AppState>`.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    /// username → backend public URL cache (moka TTL).
    pub cache: Cache<String, String>,
    /// The layered runtime config (feature 23 §4).
    pub config: Config,
    /// Shared HS256 service (secret = `resolver_jwt_secret`): verifies backend **push** tokens and
    /// signs/verifies operator `ResolverAdminSession` tokens.
    pub jwt: JwtService,
    /// Outbound client that replays each backend's stored delegation token (feature 23 §3.2, §5.3).
    pub backends: BackendClient,
    /// Spawned routines with live status + manual-trigger handles (feature 24 Routines tab).
    pub routine_registry: RoutineRegistry,
}

impl AppState {
    pub fn new(db: PgPool, config: Config, routine_registry: RoutineRegistry) -> Self {
        let jwt = JwtService::new(
            &config.get(sk::RESOLVER_JWT_SECRET),
            &config.get(sk::GLOBAL_DOMAIN),
        );
        let cache = Cache::builder()
            .time_to_live(std::time::Duration::from_secs(
                config.get(sk::CACHE_TTL_SECS),
            ))
            .max_capacity(config.get(sk::CACHE_MAX_CAPACITY))
            .build();
        let backends = BackendClient::new(db.clone(), reqwest::Client::new());
        Self {
            db,
            cache,
            config,
            jwt,
            backends,
            routine_registry,
        }
    }

    pub fn global_domain(&self) -> String {
        self.config.get(sk::GLOBAL_DOMAIN)
    }
}
