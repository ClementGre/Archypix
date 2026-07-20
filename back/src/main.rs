use archypix_back::clients::federation::FederationClient;
use archypix_back::clients::resolver::ResolverClient;
use archypix_back::infra;
use archypix_back::infra::crypto::JwtService;
use archypix_back::infra::redis::Cache;
use archypix_back::infra::routine;
use archypix_back::infra::routine::exif_drain::ExifDrainRoutine;
use archypix_back::infra::routine::job_watchdog::{JobCleanupRoutine, JobWatchdogRoutine};
use archypix_back::infra::routine::pipeline::PipelineRoutine;
use archypix_back::infra::routine::purge_sweep::PurgeSweepRoutine;
use archypix_back::infra::routine::resolver_heartbeat::ResolverHeartbeatRoutine;
use archypix_back::infra::routine::storage_reconcile::StorageReconcileRoutine;
use archypix_back::infra::routine::tag_rename::TagRenameRoutine;
use archypix_back::infra::routine::unannounce::UnannounceRoutine;
use archypix_back::infra::s3::Storage;
use archypix_back::infra::settings::keys;
use archypix_back::state::RoutineEntry;
use archypix_back::state::{AppState, RoutineRegistry, Routines};
use archypix_common::routine::RoutineStatus;
use archypix_common::settings::Settings;
use axum::extract::Request;
use axum::response::Response;
use reqwest::Client as HttpClient;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::watch::Receiver;
use tokio::task::JoinHandle;
use tower_http::trace::{DefaultOnRequest, HttpMakeClassifier, TraceLayer};
use tracing::{info, Span};

