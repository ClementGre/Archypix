use crate::clients::federation::FederationClient;
use crate::clients::resolver::ResolverClient;
use crate::infra::crypto::JwtService;
use crate::infra::redis::Cache;
use crate::infra::routine::tag_rename::TagRenameInput;
use crate::infra::routine::unannounce::UnannounceInput;
use crate::infra::routine::RoutineHandle;
use crate::infra::s3::Storage;
use archypix_common::routine::{RoutineStatus, TriggerAny};
use archypix_common::settings::Settings;
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

/// One monitored routine for the admin Routines tab (feature 23 §5.2): live status + a type-erased
/// "trigger with `Input::default()`" handle. Its tuning fields are discovered from the settings
/// registry (each field's `routine` link), so nothing is duplicated here.
pub struct RoutineEntry {
    pub name: &'static str,
    pub status: RoutineStatus,
    pub trigger: Arc<dyn TriggerAny>,
}

/// Registry of all spawned routines, read by `GET /api/admin/routines` and the trigger endpoint.
#[derive(Clone)]
pub struct RoutineRegistry {
    pub entries: Arc<Vec<RoutineEntry>>,
}

impl RoutineRegistry {
    pub fn empty() -> Self {
        Self {
            entries: Arc::new(Vec::new()),
        }
    }
}

/// Application state injected into every Axum handler via `State<AppState>`.
#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<Settings>,
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
    /// All spawned routines with live status + manual-trigger handles (feature 23 §5.2).
    pub routine_registry: RoutineRegistry,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        settings: Arc<Settings>,
        db: PgPool,
        cache: Arc<dyn Cache>,
        jwt: JwtService,
        worker_jwt: JwtService,
        storage: Arc<dyn Storage>,
        federation: FederationClient,
        resolver: ResolverClient,
        routines: Routines,
        routine_registry: RoutineRegistry,
    ) -> Self {
        Self {
            settings,
            db,
            cache,
            jwt,
            worker_jwt,
            storage,
            federation,
            resolver,
            routines,
            routine_registry,
        }
    }
}
