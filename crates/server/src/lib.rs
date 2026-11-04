//! axum backend: auth + sync API, optional static hosting of the WASM app at `/app`.

pub mod auth;
pub mod config;
pub mod error;
pub mod security;
pub mod sync;

use axum::extract::{DefaultBodyLimit, State};
use axum::middleware;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use security::RateLimiter;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

pub use config::Config;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub cfg: Arc<Config>,
    pub limiter: Arc<RateLimiter>,
}

impl AppState {
    pub fn new(db: PgPool, cfg: Config) -> Self {
        Self {
            db,
            cfg: Arc::new(cfg),
            // 30 auth requests per minute per client address.
            limiter: Arc::new(RateLimiter::new(30, Duration::from_secs(60))),
        }
    }
}

/// Connect and run migrations.
pub async fn connect(cfg: &Config) -> Result<PgPool, sqlx::Error> {
    let pool = PgPoolOptions::new().max_connections(10).connect(&cfg.database_url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

async fn health(State(s): State<AppState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&s.db).await {
        Ok(_) => (axum::http::StatusCode::OK, Json(serde_json::json!({ "ok": true }))),
        Err(_) => (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({ "ok": false }))),
    }
}

pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .nest("/auth", auth::routes(state.clone()))
        .nest("/sync", sync::routes())
        .layer(middleware::from_fn_with_state(state.clone(), security::csrf_guard));

    let mut app = Router::new().nest("/api", api);
    if let Some(dir) = &state.cfg.web_dist {
        let index = ServeFile::new(format!("{dir}/index.html"));
        app = app.nest_service("/app", ServeDir::new(dir).fallback(index));
    }
    for layer in security::security_headers() {
        app = app.layer(layer);
    }
    app.layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(DefaultBodyLimit::max(16 * 1024 * 1024))
        .with_state(state)
}
