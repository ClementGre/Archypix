mod auth;
mod backend;
mod config;
mod error;
mod imaging;
mod jobs;
mod observability;

use backend::BackendClient;
use config::Config;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env before reading config so OTEL_EXPORTER_OTLP_ENDPOINT is available.
    dotenvy::dotenv().ok();
    let config = Config::from_env()?;

    let _guard = observability::init("archypix-worker", config.worker_id.clone());

    info!("Starting Archypix Worker...");

    info!("Worker ID:         {}", config.worker_id);
    info!(
        "Backends:          {}",
        config
            .backends
            .iter()
            .map(|b| b.back_domain.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    info!("Poll interval:     {}ms", config.poll_interval_ms);
    info!("Max concurrent:    {}", config.max_concurrent_jobs);
    info!("Job types:         {:?}", config.job_types);
    if imaging::video::tools_available() {
        info!("ffmpeg/ffprobe:    available");
    } else {
        warn!(
            "ffmpeg/ffprobe not found on PATH — video metadata extraction and thumbnails will be \
             skipped (install ffmpeg or rebuild the worker image)"
        );
    }
    if imaging::exif::exiftool_available() {
        info!("exiftool:          available (BMFF EXIF writes enabled)");
    } else {
        warn!(
            "exiftool not found on PATH — BMFF EXIF writes (HEIC/HEIF/AVIF) will fail until \
             exiftool is installed"
        );
    }

    // One semaphore shared across all backend loops — total concurrency is bounded globally.
    let sem = Arc::new(Semaphore::new(config.max_concurrent_jobs));
    let config = Arc::new(config);

    // Health check HTTP server (minimal, just for orchestration probes).
    let health_addr = config.listen_addr.clone();
    tokio::spawn(async move {
        run_health_server(&health_addr).await;
    });

    // Spawn one job-loop task per backend.  All loops compete for slots on the shared semaphore,
    // which gives natural fair-share allocation: a backend with many pending jobs saturates its
    // share of slots; a quiet backend yields its share without any explicit scheduler.
    let mut handles = Vec::with_capacity(config.backends.len());
    for backend_cfg in &config.backends {
        let client = Arc::new(BackendClient::new(&config, backend_cfg));
        let config_clone = config.clone();
        let sem_clone = sem.clone();
        handles.push(tokio::spawn(jobs::run_job_loop(
            config_clone,
            client,
            sem_clone,
        )));
    }

    // All loops run forever; join keeps main alive and surfaces panics.
    futures_util::future::join_all(handles).await;

    Ok(())
}

async fn run_health_server(addr: &str) {
    use axum::{Json, Router, routing::get};

    let app = Router::new().route(
        "/health",
        get(|| async {
            Json(serde_json::json!({
                "status": "healthy",
                "service": "archypix-worker"
            }))
        }),
    );

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(addr, error = ?e, "health server failed to bind");
            return;
        }
    };
    info!("Health server listening on {}", addr);
    if let Err(e) = axum::serve(listener, app).await {
        tracing::warn!(error = ?e, "health server error");
    }
}
