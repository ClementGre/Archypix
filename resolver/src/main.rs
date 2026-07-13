use archypix_resolver::config::setting_keys as sk;
use archypix_resolver::state::AppState;
use archypix_resolver::{api, config, routine, services};
use axum::http::HeaderValue;
use sqlx::postgres::PgPoolOptions;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,archypix_resolver=debug".into()),
        )
        .init();

    let config = config::load_env_only()?;
    info!("Starting Archypix Resolver");
    info!("Listen address: {}", config.get(sk::LISTEN_ADDR));
    info!("Global domain:  {}", config.get(sk::GLOBAL_DOMAIN));
    info!("Database:       {}", config::database_url_masked(&config));

    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config::database_url(&config))
        .await?;
    info!("Connected to database");
    sqlx::migrate!("./migrations").run(&db).await?;
    config::reload_from_db(&config, &db).await?;

    // Seed / verify the operator dashboard credential (feature 23 §5.1).
    services::operator::ensure_seeded(&db, &config).await?;

    // ── Routines (feature 23 §8.3) — spawn first so their status/trigger handles feed the registry ──
    use archypix_common::routine::{RoutineStatus, spawn_with_status};
    use archypix_resolver::state::{RoutineEntry, RoutineRegistry};
    use std::sync::Arc;

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut joins = Vec::new();
    let mut routine_entries: Vec<RoutineEntry> = Vec::new();

    let (prune_handle, prune_status, pj) = spawn_with_status(
        routine::StaleBackendPrune {
            db: db.clone(),
            config: config.clone(),
        },
        RoutineStatus::default(),
        shutdown_rx.clone(),
    );
    joins.push(pj);
    routine_entries.push(RoutineEntry {
        name: "stale_backend_prune",
        status: prune_status,
        trigger: Arc::new(prune_handle),
    });

    let (cleanup_handle, cleanup_status, cj) = spawn_with_status(
        routine::InviteCleanup {
            db: db.clone(),
            config: config.clone(),
        },
        RoutineStatus::default(),
        shutdown_rx.clone(),
    );
    joins.push(cj);
    routine_entries.push(RoutineEntry {
        name: "invite_cleanup",
        status: cleanup_status,
        trigger: Arc::new(cleanup_handle),
    });

    let (reconcile_handle, reconcile_status, rj) = spawn_with_status(
        routine::MappingReconcile {
            db: db.clone(),
            config: config.clone(),
            backends: archypix_resolver::clients::BackendClient::new(
                db.clone(),
                reqwest::Client::new(),
            ),
        },
        RoutineStatus::default(),
        shutdown_rx.clone(),
    );
    joins.push(rj);
    routine_entries.push(RoutineEntry {
        name: "mapping_reconcile",
        status: reconcile_status,
        trigger: Arc::new(reconcile_handle),
    });

    let routine_registry = RoutineRegistry {
        entries: Arc::new(routine_entries),
    };
    let state = AppState::new(db.clone(), config.clone(), routine_registry);

    // Dynamic CORS (feature 23 §4.4) — reads allowed origins from the live snapshot per request.
    let cors_config = config.clone();
    let cors = CorsLayer::new()
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_origin(AllowOrigin::predicate(move |origin: &HeaderValue, _| {
            let allowed = cors_config.get(sk::CORS_ORIGINS);
            allowed.iter().any(|o| o == "*")
                || origin
                    .to_str()
                    .map(|o| allowed.iter().any(|a| a == o))
                    .unwrap_or(false)
        }));

    // `cors` (dynamic, CORS_ORIGINS-gated) covers the register/admin/backend surface; the bootstrap
    // `info`/`resolve` routes get their own open CORS inside `api::routes` (feature 25).
    let app = api::routes(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listen_addr = config.get(sk::LISTEN_ADDR);
    let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
    info!("Server listening on {}", listen_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            let _ = shutdown_tx.send(true);
        })
        .await?;

    for j in joins {
        let _ = j.await;
    }
    Ok(())
}
