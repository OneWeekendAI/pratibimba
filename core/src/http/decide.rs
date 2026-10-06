//! `POST /decide`: one decision for one slot.
//!
//! The context is a closed set of named booleans. `deny_unknown_fields` is the Phase 0 feature
//! allowlist: a `gender`, `age` or any other undeclared name is refused at the boundary rather
//! than silently ignored (backend.md, "No demographic inference").

use super::{ApiError, AppState, valid_subject};
use crate::bandit::{self, LinTs, Vector};
use axum::{Json, extract::State};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ctx {
    #[serde(default)]
    pub returning: bool,
    #[serde(default)]
    pub mobile: bool,
    #[serde(default)]
    pub from_ad: bool,
}

impl Ctx {
    pub fn vector(&self) -> Vector {
        let b = |v: bool| if v { 1.0 } else { 0.0 };
        [1.0, b(self.returning), b(self.mobile), b(self.from_ad)]
    }
}

/// Named features, as stored on the decision and shown in explanations.
pub fn named(x: &Vector) -> Value {
    let mut m = Map::new();
    for (name, v) in bandit::FEATURE_NAMES.iter().zip(x) {
        m.insert(name.to_string(), json!(v));
    }
    Value::Object(m)
}

/// Rebuild the vector from stored named features. Every name must be present.
pub fn from_named(v: &Value) -> Option<Vector> {
    let mut x = [0.0; bandit::DIM];
    for (i, name) in bandit::FEATURE_NAMES.iter().enumerate() {
        x[i] = v.get(*name)?.as_f64()?;
    }
    Some(x)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecideRequest {
    pub site: String,
    pub slot: String,
    pub subject: String,
    #[serde(default)]
    pub ctx: Ctx,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DecideResponse {
    pub decision_id: Uuid,
    pub slot: String,
    pub variant: String,
    /// HTML to swap in; `null` means keep the page's own content (the baseline).
    pub payload: Option<String>,
    pub explanation: Value,
}

pub async fn handler(State(state): State<Arc<AppState>>, Json(req): Json<DecideRequest>) -> Result<Json<DecideResponse>, ApiError> {
    decide(&state, &req).await.map(Json)
}

pub async fn decide(state: &AppState, req: &DecideRequest) -> Result<DecideResponse, ApiError> {
    if !valid_subject(&req.subject) {
        return Err(ApiError::BadRequest("subject must be 1-256 printable ASCII characters".into()));
    }
    let pool = &state.pool;
    let slot: Option<(i64, i32)> = sqlx::query_as("SELECT id, decision_ttl_secs FROM slots WHERE site_id = $1 AND key = $2")
        .bind(&req.site)
        .bind(&req.slot)
        .fetch_optional(pool)
        .await?;
    let Some((slot_id, ttl)) = slot else {
        return Err(ApiError::NotFound(format!("unknown slot {:?} on site {:?}", req.slot, req.site)));
    };
    let arms: Vec<(i64, String, Option<String>)> =
        sqlx::query_as("SELECT id, key, payload FROM variants WHERE slot_id = $1 AND status = 'approved' ORDER BY id")
            .bind(slot_id)
            .fetch_all(pool)
            .await?;
    if arms.is_empty() {
        return Err(ApiError::Conflict(format!("slot {:?} has no approved variants", req.slot)));
    }
    let key_of = |id: i64| arms.iter().find(|a| a.0 == id).map(|a| (a.1.clone(), a.2.clone()));

    // Sticky: a reload inside the TTL gets the same decision, so the hero does not reroll.
    let recent: Option<(Uuid, i64, Value, Value, i64, i64)> = sqlx::query_as(
        "SELECT id, variant_id, features, scores, seed, policy_version FROM decisions
          WHERE slot_id = $1 AND subject = $2 AND created_at > now() - make_interval(secs => $3)
          ORDER BY created_at DESC LIMIT 1",
    )
    .bind(slot_id)
    .bind(&req.subject)
    .bind(ttl as f64)
    .fetch_optional(pool)
    .await?;
    if let Some((id, variant_id, features, scores, seed, version)) = recent {
        if let Some((key, payload)) = key_of(variant_id) {
            return Ok(response(id, &req.slot, key, payload, version, features, &scores, seed as u64, &arms));
        }
    }

    let (version, state_json): (i64, Value) =
        sqlx::query_as("SELECT version, state FROM models WHERE slot_id = $1").bind(slot_id).fetch_one(pool).await?;
    let model: LinTs = serde_json::from_value(state_json).map_err(anyhow::Error::from)?;
    let x = req.ctx.vector();
    let seed = state.seeds.lock().expect("seed rng poisoned").next_u64();
    let ids: Vec<i64> = arms.iter().map(|a| a.0).collect();
    let (chosen, scores) = model.choose(&ids, &x, seed);
    let scores_json = Value::Object(scores.iter().map(|(id, s)| (id.to_string(), json!(s))).collect());
    let features = named(&x);
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO decisions (id, site_id, slot_id, variant_id, subject, pool, features, scores, policy, policy_version, seed)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(id)
    .bind(&req.site)
    .bind(slot_id)
    .bind(chosen)
    .bind(&req.subject)
    .bind(&ids)
    .bind(&features)
    .bind(&scores_json)
    .bind(bandit::POLICY)
    .bind(version)
    .bind(seed as i64)
    .execute(pool)
    .await?;
    let (key, payload) = key_of(chosen).expect("chosen arm is in the pool");
    Ok(response(id, &req.slot, key, payload, version, features, &scores_json, seed, &arms))
}

#[allow(clippy::too_many_arguments)]
fn response(
    id: Uuid,
    slot: &str,
    variant: String,
    payload: Option<String>,
    version: i64,
    features: Value,
    scores_by_id: &Value,
    seed: u64,
    arms: &[(i64, String, Option<String>)],
) -> DecideResponse {
    // Explanations name variants by key; storage keys them by id so a rename cannot lie.
    let scores: Map<String, Value> = arms
        .iter()
        .filter_map(|(id, key, _)| scores_by_id.get(id.to_string()).map(|s| (key.clone(), s.clone())))
        .collect();
    DecideResponse {
        decision_id: id,
        slot: slot.to_string(),
        variant,
        payload,
        explanation: json!({
            "policy": bandit::POLICY,
            "policy_version": version,
            "features": features,
            "scores": scores,
            // A string: a u64 does not survive a JavaScript number.
            "seed": seed.to_string(),
            "in_holdout": false,
        }),
    }
}
