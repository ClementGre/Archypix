use crate::clients::federation::FederationClient;
use crate::clients::resolver::ResolverClient;
use crate::infra::config::Config;
use crate::infra::crypto::JwtService;
use crate::infra::redis::Cache;
use crate::infra::routine::RoutineHandle;
use crate::infra::routine::tag_rename::TagRenameInput;
use crate::infra::routine::unannounce::UnannounceInput;
use crate::infra::s3::Storage;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// Trigger handles for the routine framework (feature 17). Each routine that anything outside its own
/// runtime triggers gets a handle here; the sweep-only routines (job watchdog/cleanup, purge sweep)
/// need none. See `infra::routine` and `doc/features/17_unified_routine_framework.md`.
#[derive(Clone)]
pub struct Routines {
    /// Per-user pipeline wake. Trigger after any event that creates dirty pictures or share work for
    /// a user (ingest, tag edit, service config change, share accept, …).
    pub pipeline: RoutineHandle<Uuid>,
    /// Deferred-EXIF-job drain (feature 14 §5). Trigger after a batch EXIF edit stamps new
    /// `pending_job_creation` rows.
    pub exif_drain: RoutineHandle<()>,
    /// Tag-rename cascade (edge case §7).
    pub tag_rename: RoutineHandle<TagRenameInput>,
    /// Best-effort downstream unannounce (revocation cascade).
    pub unannounce: RoutineHandle<UnannounceInput>,
}

/// Application state injected into every Axum handler via `State<AppState>`.
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: PgPool,
    /// Cache abstraction — `RedisClient` in production, `InMemoryCache` in tests.
    pub cache: Arc<dyn Cache>,
    pub jwt: JwtService,
    /// JWT service using the worker shared secret — verifies inbound worker tokens.
    pub worker_jwt: JwtService,
    /// Object storage abstraction — `StorageClient` in production, `MockStorage` in tests.
    pub storage: Arc<dyn Storage>,
    pub federation: FederationClient,
    pub resolver: ResolverClient,
    /// Background-work trigger handles (feature 17).
    pub routines: Routines,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: Config,
        db: PgPool,
        cache: Arc<dyn Cache>,
        jwt: JwtService,
        worker_jwt: JwtService,
        storage: Arc<dyn Storage>,
        federation: FederationClient,
        resolver: ResolverClient,
        routines: Routines,
    ) -> Self {
        Self {
            config,
            db,
            cache,
            jwt,
            worker_jwt,
            storage,
            federation,
            resolver,
            routines,
        }
    }
}
