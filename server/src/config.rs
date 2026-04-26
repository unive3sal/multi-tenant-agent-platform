use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: String,
    pub rust_log: String,
    pub global_max_concurrency: usize,
    pub per_tenant_max_concurrency: usize,
    pub run_migrations: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            database_url: std::env::var("DATABASE_URL").context("DATABASE_URL is required")?,
            bind_addr: std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_owned()),
            rust_log: std::env::var("RUST_LOG").unwrap_or_else(|_| "info,sqlx=warn,hyper=warn".to_owned()),
            global_max_concurrency: parse_usize("GLOBAL_MAX_CONCURRENCY", 32)?,
            per_tenant_max_concurrency: parse_usize("PER_TENANT_MAX_CONCURRENCY", 8)?,
            run_migrations: parse_bool("RUN_MIGRATIONS", true),
        })
    }
}

fn parse_usize(key: &str, default: usize) -> Result<usize> {
    match std::env::var(key) {
        Ok(value) => value
            .parse::<usize>()
            .with_context(|| format!("{key} must be a positive integer")),
        Err(_) => Ok(default),
    }
}

fn parse_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(value) => matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"),
        Err(_) => default,
    }
}
