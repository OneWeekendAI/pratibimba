//! `POST /edge/decisions`: decisions the edge Worker made from its snapshot (edge-cache.md, EC2),
//! sent to Core after the response so the decision record lands where every other one does.
//!
//! The Worker presents `PRB_EDGE_TOKEN` as a bearer token; with no token configured the route
//! answers 404, as if it did not exist. Each decision is checked before it is written, and one
//! that fails a check is refused (`rejected`, with the reason) rather than stored wrong:
//!
//! - **The subject.** The Worker sends the client's raw id beside each decision's stored
//!   subject. Core works the stored form out itself (the raw id on an identified request, else
//!   the pseudonym under the salt of the decision's own UTC day) and refuses a mismatch, so a
//!   PoP that straddled midnight cannot file a decision under yesterday's pseudonym.
//! - **The replay.** The model version the decision names must be in `model_versions`, and
//!   replaying the policy there with the stored seed, pool and features must give the scores
//!   the Worker sent (within 1e-9: JavaScript's `Math.exp` and Rust's may differ in the last
//!   bit). Core stores ITS replayed scores, so the decision page's replay check reproduces them
//!   exactly. The pick must follow from them: the holdout hash, the experiment hash, cold
//!   start's pool, or the policy's argmax.
//! - **The erase tombstone**, under the subject's shared erase lock, as `/decide` does: an
//!   erased subject's decision is dropped (`gone`), never written. The Worker then refuses that
//!   subject at its PoP and sends it to Core.
//! - **Idempotent** on the decision id: a retried send is `duplicate`, not a second row.

use super::{ApiError, AppState, valid_subject};
use crate::bandit::LinTs;
use crate::{experiments, hash, privacy};
use axum::{Json, extract::State, http::HeaderMap};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

/// How far a decision's clock may sit from Core's on `/edge/decisions`: older is refused (the
/// queue's `/e/batch` takes older ones, see `ebatch::MAX_DECISION_AGE_SECS`), and the future allows for clock skew between the PoP and the database.
pub const MAX_AGE_SECS: i64 = 600;
pub const MAX_SKEW_SECS: i64 = 60;
/// Most decisions in one send (a batch decide makes at most `decide::MAX_BATCH`).
pub const MAX_DECISIONS: usize = 20;
/// The replay tolerance on each score.
pub const SCORE_TOLERANCE: f64 = 1e-9;
const POLICIES: [&str; 4] = [crate::bandit::POLICY, crate::stated::POLICY, crate::stated::TIME_POLICY, crate::features::POLICY];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ingest {
    pub site: String,
    /// The client's own id, as the request to the Worker carried it.
    pub subject: String,
    /// The request's `ctx.consent`.
    #[serde(default)]
    pub consent: Option<bool>,
    /// The snapshot version the Worker decided from.
    pub snapshot_version: i64,
    pub decisions: Vec<EdgeDecision>,
}

#[derive(Debug, Deserialize)]
pub struct EdgeDecision {
    pub id: Uuid,
    pub slot: String,
    /// The subject as stored (the raw id, or the day's pseudonym).
    pub subject: String,
    pub variant_id: i64,
    pub pool: Vec<i64>,
    pub features: Value,
    /// Variant id -> score.
    pub scores: Value,
    pub policy: String,
    pub policy_version: i64,
    /// A u64, as a string.
    pub seed: String,
    pub in_holdout: bool,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub audiences: Value,
    #[serde(default)]
    pub cold_start: Option<Value>,
    #[serde(default)]
    pub experiment_id: Option<i64>,
    pub allocation: String,
    /// Anything else the Worker kept on its record (e.g. `created_at_ms`) is ignored.
    #[serde(flatten)]
    pub _rest: serde_json::Map<String, Value>,
}

