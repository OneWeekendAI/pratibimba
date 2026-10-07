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
//!
//! **Cold start** (backend.md's open question on a brand-new slot, shipped as the default).
//! Thompson sampling with a weak prior starves an arm that loses its first few draws. Until a
//! slot has had `cold_start_exposures` exposures (default 100, holdout excluded, 0 = off), the
//! visitor gets the eligible arm served least so far in cold start, ties broken by the seed;
//! then LinTS decides. The policy still scores the pool, so the scores, the replay and the
//! counterfactual stay LinTS's, and the posterior learns from every one of those outcomes.
//! The explanation's `cold_start` is `{exposures, until}` when cold start chose, else null.

use super::{ApiError, AppState, valid_subject};
use crate::bandit::{self, LinTs, Vector};
use crate::{hash, privacy, stated};
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
    /// Picker option keys the client holds for this visitor (see [`crate::stated`]). Each must
    /// be one of the site's options; anything else is refused.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stated: Vec<String>,
    /// The visitor's UTC offset in minutes east (`-new Date().getTimezoneOffset()`). With it,
    /// the decision carries the local-time features `sin_hour`, `cos_hour` and
    /// `is_night_local` (see [`crate::insights::localtime`]); without it, it carries none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz_offset: Option<i16>,
    /// The visitor's IANA time zone (`Asia/Kolkata`), as a server-side SDK reads it from the
    /// edge's geolocation or the browser's cookie. Resolved at the decision's clock, so DST
    /// holds; preferred over `tz_offset` when both are sent. An unknown name is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    /// Whether the visitor consented to being remembered. Read only by a `consent_aware` site
    /// (see [`crate::privacy`]); it is not a feature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent: Option<bool>,
    /// The site's declared features (see [`crate::features`]): name -> on. An undeclared name is
    /// refused; a PII one is dropped unless the request is identified.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub features: std::collections::BTreeMap<String, bool>,
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
            // An erased subject is refused as a whole, not slot by slot.
            Err(e) => return Err(e),
        });
    }
    Ok(Json(json!({ "decisions": out })))
}

/// One approved arm: `(id, key, payload, is_baseline, traffic_share)`.
type ArmRow = (i64, String, Option<String>, bool, Option<f32>);
/// A recent decision: `(id, variant_id, features, scores, seed, policy_version, in_holdout, policy, audiences, cold_start)`.
type StickyRow = (Uuid, i64, Value, Value, i64, i64, bool, String, Option<Value>, Option<Value>);

