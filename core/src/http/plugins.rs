//! `/api` routes for the plugin registry ([`crate::plugins`]). Mounted inside `admin`, so every
//! route needs the admin token. backend.md's surface: `GET /sites/:id/plugins` and
//! `POST /plugins/:name/install`, scoped to a site by the body; uninstall is its mirror.

use super::{ApiError, AppState, admin::actor};
use crate::plugins;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/sites/{site}/plugins", get(list))
        .route("/plugins/{name}/install", post(install))
        .route("/plugins/{name}/uninstall", post(uninstall))
}

async fn list(State(state): State<Arc<AppState>>, Path(site): Path<String>) -> Result<Json<Value>, ApiError> {
    plugins::listing(&state.pool, &site).await.map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Install {
    site: String,
    actor: String,
    #[serde(default)]
    config: Option<Value>,
}

async fn install(State(state): State<Arc<AppState>>, Path(name): Path<String>, Json(b): Json<Install>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let p = plugins::known(&name)?;
    let who = actor(&b.actor)?;
    let config = plugins::check_config(p, b.config.as_ref())?;
    let mut tx = state.pool.begin().await?;
    let out = plugins::install(&mut tx, &b.site, p, config, &who).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(out)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Uninstall {
    site: String,
    actor: String,
}

async fn uninstall(State(state): State<Arc<AppState>>, Path(name): Path<String>, Json(b): Json<Uninstall>) -> Result<Json<Value>, ApiError> {
    let who = actor(&b.actor)?;
    plugins::uninstall(&state.pool, &b.site, &name, &who).await.map(Json)
}