pub async fn handler(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(body): Json<Ingest>) -> Result<Json<Value>, ApiError> {
    if !state.edge.has_token() {
        return Err(ApiError::NotFound("edge ingest is off (no PRB_EDGE_TOKEN)".into()));
    }
    let presented = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
    if !state.edge.token_matches(presented) {
        return Err(ApiError::Unauthorized);
    }
    if !valid_subject(&body.subject) {
        return Err(ApiError::BadRequest("subject must be 1-256 printable ASCII characters".into()));
    }
    if body.decisions.is_empty() || body.decisions.len() > MAX_DECISIONS {
        return Err(ApiError::BadRequest(format!("send 1-{MAX_DECISIONS} decisions")));
    }
    let Some(outcomes) = ingest(&state, &body, MAX_AGE_SECS).await? else {
        return Err(ApiError::NotFound(format!("unknown site {:?}", body.site)));
    };
    let results: Vec<Value> = outcomes
        .into_iter()
        .map(|(id, o)| match o {
            Outcome::Rejected(reason) => json!({ "id": id, "status": "rejected", "reason": reason }),
            Outcome::Stored => json!({ "id": id, "status": "stored" }),
            Outcome::Duplicate => json!({ "id": id, "status": "duplicate" }),
            Outcome::Gone => json!({ "id": id, "status": "gone" }),
        })
        .collect();
    Ok(Json(json!({ "results": results })))
}

/// What became of one edge decision.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Stored,
    Duplicate,
    /// The subject was erased: dropped, never written.
    Gone,
    /// A check failed; the reason.
    Rejected(String),
}

/// Check and store each decision of `body` (see the module doc); `None` for an unknown site.
/// `max_age` bounds how old a decision may be: `/edge/decisions` takes [`MAX_AGE_SECS`], the
/// queue's `/e/batch` more, since a retried or replayed message is older and still the record of
/// what a visitor saw. The caller checks the subject and the count.
pub async fn ingest(state: &AppState, body: &Ingest, max_age: i64) -> Result<Option<Vec<(Uuid, Outcome)>>, ApiError> {
    let site: Option<(String, Option<Vec<String>>, f32, DateTime<Utc>)> =
        sqlx::query_as("SELECT privacy_mode, fields_allowed, holdout_fraction, now() FROM sites WHERE id = $1 AND sim_now IS NULL")
            .bind(&body.site)
            .fetch_optional(&state.pool)
            .await?;
    let Some((mode, fields, holdout, now)) = site else {
        return Ok(None);
    };
    let policy = privacy::Policy { mode, fields, at: now };
    let identified = policy.identified(body.consent);
    let mut results = Vec::with_capacity(body.decisions.len());
    for d in &body.decisions {
        let outcome = match check(state, body, d, identified, holdout, now, max_age).await? {
            Err(reason) => {
                tracing::warn!(site = body.site, decision = %d.id, reason, "edge decision refused");
                Outcome::Rejected(reason)
            }
            Ok(scores) => match store(state, body, d, &scores).await? {
                Stored::Written => Outcome::Stored,
                Stored::Duplicate => Outcome::Duplicate,
                Stored::Gone => Outcome::Gone,
            },
        };
        results.push((d.id, outcome));
    }
    Ok(Some(results))
}