pub async fn decide(state: &AppState, req: &DecideRequest) -> Result<DecideResponse, ApiError> {
    if !valid_subject(&req.subject) {
        return Err(ApiError::BadRequest("subject must be 1-256 printable ASCII characters".into()));
    }
    if req.ctx.tz_offset.is_some_and(|m| !crate::insights::localtime::valid_offset(m)) {
        return Err(ApiError::BadRequest("ctx.tz_offset is minutes east of UTC, -720 to 840".into()));
    }
    let zone = match req.ctx.tz.as_deref() {
        None => None,
        Some(name) => Some(
            crate::insights::localtime::parse_zone(name)
                .ok_or_else(|| ApiError::BadRequest(format!("ctx.tz {name:?} is not an IANA time zone")))?,
        ),
    };
    let pool = &state.pool;
    // Privacy first: from here on the subject is the one stored (a pseudonym unless the request
    // is identified) and the context holds only the fields this site collects.
    let Some(policy) = privacy::policy(pool, &req.site).await? else {
        return Err(ApiError::NotFound(format!("unknown slot {:?} on site {:?}", req.slot, req.site)));
    };
    let identified = policy.identified(req.ctx.consent);
    let raw = req.subject.as_str();
    let req = &DecideRequest {
        site: req.site.clone(),
        slot: req.slot.clone(),
        subject: privacy::stored_subject(pool, &policy, &req.site, raw, identified).await?,
        ctx: policy.ctx(&req.ctx, identified),
    };
    // A named zone is the visitor's clock too: collected only with `tz_offset`.
    let zone = zone.filter(|_| req.ctx.tz.is_some());
    // `at` is the site's clock: the database's now, or a sandbox's simulated time.
    let slot: Option<(i64, i32, f32, bool, chrono::DateTime<chrono::Utc>, i16, i16, i32)> = sqlx::query_as(
        "SELECT s.id, s.decision_ttl_secs, t.holdout_fraction, t.paused_at IS NOT NULL, COALESCE(t.sim_now, now()),
                t.night_start_hour, t.night_end_hour, s.cold_start_exposures
           FROM slots s JOIN sites t ON t.id = s.site_id
          WHERE s.site_id = $1 AND s.key = $2",
    )
    .bind(&req.site)
    .bind(&req.slot)
    .fetch_optional(pool)
    .await?;
    let Some((slot_id, ttl, holdout_fraction, paused, at, night_start, night_end, cold_until)) = slot else {
        return Err(ApiError::NotFound(format!("unknown slot {:?} on site {:?}", req.slot, req.site)));
    };
    // The visitor's local clock: a named zone (resolved at the site's clock) wins over an offset.
    let ctx = Ctx {
        tz_offset: zone.map(|z| crate::insights::localtime::zone_offset(z, at)).or(req.ctx.tz_offset),
        ..req.ctx.clone()
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
    // Promoted audiences gate their variants: a visitor outside the audience never gets them.
    let night = crate::insights::localtime::NightWindow { start: night_start as u8, end: night_end as u8 };
    let audiences = gate_by_audience(pool, slot_id, &ctx, at, night, &mut arms).await?;
    if arms.is_empty() {
        return Err(ApiError::Conflict(format!("no variant of slot {:?} is open to this visitor's audiences", req.slot)));
    }
    // The `stated` feature group: present on every decision of a site that has a picker.
    let site_keys = stated::site_keys(pool, &req.site).await?;
    let said = stated::resolve(pool, &req.site, &req.subject, &site_keys, &ctx.stated).await?;
    let said_now = stated::from_features(&stated::with_stated(json!({}), &site_keys, &said));
    let declared = crate::features::values(&crate::features::declared(pool, &req.site).await?, &ctx.features, identified)?;
    let key_of = |id: i64| arms.iter().find(|a| a.0 == id).map(|a| (a.1.clone(), a.2.clone()));

    // Sticky: a reload inside the TTL gets the same decision, so the hero does not reroll.
    let recent: Option<StickyRow> = sqlx::query_as(
        "SELECT id, variant_id, features, scores, seed, policy_version, in_holdout, policy, audiences, cold_start FROM decisions
          WHERE slot_id = $1 AND subject = $2 AND created_at > $4 - make_interval(secs => $3) AND created_at <= $4
            AND NOT (source = 'client' AND COALESCE(divergence, 0) > $5)
          ORDER BY created_at DESC LIMIT 1",
    )
    .bind(slot_id)
    .bind(&req.subject)
    .bind(ttl as f64)
    .bind(at)
    .bind(crate::adapt::DIVERGENCE)
    .fetch_optional(pool)
    .await?;
    if let Some((id, variant_id, features, scores, seed, version, held, policy, auds, cold)) = recent {
        // A visitor who has stated something new since gets a fresh decision.
        let same_statement = stated::from_features(&features) == said_now;
        if let (true, Some((key, payload))) = (same_statement, key_of(variant_id)) {
            let auds = auds.unwrap_or_else(|| json!([]));
            return Ok(response(id, &req.slot, key, payload, &policy, version, features, &scores, seed as u64, held, &arms, auds, cold));
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
    let x = ctx.vector();
    let seed = state.seeds.lock().expect("seed rng poisoned").next_u64();
    let ids = if eligible.is_empty() { arms.iter().map(|a| a.0).collect() } else { eligible };
    // The extension: the site's stated tastes, plus the local clock when the visitor reported it.
    let mut ext_keys = site_keys.clone();
    let mut ext = said.clone();
    ext_keys.extend(declared.keys().cloned());
    ext.extend(declared.clone());
    if let Some(off) = ctx.tz_offset {
        let local = crate::insights::localtime::LocalTime::at(at, Some(off), night);
        ext_keys.extend(stated::time_keys());
        ext.extend(stated::time_values(&local));
    }
    let (picked, scores, policy, features) = if ext_keys.is_empty() {
        let (p, s) = model.choose(&ids, &x, seed);
        (p, s, bandit::POLICY, named(&x))
    } else {
        let (p, s) = stated::choose(&model, &ext_keys, &ids, &x, &ext, seed);
        let policy = if !site_keys.is_empty() {
            stated::POLICY
        } else if !declared.is_empty() {
            crate::features::POLICY
        } else {
            stated::TIME_POLICY
        };
        (p, s, policy, stated::with_stated(named(&x), &ext_keys, &ext))
    };
    let held = in_holdout && baseline.is_some();
    let cold = if held { None } else { cold_start(pool, slot_id, cold_until, &ids, seed).await? };
    let chosen = match (held, baseline, &cold) {
        (true, Some(b), _) => b,
        (_, _, Some((arm, _))) => *arm,
        _ => picked,
    };
    let cold = cold.map(|(_, c)| c);
    let scores_json = Value::Object(scores.iter().map(|(id, s)| (id.to_string(), json!(s))).collect());
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
    // The tombstone check and the insert share one transaction under the subject's lock.
    let mut tx = pool.begin().await?;
    privacy::admit(&mut tx, &req.site, raw).await?;
    sqlx::query(
        "WITH ins AS (
             INSERT INTO decisions (id, site_id, slot_id, variant_id, subject, pool, features, scores, policy, policy_version, seed, in_holdout, created_at, audiences, cold_start)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $16, $17) RETURNING 1)
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
    .bind(policy)
    .bind(version)
    .bind(seed as i64)
    .bind(in_holdout)
    .bind(at)
    .bind(hook_key)
    .bind(hook_body)
    .bind(&audiences)
    .bind(&cold)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(response(id, &req.slot, key, payload, policy, version, features, &scores_json, seed, in_holdout, &arms, audiences, cold))
}

/// Cold start's pick, `(arm, {exposures, until})`, or `None` once the slot has had `until`
/// learned-from exposures (or `until` is 0). The pick is the arm of `ids` that cold start has
/// served least, ties broken by the decision's seed, so the arms take equal turns.
async fn cold_start(pool: &sqlx::PgPool, slot_id: i64, until: i32, ids: &[i64], seed: u64) -> Result<Option<(i64, Value)>, ApiError> {
    if until <= 0 {
        return Ok(None);
    }
    let exposures: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT 1 FROM decisions WHERE slot_id = $1 AND exposed_at IS NOT NULL AND NOT in_holdout LIMIT $2) e",
    )
    .bind(slot_id)
    .bind(i64::from(until))
    .fetch_one(pool)
    .await?;
    if exposures >= i64::from(until) {
        return Ok(None);
    }
    let served: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT variant_id, count(*) FROM decisions WHERE slot_id = $1 AND cold_start IS NOT NULL AND variant_id = ANY($2) GROUP BY variant_id",
    )
    .bind(slot_id)
    .bind(ids)
    .fetch_all(pool)
    .await?;
    let turns = |id: i64| served.iter().find(|s| s.0 == id).map_or(0, |s| s.1);
    let fewest = ids.iter().map(|id| turns(*id)).min().unwrap_or(0);
    let due: Vec<i64> = ids.iter().copied().filter(|id| turns(*id) == fewest).collect();
    let arm = due[(seed % due.len() as u64) as usize];
    Ok(Some((arm, json!({ "exposures": exposures, "until": until }))))
}