/// `TraceLayer` `on_response` hook: records the status and marks 5xx as an OTel error.
/// See `doc/features/12_observability_tracing.md` §3.2.
fn record_response<B>(
    response: &http::Response<B>,
    _latency: std::time::Duration,
    span: &tracing::Span,
) {
    let status = response.status();
    span.record("http.response.status_code", status.as_u16());
    if status.is_server_error() {
        span.record("otel.status_code", "ERROR");
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use archypix_back::infra::settings::{self, keys};
    dotenvy::dotenv().ok();
    let settings = settings::load_env_only()?;

    let _guard = infra::observability::init("archypix-back", settings.get(keys::BACK_DOMAIN));

    info!("Starting Archypix Backend...");
    info!("Back domain:   {}", settings.get(keys::BACK_DOMAIN));
    info!("Global domain: {}", settings.get(keys::GLOBAL_DOMAIN));
    info!(
        "Database:      {}",
        settings::database_url_masked(&settings)
    );
    info!("Redis:         {}", settings::redis_url_masked(&settings));

    let db = infra::db::connect(&settings).await?;
    infra::db::run_migrations(&db).await?;
    settings::reload_from_db(&settings, &db).await?;

    if settings.get(keys::CORS_ORIGINS).iter().any(|o| o == "*") {
        tracing::warn!(
            "CORS is configured to allow ANY origin (CORS_ORIGINS contains '*'). This is intended \
             for development only. Pin CORS_ORIGINS to your frontend origin(s) in production."
        );
    }

    let redis = infra::redis::connect(&settings).await?;
    let storage: Arc<dyn infra::s3::Storage> = Arc::new(infra::s3::connect(&settings).await?);
    let http = HttpClient::new();
    // Federation gets a bounded client (feature 28 §4.1): a *down* peer fails fast on connect, a
    // *slow* peer is bounded by the overall request timeout. Covers every outbound federation call
    // (incl. backend resolution).
    let federation_http = HttpClient::builder()
        .connect_timeout(std::time::Duration::from_millis(
            settings.get(keys::FEDERATION_CONNECT_TIMEOUT_MS),
        ))
        .timeout(std::time::Duration::from_millis(
            settings.get(keys::FEDERATION_REQUEST_TIMEOUT_MS),
        ))
        .build()
        .unwrap_or_else(|_| HttpClient::new());

    let jwt = JwtService::new(
        &settings.get(keys::JWT_SECRET),
        &settings.get(keys::BACK_DOMAIN),
    );
    let resolver_jwt = JwtService::new(
        &settings.get(keys::RESOLVER_JWT_SECRET).unwrap_or_default(),
        &settings.get(keys::BACK_DOMAIN),
    );
    let worker_jwt = JwtService::new(
        &settings.get(keys::WORKER_JWT_SECRET),
        &settings.get(keys::BACK_DOMAIN),
    );

    let federation = FederationClient::new(
        federation_http,
        settings.clone(),
        jwt.clone(),
        Arc::new(redis.clone()),
    );
    let resolver = ResolverClient::new(http, settings.clone(), resolver_jwt, jwt.clone());

    // Register with the resolver so it can route user registrations to this backend.
    resolver.self_register().await?;

    // Cache handle, shared by the pipeline (same-backend resolution) and request handlers.
    let cache: Arc<dyn infra::redis::Cache> = Arc::new(redis);

    // ── Routine framework (feature 17) ──────────────────────────────────────────
    // `routine::spawn` spawns each runtime and returns its trigger handle plus a `JoinHandle`; we keep
    // the join handles so graceful shutdown can flip `shutdown_tx` and drain in-flight runs.
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut routine_joins = Vec::new();

    let (routines, routine_registry) = start_routines(
        &settings,
        &db,
        &storage,
        &federation,
        &resolver,
        &cache,
        shutdown_rx,
        &mut routine_joins,
    );

    let state = AppState::new(
        settings.clone(),
        db,
        cache,
        jwt,
        worker_jwt,
        storage,
        federation,
        resolver,
        routines,
        routine_registry,
    );

    let listener = tokio::net::TcpListener::bind(&settings.get(keys::LISTEN_ADDR)).await?;
    info!("Listening on {}", settings.get(keys::LISTEN_ADDR));

    use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
    const REQUEST_ID: http::HeaderName = http::HeaderName::from_static("x-request-id");

    let app = archypix_back::api::routes(state.settings.clone())
        .layer(PropagateRequestIdLayer::new(REQUEST_ID.clone()))
        .layer(trace_layer())
        .layer(SetRequestIdLayer::new(REQUEST_ID.clone(), MakeRequestUuid))
        .with_state(state);

    // `into_make_service_with_connect_info` exposes the peer `SocketAddr` to handlers via
    // `ConnectInfo`, used by the per-IP registration rate limiter.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    // Server stopped accepting connections (SIGINT/SIGTERM). Tell the routines to stop and drain any
    // in-flight runs before exiting.
    info!("Shutdown signal received; stopping background routines...");
    let _ = shutdown_tx.send(true);
    for join in routine_joins {
        let _ = join.await;
    }
    info!("Shutdown complete.");
    Ok(())
}

fn trace_layer() -> TraceLayer<
    HttpMakeClassifier,
    fn(&Request) -> Span,
    DefaultOnRequest,
    fn(&Response, std::time::Duration, &Span),
> {
    let make_span: fn(&Request) -> Span = |req: &http::Request<_>| -> Span {
        let request_id = req
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_owned();
        if req.uri().path() == "/health" {
            return Span::none();
        }
        // Operation name is the matched route (not the concrete path) to bound
        // cardinality; unmatched requests fold into `{METHOD} <unmatched>`. Field names
        // are OTel HTTP semconv. Rationale in doc/features/12_observability_tracing.md §3.2.
        let route = req
            .extensions()
            .get::<axum::extract::MatchedPath>()
            .map(|m| m.as_str());
        let otel_name = match route {
            Some(route) => format!("{} {}", req.method(), route),
            None => format!("{} <unmatched>", req.method()),
        };
        let client_addr = req
            .extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|c| c.0.ip().to_string())
            .unwrap_or_default();
        let server_addr = req
            .headers()
            .get(http::header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        tracing::info_span!(
            "http_request",
            "otel.name" = otel_name,
            "otel.kind" = "server",
            "http.request.method" = %req.method(),
            "http.route" = route.unwrap_or("<unmatched>"),
            "url.path" = %req.uri().path(),
            "client.address" = %client_addr,
            "server.address" = %server_addr,
            "http.response.status_code" = tracing::field::Empty,
            "otel.status_code" = tracing::field::Empty,
            "enduser.id" = tracing::field::Empty,
            request_id = %request_id,
        )
    };
    TraceLayer::new_for_http()
        .make_span_with(make_span)
        .on_response(record_response)
}

fn start_routines(
    settings: &Arc<Settings>,
    db: &PgPool,
    storage: &Arc<dyn Storage>,
    federation: &FederationClient,
    resolver: &ResolverClient,
    cache: &Arc<dyn Cache>,
    shutdown_rx: Receiver<bool>,
    routine_joins: &mut Vec<JoinHandle<()>>,
) -> (Routines, RoutineRegistry) {
    // Registry of spawned routines for the admin Routines tab (feature 23 §5.2): each `spawn_with_status`
    // yields a live status handle + a trigger handle we type-erase into `RoutineEntry`.
    let mut routine_entries: Vec<RoutineEntry> = Vec::new();

    // Pipeline: wakes same-backend recipients via its own handle, wired into the routine after spawn.
    let pipeline = PipelineRoutine::new(
        db.clone(),
        federation.clone(),
        cache.clone(),
        settings.clone(),
    );
    let pipeline_cell = pipeline.handle_cell();
    let (pipeline_handle, pipeline_status, pipeline_join) =
        routine::spawn_with_status(pipeline, RoutineStatus::default(), shutdown_rx.clone());
    let _ = pipeline_cell.set(pipeline_handle.clone());
    routine_joins.push(pipeline_join);
    routine_entries.push(RoutineEntry {
        name: "pipeline",
        status: pipeline_status,
        trigger: Arc::new(pipeline_handle.clone()),
    });

    // Deferred-EXIF-job drain (feature 14 §5).
    let (exif_drain_handle, exif_drain_status, exif_drain_join) = routine::spawn_with_status(
        ExifDrainRoutine::new(db.clone(), settings.clone()),
        RoutineStatus::default(),
        shutdown_rx.clone(),
    );
    routine_joins.push(exif_drain_join);
    routine_entries.push(RoutineEntry {
        name: "exif_drain",
        status: exif_drain_status,
        trigger: Arc::new(exif_drain_handle.clone()),
    });

    // Tag-rename cascade (trigger-only) — wakes the pipeline to re-tag + re-announce.
    let (tag_rename_handle, tag_rename_status, tag_rename_join) = routine::spawn_with_status(
        TagRenameRoutine::new(
            db.clone(),
            pipeline_handle.clone(),
            settings.clone(),
        ),
        RoutineStatus::default(),
        shutdown_rx.clone(),
    );
    routine_joins.push(tag_rename_join);
    routine_entries.push(RoutineEntry {
        name: "tag_rename",
        status: tag_rename_status,
        trigger: Arc::new(tag_rename_handle.clone()),
    });

    // Best-effort downstream unannounce (trigger-only) — wakes the recipient's pipeline.
    let (unannounce_handle, unannounce_status, unannounce_join) = routine::spawn_with_status(
        UnannounceRoutine::new(
            db.clone(),
            federation.clone(),
            settings.clone(),
            pipeline_handle.clone(),
        ),
        RoutineStatus::default(),
        shutdown_rx.clone(),
    );
    routine_joins.push(unannounce_join);
    routine_entries.push(RoutineEntry {
        name: "unannounce",
        status: unannounce_status,
        trigger: Arc::new(unannounce_handle.clone()),
    });

    // Sweep-only routines: job watchdog, job cleanup, purge sweep, storage reconcile.
    let (jw_handle, jw_status, job_watchdog_join) = routine::spawn_with_status(
        JobWatchdogRoutine::new(db.clone(), settings.clone()),
        RoutineStatus::default(),
        shutdown_rx.clone(),
    );
    routine_joins.push(job_watchdog_join);
    routine_entries.push(RoutineEntry {
        name: "job_watchdog",
        status: jw_status,
        trigger: Arc::new(jw_handle),
    });
    let (jc_handle, jc_status, job_cleanup_join) = routine::spawn_with_status(
        JobCleanupRoutine::new(db.clone(), settings.clone()),
        RoutineStatus::default(),
        shutdown_rx.clone(),
    );
    routine_joins.push(job_cleanup_join);
    routine_entries.push(RoutineEntry {
        name: "job_cleanup",
        status: jc_status,
        trigger: Arc::new(jc_handle),
    });
    let (ps_handle, ps_status, purge_sweep_join) = routine::spawn_with_status(
        PurgeSweepRoutine::new(
            db.clone(),
            storage.clone(),
            cache.clone(),
            unannounce_handle.clone(),
            settings.clone(),
        ),
        RoutineStatus::default(),
        shutdown_rx.clone(),
    );
    routine_joins.push(purge_sweep_join);
    routine_entries.push(RoutineEntry {
        name: "purge_sweep",
        status: ps_status,
        trigger: Arc::new(ps_handle),
    });
    let (sr_handle, sr_status, storage_reconcile_join) = routine::spawn_with_status(
        StorageReconcileRoutine::new(db.clone(), cache.clone(), settings.clone()),
        RoutineStatus::default(),
        shutdown_rx.clone(),
    );
    routine_joins.push(storage_reconcile_join);
    routine_entries.push(RoutineEntry {
        name: "storage_reconcile",
        status: sr_status,
        trigger: Arc::new(sr_handle),
    });

    // Resolver heartbeat (feature 23 §3.2) — pushes a fresh delegation token + fleet metrics. Only
    // when a resolver is configured; a standalone backend needs none.
    if settings.get(keys::USE_RESOLVER) {
        let (hb_handle, hb_status, heartbeat_join) = routine::spawn_with_status(
            ResolverHeartbeatRoutine::new(db.clone(), resolver.clone(), settings.clone()),
            RoutineStatus::default(),
            shutdown_rx.clone(),
        );
        routine_joins.push(heartbeat_join);
        routine_entries.push(RoutineEntry {
            name: "resolver_heartbeat",
            status: hb_status,
            trigger: Arc::new(hb_handle),
        });
    }

    let routines = Routines {
        pipeline: pipeline_handle,
        exif_drain: exif_drain_handle,
        tag_rename: tag_rename_handle,
        unannounce: unannounce_handle,
    };
    let routine_registry = RoutineRegistry {
        entries: Arc::new(routine_entries),
    };
    (routines, routine_registry)
}

/// Resolves on the first SIGINT (Ctrl-C) or SIGTERM, driving graceful shutdown.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
