use crate::config::Config;
use axum::{Router, routing::{delete, get, post}};
use sqlx::PgPool;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, Semaphore};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

pub struct AppState {
    pub db: PgPool,
    pub config: Config,
    pub global_semaphore: Arc<Semaphore>,
    pub tenant_semaphores: Arc<Mutex<HashMap<Uuid, Arc<Semaphore>>>>,
}

impl AppState {
    pub fn new(db: PgPool, config: Config) -> Self {
        Self {
            db,
            global_semaphore: Arc::new(Semaphore::new(config.global_max_concurrency)),
            tenant_semaphores: Arc::new(Mutex::new(HashMap::new())),
            config,
        }
    }
}

pub fn build_app(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(crate::http::healthz))
        .route("/tenants", post(crate::http::create_tenant))
        .route(
            "/tenants/{tenant_id}/api-keys",
            post(crate::http::create_api_key),
        )
        .route("/tools", post(crate::http::create_tool).get(crate::http::list_tools))
        .route("/tools/{tool_id}", delete(crate::http::delete_tool))
        .route("/agents", post(crate::http::create_agent))
        .route("/agents/{agent_id}", get(crate::http::get_agent))
        .route("/agents/{agent_id}/run", post(crate::http::start_run))
        .route("/runs/{run_id}", get(crate::http::get_run))
        .route("/runs/{run_id}/trace", get(crate::http::get_trace))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
