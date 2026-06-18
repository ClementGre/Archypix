mod config;
mod database;
mod error;
mod handler;

use crate::database::init_database;
use crate::handler::{
    health_handler, list_backends_handler, register_backend_handler, register_handler,
    update_handler, webfinger_handler,
};
use axum::{
    Router,
    http::HeaderValue,
    routing::{get, post},
};
use config::Config;
use moka::future::Cache;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

#[derive(Clone)]
struct AppState {
    db: PgPool,
    cache: Cache<String, String>,
    global_domain: String,
    resolver_jwt_secret: String,
    reqwest_client: reqwest::Client,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,archypix_resolver=debug".into()),
        )
        .init();

    let config = Config::from_env()?;
    info!("Starting Archypix Resolver");
    info!("Listen address:    {}", config.listen_addr);
    info!("Global domain:     {}", config.global_domain);
    info!("Database:          {}", config.database_url_masked());
    info!("Cache TTL:         {}s", config.cache_ttl_secs);
    info!("Cache max entries: {}", config.cache_max_capacity);

    let db_pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url())
        .await?;
    info!("Connected to database");

    init_database(&db_pool).await?;

    let cache = Cache::builder()
        .time_to_live(Duration::from_secs(config.cache_ttl_secs))
        .max_capacity(config.cache_max_capacity)
        .build();

    let reqwest_client = reqwest::Client::new();

    let state = AppState {
        db: db_pool,
        global_domain: config.global_domain,
        cache,
        resolver_jwt_secret: config.resolver_jwt_secret,
        reqwest_client,
    };

    let allow_origin = build_cors_origin(&config.cors_origins);
    let cors = CorsLayer::new()
        .allow_methods(Any)
        .allow_origin(allow_origin)
        .allow_headers(Any);

    let app = Router::new()
        .route("/.well-known/webfinger", get(webfinger_handler))
        .route("/api/update", post(update_handler))
        // Mirrors the standalone backend's registration route, so the frontend uses one
        // URL regardless of whether the global domain is a resolver or a standalone backend.
        .route("/api/public/register", post(register_handler))
        .route(
            "/api/backends",
            post(register_backend_handler).get(list_backends_handler),
        )
        .route("/health", get(health_handler))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.listen_addr).await?;
    info!("Server listening on {}", config.listen_addr);

    axum::serve(listener, app).await?;
    Ok(())
}

fn build_cors_origin(origins: &[String]) -> tower_http::cors::AllowOrigin {
    if origins.iter().any(|o| o == "*") {
        tower_http::cors::AllowOrigin::any()
    } else {
        let list: Vec<HeaderValue> = origins
            .iter()
            .filter_map(|o| o.parse::<HeaderValue>().ok())
            .collect();
        tower_http::cors::AllowOrigin::list(list)
    }
}
