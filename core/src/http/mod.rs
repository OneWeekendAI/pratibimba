//! The HTTP surface: `/decide`, `/e`, `/manifest/:site`, `/sdk.js`, `/health`.
//!
//! Browsers reach these through the site's own origin (`/_prb/*` proxied to Core), so the
//! decision path stays first-party. CORS exists only for dev setups where the page and Core
//! sit on different origins (`PRB_SITE_ORIGIN`).

pub mod decide;
pub mod events;
pub mod manifest;

use axum::{
    Json, Router,
    http::{HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
use tower_http::cors::{AllowOrigin, CorsLayer};

pub use decide::{Ctx, DecideRequest, DecideResponse};

pub struct AppState {
    pub pool: PgPool,
    /// Source of per-decision seeds. OS entropy in production; fixed in tests so a cohort
    /// replays exactly.
    pub seeds: Mutex<ChaCha8Rng>,
}

impl AppState {
    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(AppState { pool, seeds: Mutex::new(ChaCha8Rng::from_os_rng()) })
    }

    pub fn with_seed(pool: PgPool, seed: u64) -> Arc<Self> {
        Arc::new(AppState { pool, seeds: Mutex::new(ChaCha8Rng::seed_from_u64(seed)) })
    }
}

pub fn router(state: Arc<AppState>, cors_origins: &[String]) -> Router {
    let app = Router::new()
        .route("/decide", post(decide::handler))
        .route("/e", post(events::handler))
        .route("/manifest/{site}", get(manifest::handler))
        .route("/sdk.js", get(sdk_js))
        .route("/health", get(health))
        .with_state(state);
    let origins: Vec<HeaderValue> = cors_origins.iter().filter_map(|o| o.trim().parse().ok()).collect();
    if origins.is_empty() {
        return app;
    }
    app.layer(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods([Method::GET, Method::POST])
            .allow_headers([header::CONTENT_TYPE]),
    )
}

pub const SDK_JS: &str = include_str!("../../../packages/client/sdk.js");

async fn sdk_js() -> Response {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8"), (header::CACHE_CONTROL, "public, max-age=300")],
        SDK_JS,
    )
        .into_response()
}

async fn health(axum::extract::State(state): axum::extract::State<Arc<AppState>>) -> Response {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (StatusCode::OK, "ok").into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "database unreachable").into_response(),
    }
}

/// Errors the handlers return; each maps to one status and a JSON `{error}` body.
#[derive(Debug)]
pub enum ApiError {
    NotFound(String),
    BadRequest(String),
    Conflict(String),
    Internal(anyhow::Error),
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError::Internal(e.into())
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::Internal(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, m),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, m),
            ApiError::Internal(e) => {
                tracing::error!(error = %e, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
            }
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

/// A subject is an opaque visitor id: 1-256 printable ASCII characters.
pub fn valid_subject(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && s.bytes().all(|b| b.is_ascii_graphic())
}
