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
use crate::infra::tasks;
use crate::state::AppState;
use reqwest::Client as HttpClient;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,archypix_back=debug".into()),
        )
        .init();

    info!("Starting Archypix Backend...");

    let config = Config::from_env()?;

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
    let storage = infra::s3::connect(&config).await?;
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

    // Pipeline wake handle — shared by request handlers and the task runner (which wakes recipients
    // after same-backend announce/unannounce). The receiver is consumed by the pipeline loop below.
    // Built before the task queue to break the waker ↔ task_queue cycle.
    let (pipeline_waker, pipeline_rx) = infra::pipeline::channel();

    // Start the in-process background task queue (tag rename, share announce/unannounce).
    let (task_queue, task_runner) = tasks::create(
        db.clone(),
        federation.clone(),
        config.clone(),
        pipeline_waker.clone(),
        config.task_queue_concurrency,
    );
    tokio::spawn(task_runner);

    // Cache handle, shared by the pipeline (same-backend resolution) and request handlers.
    let cache: Arc<dyn infra::redis::Cache> = Arc::new(redis);

    // Periodic background tasks: stale-job watchdog, terminal-job cleanup, and the pipeline
    // recovery sweep (the pipeline loop itself is event-driven only). `shutdown_tx` is kept alive
    // for the lifetime of `main`; graceful shutdown is out of scope, so it is never signalled.
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut scheduler = infra::scheduler::Scheduler::new();
    scheduler
        .register(Arc::new(infra::job_watchdog::JobWatchdogTask::new(
            db.clone(),
            config.job_processing_timeout_secs,
            Duration::from_secs(config.job_watchdog_interval_secs),
        )))
        .register(Arc::new(infra::job_watchdog::JobCleanupTask::new(
            db.clone(),
            config.job_retention_secs,
            Duration::from_secs(config.job_cleanup_interval_secs),
        )))
        .register(Arc::new(infra::pipeline::PipelineRecoverySweepTask::new(
            db.clone(),
            pipeline_waker.clone(),
            Duration::from_secs(config.pipeline_poll_interval_secs),
        )));
    tokio::spawn(scheduler.run(shutdown_rx));

    // Start the tagging pipeline loop. Delivery is inline, so it holds the federation client, the
    // cache, and the waker (to wake same-backend recipients).
    tokio::spawn(infra::pipeline::create(
        db.clone(),
        pipeline_rx,
        config.clone(),
        config.pipeline_concurrency,
        federation.clone(),
        cache.clone(),
        pipeline_waker.clone(),
    ));

    let state = AppState::new(
        config.clone(),
        db,
        cache,
        jwt,
        worker_jwt,
        Arc::new(storage),
        federation,
        resolver,
        task_queue,
        pipeline_waker,
    );

    let listener = tokio::net::TcpListener::bind(&config.listen_addr).await?;
    info!("Listening on {}", config.listen_addr);

    let app = api::routes(&config)
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state);

    // `into_make_service_with_connect_info` exposes the peer `SocketAddr` to handlers via
    // `ConnectInfo`, used by the per-IP registration rate limiter.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}