/// Every check in the module doc. `Ok(Ok(scores))` is Core's replayed scores to store.
#[allow(clippy::too_many_arguments)]
async fn check(state: &AppState, body: &Ingest, d: &EdgeDecision, identified: bool, holdout: f32, now: DateTime<Utc>, max_age: i64) -> Result<Result<Value, String>, ApiError> {
    let age = (now - d.created_at).num_seconds();
    if age > max_age || -age > MAX_SKEW_SECS {
        return Ok(Err(format!("created_at is {age} s from Core's clock")));
    }
    // The subject Core would have stored for this request on the decision's day.
    let Some(expected) = stored_subject(&state.pool, &body.site, &body.subject, identified, d.created_at).await? else {
        return Ok(Err("no cookieless salt for the decision's day".into()));
    };
    if d.subject != expected {
        return Ok(Err("subject is not what Core stores for this visitor today".into()));
    }
    if !POLICIES.contains(&d.policy.as_str()) || !["bandit", "experiment"].contains(&d.allocation.as_str()) {
        return Ok(Err(format!("policy {:?} / allocation {:?}", d.policy, d.allocation)));
    }
    let Ok(seed) = d.seed.parse::<u64>() else {
        return Ok(Err("seed is not a u64".into()));
    };
    let slot: Option<i64> = sqlx::query_scalar("SELECT id FROM slots WHERE site_id = $1 AND key = $2").bind(&body.site).bind(&d.slot).fetch_optional(&state.pool).await?;
    let Some(slot_id) = slot else {
        return Ok(Err(format!("unknown slot {:?}", d.slot)));
    };
    let variants: Vec<(i64, bool)> = sqlx::query_as("SELECT id, is_baseline FROM variants WHERE slot_id = $1").bind(slot_id).fetch_all(&state.pool).await?;
    let is_variant = |id: i64| variants.iter().any(|v| v.0 == id);
    if d.pool.is_empty() || !d.pool.iter().all(|id| is_variant(*id)) || !is_variant(d.variant_id) {
        return Ok(Err("variant or pool is not this slot's".into()));
    }
    let state_json: Option<Value> = sqlx::query_scalar("SELECT state FROM model_versions WHERE slot_id = $1 AND version = $2")
        .bind(slot_id)
        .bind(d.policy_version)
        .fetch_optional(&state.pool)
        .await?;
    let Some(model) = state_json.and_then(|s| serde_json::from_value::<LinTs>(s).ok()) else {
        return Ok(Err(format!("model version {} is not in model_versions", d.policy_version)));
    };
    let Some(x) = super::decide::from_named(&d.features) else {
        return Ok(Err("features lack the base context".into()));
    };
    let (pick, replayed) = crate::explain::replay(&model, &d.policy, &d.features, &x, &d.pool, seed);
    let sent = d.scores.as_object().cloned().unwrap_or_default();
    let close = replayed.len() == sent.len()
        && replayed.iter().all(|(id, s)| {
            sent.get(&id.to_string()).and_then(Value::as_f64).is_some_and(|e| (e - s).abs() <= SCORE_TOLERANCE * s.abs().max(1.0))
        });
    if !close {
        return Ok(Err("replaying the policy does not give the scores sent".into()));
    }
    // The pick must follow from the allocation the decision names.
    let baseline = variants.iter().find(|v| v.1).map(|v| v.0);
    let held = baseline.is_some() && hash::in_holdout(&body.site, &d.subject, f64::from(holdout));
    if held != d.in_holdout {
        return Ok(Err("in_holdout does not match the holdout hash".into()));
    }
    let follows = if held {
        Some(d.variant_id) == baseline && d.allocation == "bandit"
    } else if d.allocation == "experiment" {
        match d.experiment_id {
            None => false,
            Some(e) => {
                let arms: Option<Value> = sqlx::query_scalar("SELECT arms FROM experiments WHERE id = $1 AND slot_id = $2").bind(e).bind(slot_id).fetch_optional(&state.pool).await?;
                let open: Vec<(i64, f64)> = arms.map(|a| experiments::parse_arms(&a)).unwrap_or_default().into_iter().filter(|a| d.pool.contains(&a.0)).collect();
                !open.is_empty() && experiments::assign(e, &d.subject, &open) == d.variant_id
            }
        }
    } else if d.cold_start.as_ref().is_some_and(|c| !c.is_null()) {
        d.pool.contains(&d.variant_id)
    } else {
        pick == d.variant_id
    };
    if !follows {
        return Ok(Err(format!("variant {} does not follow from allocation {:?}", d.variant_id, d.allocation)));
    }
    Ok(Ok(Value::Object(replayed.iter().map(|(id, s)| (id.to_string(), json!(s))).collect())))
}

