use dam_creamy_cream_rust::{config::Config, infrastructure, router, state::AppState};
use sea_orm::{ConnectOptions, Database};
use std::time::Duration;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env()?;
    let mut options = ConnectOptions::new(&config.database_url);
    options
        .max_connections(10)
        .min_connections(1)
        .connect_timeout(Duration::from_secs(10));
    let db = Database::connect(options).await?;
    let redis = infrastructure::redis::connect(&config.redis_url).await?;
    let storage = infrastructure::storage::Storage::connect(&config).await?;
    let port = config.port;
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!(port, "creamy cream rust listening");
    axum::serve(
        listener,
        router(AppState {
            db,
            redis,
            storage,
            config,
        }),
    )
    .await?;
    Ok(())
}
