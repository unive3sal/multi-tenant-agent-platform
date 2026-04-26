use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use std::{path::Path, sync::Arc};
use tokio::net::TcpListener;
use tracing::info;

mod app;
mod auth;
mod config;
mod db;
mod error;
mod http;
mod runtime;
mod trace;

use app::{AppState, build_app};
use config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    init_tracing(&config.rust_log);

    let db = PgPoolOptions::new()
        .max_connections(20)
        .connect(&config.database_url)
        .await
        .context("failed to connect to postgres")?;

    if config.run_migrations {
        let migrator = sqlx::migrate::Migrator::new(Path::new("server/migrations"))
            .await
            .context("failed to load migrations")?;
        migrator
            .run(&db)
            .await
            .context("failed to run migrations")?;
    }

    let state = Arc::new(AppState::new(db, config.clone()));
    let app = build_app(state);
    let listener = TcpListener::bind(&config.bind_addr)
        .await
        .with_context(|| format!("failed to bind {}", config.bind_addr))?;

    info!(bind_addr = %config.bind_addr, "platform server listening");
    axum::serve(listener, app).await.context("server failed")?;
    Ok(())
}

fn init_tracing(rust_log: &str) {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(rust_log.to_owned())),
        )
        .with_target(false)
        .compact()
        .init();
}