/// The subject Core stores for raw id `raw` on a decision made `at`: the raw id when the request
/// is identified, else the pseudonym under the salt of `at`'s UTC day (not today's: a decision
/// made before midnight and sent after it keeps its own day). `None` when that day has no salt.
pub async fn stored_subject(pool: &sqlx::PgPool, site: &str, raw: &str, identified: bool, at: DateTime<Utc>) -> Result<Option<String>, ApiError> {
    if identified {
        return Ok(Some(raw.to_string()));
    }
    let salt: Option<String> = sqlx::query_scalar("SELECT salt FROM privacy_salts WHERE site_id = $1 AND day = $2")
        .bind(site)
        .bind(privacy::day_of(at))
        .fetch_optional(pool)
        .await?;
    Ok(salt.map(|s| privacy::pseudonym(&s, site, raw)))
}

enum Stored {
    Written,
    Duplicate,
    Gone,
}

/// The decision and its webhook rows, under the subject's shared erase lock with the tombstone
/// checked after the lock is held, exactly as `/decide` writes one.
async fn store(state: &AppState, body: &Ingest, d: &EdgeDecision, scores: &Value) -> Result<Stored, ApiError> {
    let site = &body.site;
    let key: Option<String> = sqlx::query_scalar("SELECT key FROM variants WHERE id = $1").bind(d.variant_id).fetch_optional(&state.pool).await?;
    let (hook_key, hook_body) = crate::webhooks::envelope(
        site,
        "decision",
        json!({ "decision_id": d.id, "slot": d.slot, "variant": key, "subject": d.subject, "in_holdout": d.in_holdout,
                "policy_version": d.policy_version }),
        d.created_at,
    );
    let audiences = if d.audiences.is_null() { json!([]) } else { d.audiences.clone() };
    let cold = d.cold_start.clone().filter(|c| !c.is_null());
    let mut conn = state.pool.acquire().await?;
    let mut tx = sqlx::Connection::begin_with(&mut *conn, format!("BEGIN; SELECT pg_advisory_xact_lock_shared({})", privacy::lock_key(site, &body.subject))).await?;
    let (erased, inserted): (bool, bool) = sqlx::query_as(
        "WITH gone AS (SELECT EXISTS (SELECT 1 FROM erasures WHERE site_id = $2 AND subject_hash = $20) AS g),
              ins AS (
             INSERT INTO decisions (id, site_id, slot_id, variant_id, subject, pool, features, scores, policy, policy_version, seed, in_holdout, created_at,
                                    audiences, cold_start, experiment_id, allocation, source, edge_snapshot_version)
             SELECT $1, $2, s.id, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $16, $17, $18, $19, 'edge', $21
               FROM gone, slots s WHERE NOT gone.g AND s.site_id = $2 AND s.key = $3
             ON CONFLICT (id) DO NOTHING RETURNING 1),
              hooks AS (
             INSERT INTO webhook_deliveries (subscription_id, event, idempotency_key, payload)
             SELECT id, 'decision', $14, $15 FROM webhook_subscriptions WHERE site_id = $2 AND 'decision' = ANY(events) AND EXISTS (SELECT 1 FROM ins))
         SELECT g, EXISTS (SELECT 1 FROM ins) FROM gone",
    )
    .bind(d.id)
    .bind(site)
    .bind(&d.slot)
    .bind(d.variant_id)
    .bind(&d.subject)
    .bind(&d.pool)
    .bind(&d.features)
    .bind(scores)
    .bind(&d.policy)
    .bind(d.policy_version)
    .bind(d.seed.parse::<u64>().unwrap_or_default() as i64)
    .bind(d.in_holdout)
    .bind(d.created_at)
    .bind(hook_key)
    .bind(hook_body)
    .bind(&audiences)
    .bind(&cold)
    .bind(d.experiment_id)
    .bind(&d.allocation)
    .bind(privacy::subject_hash(site, &body.subject))
    .bind(body.snapshot_version)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(if erased {
        Stored::Gone
    } else if inserted {
        Stored::Written
    } else {
        Stored::Duplicate
    })
}