#[allow(clippy::too_many_arguments)]
fn response(
    id: Uuid,
    slot: &str,
    variant: String,
    payload: Option<String>,
    policy: &str,
    version: i64,
    features: Value,
    scores_by_id: &Value,
    seed: u64,
    in_holdout: bool,
    arms: &[ArmRow],
    audiences: Value,
    cold_start: Option<Value>,
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
            "policy": policy,
            "policy_version": version,
            "features": features,
            "scores": scores,
            // A string: a u64 does not survive a JavaScript number.
            "seed": seed.to_string(),
            "in_holdout": in_holdout,
            // Every audience that gates a variant of this slot: whether this visitor matched,
            // and the variants it kept out when they did not.
            "audiences": audiences,
            // `{exposures, until}` when cold start chose this arm (see the module doc).
            "cold_start": cold_start,
        }),
    }
}

/// A variant's gate: `(variant_id, variant key, audience id, name, predicate, status)`.
type GateRow = (i64, String, i64, String, Value, String);

/// Drop from `arms` every variant whose audience this visitor is outside, and say so:
/// `[{id, name, matched, variants}]`, one entry per audience, ascending id. Fails closed: an
/// audience that is no longer promoted, or reads a local clock the visitor did not report,
/// matches nobody. The baseline is never gated (a CHECK holds it), so the pool is never empty.
async fn gate_by_audience(
    pool: &sqlx::PgPool,
    slot_id: i64,
    ctx: &Ctx,
    at: chrono::DateTime<chrono::Utc>,
    night: crate::insights::localtime::NightWindow,
    arms: &mut Vec<ArmRow>,
) -> Result<Value, ApiError> {
    use crate::insights::{localtime::LocalTime, segments};
    let gates: Vec<GateRow> = sqlx::query_as(
        "SELECT v.id, v.key, g.id, g.name, g.predicate, g.status FROM variants v JOIN segments g ON g.id = v.audience_id
          WHERE v.slot_id = $1 AND v.status = 'approved' ORDER BY g.id, v.id",
    )
    .bind(slot_id)
    .fetch_all(pool)
    .await?;
    let visit = segments::Visit {
        ctx: (ctx.returning as u8) * segments::CTX_RETURNING | (ctx.mobile as u8) * segments::CTX_MOBILE | (ctx.from_ad as u8) * segments::CTX_FROM_AD,
        local: LocalTime::at(at, ctx.tz_offset, night),
    };
    let mut out: Vec<Value> = Vec::new();
    let mut shut: Vec<i64> = Vec::new();
    for (vid, vkey, gid, name, predicate, status) in gates {
        if out.last().and_then(|o| o["id"].as_i64()) != Some(gid) {
            let matched = status == "audience"
                && segments::Predicate::parse(&predicate)
                    .is_ok_and(|p| (ctx.tz_offset.is_some() || !p.uses_local_time()) && p.matches(&visit));
            out.push(json!({ "id": gid, "name": name, "matched": matched, "variants": [] }));
        }
        let entry = out.last_mut().expect("pushed above");
        if entry["matched"] == json!(false) {
            shut.push(vid);
            entry["variants"].as_array_mut().expect("an array").push(json!(vkey));
        }
    }
    arms.retain(|a| !shut.contains(&a.0));
    Ok(Value::Array(out))
}
