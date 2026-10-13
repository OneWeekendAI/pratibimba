//! `/api` routes for experiments ([`crate::experiments`]). Mounted inside `admin`, so every
//! route needs the admin token. backend.md's surface: `GET /sites/:id/experiments` and
//! `POST /experiments` (scoped to a site by the body); `POST /experiments/:id/stop` ends one early.

use super::{ApiError, AppState, admin::actor};
use crate::experiments::{self, New, NewArm};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/sites/{site}/experiments", get(list))
        .route("/experiments", post(create))
        .route("/experiments/{id}/stop", post(stop))
}

async fn list(State(state): State<Arc<AppState>>, Path(site): Path<String>) -> Result<Json<Value>, ApiError> {
    experiments::listing(&state.pool, &site).await.map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arm {
    variant: String,
    weight: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    site: String,
    slot: String,
    name: String,
    actor: String,
    arms: Vec<Arm>,
    #[serde(default)]
    metric: Option<String>,
    #[serde(default)]
    starts_at: Option<DateTime<Utc>>,
    stops_at: DateTime<Utc>,
}

async fn create(State(state): State<Arc<AppState>>, Json(b): Json<Create>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let who = actor(&b.actor)?;
    let site = b.site.clone();
    let new = New {
        site: b.site,
        slot: b.slot,
        name: b.name,
        metric: b.metric,
        arms: b.arms.into_iter().map(|a| NewArm { variant: a.variant, weight: a.weight }).collect(),
        starts_at: b.starts_at,
        stops_at: b.stops_at,
        actor: who,
    };
    let out = experiments::create(&state.pool, new).await?;
    crate::edge::changed(&state.pool, &state.edge, &site).await;
    Ok((StatusCode::CREATED, Json(out)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stop {
    actor: String,
}

async fn stop(State(state): State<Arc<AppState>>, Path(id): Path<i64>, Json(b): Json<Stop>) -> Result<Json<Value>, ApiError> {
    let who = actor(&b.actor)?;
    let out = experiments::stop(&state.pool, id, &who).await?;
    crate::edge::changed_by(&state.pool, &state.edge, "SELECT site_id FROM experiments WHERE id = $1", id).await;
    Ok(Json(out))
}
