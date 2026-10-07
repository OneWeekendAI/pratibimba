//! `/api` routes for privacy (mounted inside `admin`, so every route needs the admin token):
//!
//! - `GET /api/sites/{site}/privacy`, `POST /api/sites/{site}/privacy`: the consent mode and
//!   the `fields_allowed` list. A POST changes only what it names; `"fields_allowed": null`
//!   clears the list (every field is collected).
//! - `POST /api/sites/{site}/erase` `{subject, actor, reason?}`: right-to-erase.
//! - `GET /api/sites/{site}/erasures`: the erase audit log (hashes, never subjects).
//! - `GET /api/sites/{site}/subjects/{subject}`: right-of-access, everything held for a subject.

use super::{ApiError, AppState};
use crate::privacy;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/sites/{site}/privacy", get(read).post(write))
        .route("/sites/{site}/erase", post(erase))
        .route("/sites/{site}/erasures", get(erasures))
        .route("/sites/{site}/subjects/{subject}", get(access))
}

async fn read(State(state): State<Arc<AppState>>, Path(site): Path<String>) -> Result<Json<Value>, ApiError> {
    let Some(p) = privacy::policy(&state.pool, &site).await? else { return Err(ApiError::NotFound(format!("unknown site {site:?}"))) };
    Ok(Json(json!({ "site": site, "privacy_mode": p.mode, "fields_allowed": p.fields, "modes": privacy::MODES, "fields": privacy::FIELDS })))
}

/// Tells "absent" (keep) from `null` (clear) for `fields_allowed`.
fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<Vec<String>>>, D::Error> {
    Option::<Vec<String>>::deserialize(d).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetPrivacy {
    #[serde(default)]
    privacy_mode: Option<String>,
    #[serde(default, deserialize_with = "present")]
    fields_allowed: Option<Option<Vec<String>>>,
}

async fn write(State(state): State<Arc<AppState>>, Path(site): Path<String>, Json(b): Json<SetPrivacy>) -> Result<Json<Value>, ApiError> {
    privacy::set(&state.pool, &site, b.privacy_mode.as_deref(), b.fields_allowed).await.map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Erase {
    subject: String,
    actor: String,
    #[serde(default)]
    reason: Option<String>,
}

async fn erase(State(state): State<Arc<AppState>>, Path(site): Path<String>, Json(b): Json<Erase>) -> Result<Json<Value>, ApiError> {
    privacy::erase(&state.pool, &site, &b.subject, &b.actor, b.reason.as_deref()).await.map(Json)
}

async fn erasures(State(state): State<Arc<AppState>>, Path(site): Path<String>) -> Result<Json<Value>, ApiError> {
    if privacy::policy(&state.pool, &site).await?.is_none() {
        return Err(ApiError::NotFound(format!("unknown site {site:?}")));
    }
    Ok(Json(json!({ "erasures": privacy::erasures(&state.pool, &site).await? })))
}

async fn access(State(state): State<Arc<AppState>>, Path((site, subject)): Path<(String, String)>) -> Result<Json<Value>, ApiError> {
    privacy::access(&state.pool, &site, &subject).await.map(Json)
}
