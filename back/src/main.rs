mod api;
mod clients;
mod domain;
mod infra;
mod repository;
mod services;
mod state;

use crate::clients::federation::FederationClient;
use crate::clients::resolver::ResolverClient;
use crate::infra::config::Config;
use crate::infra::crypto::JwtService;
use crate::infra::routine;
use crate::state::{AppState, Routines};
use reqwest::Client as HttpClient;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

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
    let config = Config::from_env()?;

    let _guard = infra::observability::init("archypix-back", config.back_domain.clone());

    info!("Starting Archypix Backend...");
    info!("Back domain:   {}", config.back_domain);
    info!("Global domain: {}", config.global_domain);
    info!("Database:      {}", config.database_url_masked());
    info!("Redis:         {}", config.redis_url_masked());

    // Surface an over-permissive CORS configuration at startup (07_security_audit.md §2.10).
    if config.cors_origins.iter().any(|o| o == "*") {
        tracing::warn!(
            "CORS is configured to allow ANY origin (CORS_ORIGINS contains '*'). This is intended \
             for development only. Pin CORS_ORIGINS to your frontend origin(s) in production."
        );
    }

    let db = infra::db::connect(&config).await?;
    infra::db::run_migrations(&db).await?;

    let redis = infra::redis::connect(&config).await?;
    let storage: Arc<dyn infra::s3::Storage> = Arc::new(infra::s3::connect(&config).await?);
    let http = HttpClient::new();

    let jwt = JwtService::new(&config.jwt_secret, &config.back_domain);
    let resolver_jwt = JwtService::new(&config.resolver_jwt_secret, &config.back_domain);
    let worker_jwt = JwtService::new(&config.worker_jwt_secret, &config.back_domain);

    let federation = FederationClient::new(
        http.clone(),
        config.clone(),
        jwt.clone(),
        Arc::new(redis.clone()),
    );
    let resolver = ResolverClient::new(http, config.clone(), resolver_jwt);

    // Register with the resolver so it can route user registrations to this backend.
    resolver.self_register().await?;

    // Cache handle, shared by the pipeline (same-backend resolution) and request handlers.
    let cache: Arc<dyn infra::redis::Cache> = Arc::new(redis);

    // ── Routine framework (feature 17) ──────────────────────────────────────────
    // `routine::spawn` spawns each runtime and returns its trigger handle plus a `JoinHandle`; we keep
    // the join handles so graceful shutdown can flip `shutdown_tx` and drain in-flight runs.
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut routine_joins = Vec::new();

    // Pipeline: wakes same-backend recipients via its own handle, wired into the routine after spawn.
    let pipeline = routine::pipeline::Pipeline::new(
        db.clone(),
        federation.clone(),
        cache.clone(),
        config.clone(),
    );
    let pipeline_cell = pipeline.handle_cell();
    let (pipeline_handle, pipeline_join) = routine::spawn(pipeline, shutdown_rx.clone());
    let _ = pipeline_cell.set(pipeline_handle.clone());
    routine_joins.push(pipeline_join);

    // Deferred-EXIF-job drain (feature 14 §5).
    let (exif_drain_handle, exif_drain_join) = routine::spawn(
        routine::exif_drain::ExifDrain::new(
            db.clone(),
            Duration::from_secs(config.exif_drain_interval_secs),
            config.exif_drain_batch,
        ),
        shutdown_rx.clone(),
    );
    routine_joins.push(exif_drain_join);

    // Tag-rename cascade (trigger-only) — wakes the pipeline to re-tag + re-announce.
    let (tag_rename_handle, tag_rename_join) = routine::spawn(
        routine::tag_rename::TagRename::new(
            db.clone(),
            pipeline_handle.clone(),
            config.task_queue_concurrency,
        ),
        shutdown_rx.clone(),
    );
    routine_joins.push(tag_rename_join);

    // Best-effort downstream unannounce (trigger-only) — wakes the recipient's pipeline.
    let (unannounce_handle, unannounce_join) = routine::spawn(
        routine::unannounce::Unannounce::new(
            db.clone(),
            federation.clone(),
            config.clone(),
            pipeline_handle.clone(),
        ),
        shutdown_rx.clone(),
    );
    routine_joins.push(unannounce_join);

    // Sweep-only routines (no external trigger handle): job watchdog, job cleanup, purge sweep.
    let (_jw, job_watchdog_join) = routine::spawn(
        routine::job_watchdog::JobWatchdogTask::new(
            db.clone(),
            config.job_processing_timeout_secs,
            Duration::from_secs(config.job_watchdog_interval_secs),
        ),
        shutdown_rx.clone(),
    );
    routine_joins.push(job_watchdog_join);
    let (_jc, job_cleanup_join) = routine::spawn(
        routine::job_watchdog::JobCleanupTask::new(
            db.clone(),
            config.job_retention_secs,
            Duration::from_secs(config.job_cleanup_interval_secs),
        ),
        shutdown_rx.clone(),
    );
    routine_joins.push(job_cleanup_join);
    let (_ps, purge_sweep_join) = routine::spawn(
        routine::purge_sweep::PurgeSweepTask::new(
            db.clone(),
            storage.clone(),
            cache.clone(),
            config.clone(),
            unannounce_handle.clone(),
            Duration::from_secs(config.purge_sweep_interval_secs),
            config.purge_sweep_batch,
        ),
        shutdown_rx.clone(),
    );
    routine_joins.push(purge_sweep_join);

    let routines = Routines {
        pipeline: pipeline_handle,
        exif_drain: exif_drain_handle,
        tag_rename: tag_rename_handle,
        unannounce: unannounce_handle,
    };

    let state = AppState::new(
        config.clone(),
        db,
        cache,
        jwt,
        worker_jwt,
        storage,
        federation,
        resolver,
        routines,
    );

    let listener = tokio::net::TcpListener::bind(&config.listen_addr).await?;
    info!("Listening on {}", config.listen_addr);

    use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
    use tower_http::trace::TraceLayer;

    const REQUEST_ID: http::HeaderName = http::HeaderName::from_static("x-request-id");

    let app = api::routes(&config)
        .layer(PropagateRequestIdLayer::new(REQUEST_ID.clone()))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|req: &http::Request<_>| -> tracing::Span {
                    let request_id = req
                        .headers()
                        .get("x-request-id")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("unknown")
                        .to_owned();
                    if req.uri().path() == "/health" {
                        return tracing::Span::none();
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
                })
                .on_response(record_response),
        )
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
