//! The HTTP surface.
//!
//! - **Public** (what owners proxy at `/_prb/*`): `/decide`, `/decide/batch`, `/e`,
//!   `/manifest/:site`, `/sdk.js`, `/health`. Browsers reach these through the site's own
//!   origin, so the decision path stays first-party. CORS exists only for dev setups where the
//!   page and Core sit on different origins (`PRB_SITE_ORIGIN`).
//! - **Webhooks** (`/webhook/stripe/:site`, `/webhook/conversion/:site`): signed platform
//!   conversions. See `webhooks`.
//! - **Admin** (`/api/*`): the dashboard's API, behind `PRB_ADMIN_TOKEN`. See `admin`.
//! - **MCP** (`/mcp`): the open MCP server for coding agents, behind per-site tokens. See `mcp`.
//! - **Dashboard** (`/app/*`): the static export of `dashboard/`, from `PRB_DASHBOARD_DIR`.

pub mod admin;
pub mod decide;
pub mod events;
pub mod hooks;
pub mod manifest;
pub mod sim;
pub mod webhooks;

use axum::{
    Json, Router,
    http::{HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use sqlx::PgPool;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower_http::cors::{AllowOrigin, CorsLayer};

pub use decide::{Ctx, DecideRequest, DecideResponse};

pub struct AppState {
    pub pool: PgPool,
    /// Source of per-decision seeds. OS entropy in production; fixed in tests so a cohort
    /// replays exactly.
    pub seeds: Mutex<ChaCha8Rng>,
    /// `PRB_ADMIN_TOKEN`. `None` turns the whole `/api` surface off.
    pub admin_token: Option<String>,
    /// Where the dashboard's static export lives. `None` serves no `/app`.
    pub dashboard_dir: Option<PathBuf>,
    /// For the install probes.
    pub http: reqwest::Client,
    /// How often `/api/sites/:id/stream` sends the overview.
    pub stream_every: Duration,
}

impl AppState {
    fn build(pool: PgPool, seeds: ChaCha8Rng) -> Self {
        AppState {
            pool,
            seeds: Mutex::new(seeds),
            admin_token: None,
            dashboard_dir: None,
            http: crate::probes::client(),
            stream_every: Duration::from_secs(3),
        }
    }

    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self::build(pool, ChaCha8Rng::from_os_rng()))
    }

    pub fn with_seed(pool: PgPool, seed: u64) -> Arc<Self> {
        Arc::new(Self::build(pool, ChaCha8Rng::seed_from_u64(seed)))
    }

    /// The full state for `serve`: an admin token and a dashboard directory, both optional.
    pub fn configured(pool: PgPool, seed: Option<u64>, admin_token: Option<String>, dashboard_dir: Option<PathBuf>) -> Arc<Self> {
        let rng = seed.map(ChaCha8Rng::seed_from_u64).unwrap_or_else(ChaCha8Rng::from_os_rng);
        let mut s = Self::build(pool, rng);
        s.admin_token = admin_token.filter(|t| !t.is_empty());
        s.dashboard_dir = dashboard_dir;
        Arc::new(s)
    }
}

pub fn router(state: Arc<AppState>, cors_origins: &[String]) -> Router {
    let mut app = Router::new()
        .route("/decide", post(decide::handler))
        .route("/decide/batch", post(decide::batch_handler))
        .route("/e", post(events::handler))
        .route("/manifest/{site}", get(manifest::handler))
        .route("/webhook/stripe/{site}", post(webhooks::stripe))
        .route("/webhook/conversion/{site}", post(webhooks::conversion))
        .route("/sdk.js", get(sdk_js))
        .route("/health", get(health))
        .route("/mcp", post(crate::mcp::handler))
        .nest("/api", admin::router(state.clone()));
    if let Some(dir) = state.dashboard_dir.clone() {
        // The export uses trailing slashes, so /app/slot/ is slot/index.html. Unknown paths get
        // the export's own 404 page.
        let not_found = dir.join("404.html");
        app = app
            .route("/app", get(|| async { axum::response::Redirect::permanent("/app/") }))
            .nest_service("/app/", tower_http::services::ServeDir::new(dir).not_found_service(tower_http::services::ServeFile::new(not_found)));
    }
    let app = app.with_state(state);
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
