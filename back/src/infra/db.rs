use crate::infra::settings;
use archypix_common::settings::Settings;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::info;

pub async fn connect(settings: &Arc<Settings>) -> anyhow::Result<PgPool> {
    info!(
        "Connecting to database: {}",
        settings::database_url_masked(&settings)
    );
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&settings::database_url(&settings))
        .await?;
    info!("Connected to database");
    Ok(pool)
}

pub async fn run_migrations(pool: &PgPool) -> anyhow::Result<()> {
    info!("Running database migrations...");
    sqlx::migrate!("./migrations").run(pool).await?;
    info!("Migrations complete");
    Ok(())
}
