//! `POST /decide` (one slot) and `POST /decide/batch` (several slots, one request).
//!
//! The context is a closed set of named booleans. `deny_unknown_fields` is the feature
//! allowlist: a `gender`, `age` or any other undeclared name is refused at the boundary rather
//! than silently ignored (backend.md, "No demographic inference").
//!
//! Phase 1 adds two guardrails on the arm pool:
//! - **Holdout.** A fixed fraction of visitors per site (default 10%) always get the baseline.
//!   Their outcomes are recorded and never learned from; the gap between them and everyone
//!   else is the lift. The policy still scores the pool with the decision's seed, so the
//!   counterfactual can say what the policy would have shown.
//! - **Planned traffic share.** A variant approved with share `s` is eligible only for the
//!   visitors whose bucket falls under `s`.

use super::{ApiError, AppState, valid_subject};
use crate::bandit::{self, LinTs, Vector};
use crate::hash;
use axum::{Json, extract::State};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::sync::Arc;
use uuid::Uuid;

pub const MAX_BATCH: usize = 20;

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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchRequest {
    pub site: String,
    pub subject: String,
    pub slots: Vec<String>,
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

/// `{decisions: [DecideResponse | {slot, error}]}` in request order. One bad slot does not
/// fail the others: a page with a stale slot name must still render the rest.
pub async fn batch_handler(State(state): State<Arc<AppState>>, Json(req): Json<BatchRequest>) -> Result<Json<Value>, ApiError> {
    if req.slots.is_empty() || req.slots.len() > MAX_BATCH {
        return Err(ApiError::BadRequest(format!("send 1-{MAX_BATCH} slots per batch")));
    }
    if !valid_subject(&req.subject) {
        return Err(ApiError::BadRequest("subject must be 1-256 printable ASCII characters".into()));
    }
    let mut out = Vec::with_capacity(req.slots.len());
    for slot in &req.slots {
        let one = DecideRequest { site: req.site.clone(), slot: slot.clone(), subject: req.subject.clone(), ctx: req.ctx.clone() };
        out.push(match decide(&state, &one).await {
            Ok(d) => serde_json::to_value(d).map_err(anyhow::Error::from)?,
            Err(ApiError::NotFound(m) | ApiError::Conflict(m) | ApiError::BadRequest(m)) => json!({ "slot": slot, "error": m }),
            Err(e) => return Err(e),
        });
    }
    Ok(Json(json!({ "decisions": out })))
}

/// One approved arm: `(id, key, payload, is_baseline, traffic_share)`.
type ArmRow = (i64, String, Option<String>, bool, Option<f32>);

pub async fn decide(state: &AppState, req: &DecideRequest) -> Result<DecideResponse, ApiError> {
    if !valid_subject(&req.subject) {
        return Err(ApiError::BadRequest("subject must be 1-256 printable ASCII characters".into()));
    }
    let pool = &state.pool;
    // `at` is the site's clock: the database's now, or a sandbox's simulated time.
    let slot: Option<(i64, i32, f32, bool, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT s.id, s.decision_ttl_secs, t.holdout_fraction, t.paused_at IS NOT NULL, COALESCE(t.sim_now, now())
           FROM slots s JOIN sites t ON t.id = s.site_id
          WHERE s.site_id = $1 AND s.key = $2",
    )
    .bind(&req.site)
    .bind(&req.slot)
    .fetch_optional(pool)
    .await?;
    let Some((slot_id, ttl, holdout_fraction, paused, at)) = slot else {
        return Err(ApiError::NotFound(format!("unknown slot {:?} on site {:?}", req.slot, req.site)));
    };
    let mut arms: Vec<ArmRow> = sqlx::query_as(
        "SELECT id, key, payload, is_baseline, traffic_share FROM variants WHERE slot_id = $1 AND status = 'approved' ORDER BY id",
    )
    .bind(slot_id)
    .fetch_all(pool)
    .await?;
    // The kill switch: a paused site serves only the page's own content.
    if paused {
        arms.retain(|a| a.3);
        if arms.is_empty() {
            return Err(ApiError::Conflict(format!("site {:?} is paused", req.site)));
        }
    }
    if arms.is_empty() {
        return Err(ApiError::Conflict(format!("slot {:?} has no approved variants", req.slot)));
    }
    let key_of = |id: i64| arms.iter().find(|a| a.0 == id).map(|a| (a.1.clone(), a.2.clone()));

    // Sticky: a reload inside the TTL gets the same decision, so the hero does not reroll.
    let recent: Option<(Uuid, i64, Value, Value, i64, i64, bool)> = sqlx::query_as(
        "SELECT id, variant_id, features, scores, seed, policy_version, in_holdout FROM decisions
          WHERE slot_id = $1 AND subject = $2 AND created_at > $4 - make_interval(secs => $3) AND created_at <= $4
          ORDER BY created_at DESC LIMIT 1",
    )
    .bind(slot_id)
    .bind(&req.subject)
    .bind(ttl as f64)
    .bind(at)
    .fetch_optional(pool)
    .await?;
    if let Some((id, variant_id, features, scores, seed, version, held)) = recent {
        if let Some((key, payload)) = key_of(variant_id) {
            return Ok(response(id, &req.slot, key, payload, version, features, &scores, seed as u64, held, &arms));
        }
    }

    // The pool: every approved arm this visitor is eligible for.
    let eligible: Vec<i64> = arms
        .iter()
        .filter(|(id, _, _, _, share)| match share {
            Some(s) => hash::within_share(*id, &req.subject, f64::from(*s)),
            None => true,
        })
        .map(|a| a.0)
        .collect();
    let baseline = arms.iter().find(|a| a.3).map(|a| a.0);
    let in_holdout = baseline.is_some() && hash::in_holdout(&req.site, &req.subject, f64::from(holdout_fraction));

    let (version, state_json): (i64, Value) =
        sqlx::query_as("SELECT version, state FROM models WHERE slot_id = $1").bind(slot_id).fetch_one(pool).await?;
    let model: LinTs = serde_json::from_value(state_json).map_err(anyhow::Error::from)?;
    let x = req.ctx.vector();
    let seed = state.seeds.lock().expect("seed rng poisoned").next_u64();
    let ids = if eligible.is_empty() { arms.iter().map(|a| a.0).collect() } else { eligible };
    let (picked, scores) = model.choose(&ids, &x, seed);
    let chosen = match (in_holdout, baseline) {
        (true, Some(b)) => b,
        _ => picked,
    };
    let scores_json = Value::Object(scores.iter().map(|(id, s)| (id.to_string(), json!(s))).collect());
    let features = named(&x);
    let id = Uuid::new_v4();
    let (key, payload) = key_of(chosen).expect("chosen arm is in the pool");
    // One statement writes the decision and its webhook outbox rows, so both land or neither.
    let (hook_key, hook_body) = crate::webhooks::envelope(
        &req.site,
        "decision",
        json!({ "decision_id": id, "slot": req.slot, "variant": key, "subject": req.subject, "in_holdout": in_holdout,
                "policy_version": version }),
        at,
    );
    sqlx::query(
        "WITH ins AS (
             INSERT INTO decisions (id, site_id, slot_id, variant_id, subject, pool, features, scores, policy, policy_version, seed, in_holdout, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) RETURNING 1)
         INSERT INTO webhook_deliveries (subscription_id, event, idempotency_key, payload)
         SELECT id, 'decision', $14, $15 FROM webhook_subscriptions WHERE site_id = $2 AND 'decision' = ANY(events) AND EXISTS (SELECT 1 FROM ins)",
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
    .bind(in_holdout)
    .bind(at)
    .bind(hook_key)
    .bind(hook_body)
    .execute(pool)
    .await?;
    Ok(response(id, &req.slot, key, payload, version, features, &scores_json, seed, in_holdout, &arms))
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
    in_holdout: bool,
    arms: &[ArmRow],
) -> DecideResponse {
    // Explanations name variants by key; storage keys them by id so a rename cannot lie.
    let scores: Map<String, Value> = arms
        .iter()
        .filter_map(|(id, key, ..)| scores_by_id.get(id.to_string()).map(|s| (key.clone(), s.clone())))
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
            "in_holdout": in_holdout,
        }),
    }
}
