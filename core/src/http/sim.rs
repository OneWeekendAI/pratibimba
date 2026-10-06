//! The simulator's admin API (behind `PRB_ADMIN_TOKEN`, merged into `/api`). `{site}` is always
//! the REAL site; the sandbox it drives is `<site>~sim`. See `crate::sim`.
//!
//! - `POST   /api/sites/:site/sim`            create or wipe the sandbox; `{at?}` sets its clock
//! - `GET    /api/sites/:site/sim`            clock, counts and results as of the clock
//! - `DELETE /api/sites/:site/sim`            drop the sandbox
//! - `POST   /api/sites/:site/sim/visitors`   one cohort at the current clock
//! - `POST   /api/sites/:site/sim/advance`    `{days | seconds, cohort?}`: time-travel

use super::{ApiError, AppState};
use crate::sim::{self, Cohort};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/sites/{site}/sim", get(status).post(reset).delete(delete))
        .route("/sites/{site}/sim/visitors", post(visitors))
        .route("/sites/{site}/sim/advance", post(advance))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct Reset {
    at: Option<DateTime<Utc>>,
}

async fn reset(State(state): State<Arc<AppState>>, Path(site): Path<String>, Json(b): Json<Reset>) -> Result<Json<Value>, ApiError> {
    sim::reset(&state.pool, &site, b.at.unwrap_or_else(Utc::now)).await?;
    Ok(Json(sim::status(&state.pool, &site).await?))
}

async fn status(State(state): State<Arc<AppState>>, Path(site): Path<String>) -> Result<Json<Value>, ApiError> {
    Ok(Json(sim::status(&state.pool, &site).await?))
}

async fn delete(State(state): State<Arc<AppState>>, Path(site): Path<String>) -> Result<Json<Value>, ApiError> {
    if !sim::delete(&state.pool, &site).await? {
        return Err(ApiError::NotFound(format!("site {site:?} has no sandbox")));
    }
    Ok(Json(json!({ "deleted": sim::sandbox_id(&site) })))
}

async fn visitors(State(state): State<Arc<AppState>>, Path(site): Path<String>, Json(c): Json<Cohort>) -> Result<Json<Value>, ApiError> {
    let day = sim::visitors(&state.pool, &site, &c).await?;
    Ok(Json(serde_json::to_value(day).map_err(anyhow::Error::from)?))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Advance {
    days: Option<f64>,
    seconds: Option<i64>,
    cohort: Option<Cohort>,
}

async fn advance(State(state): State<Arc<AppState>>, Path(site): Path<String>, Json(b): Json<Advance>) -> Result<Json<Value>, ApiError> {
    let secs = match (b.days, b.seconds) {
        (Some(d), None) if d.is_finite() => (d * sim::DAY as f64).round() as i64,
        (None, Some(s)) => s,
        _ => return Err(ApiError::BadRequest("send exactly one of days or seconds".into())),
    };
    let days = sim::advance(&state.pool, &site, secs, b.cohort.as_ref()).await?;
    Ok(Json(json!({ "clock": sim::clock(&state.pool, &site).await?, "days": days })))
}
