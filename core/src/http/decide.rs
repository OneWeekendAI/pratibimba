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
use crate::{experiments, hash, privacy, stated};
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
/// A recent decision: `(id, variant_id, features, scores, seed, policy_version, in_holdout, policy, audiences, cold_start,
/// experiment_id, allocation)`.
type StickyRow = (Uuid, i64, Value, Value, i64, i64, bool, String, Option<Value>, Option<Value>, Option<i64>, String);

/// Everything one decide reads, in one statement (one round trip). Binds: `$1` site, `$2` slot,
/// `$3` the subject when the request is identified, `$4` its pseudonym under the salt decide
/// guessed (NULL when it has none), `$5` consent, `$6` [`crate::adapt::DIVERGENCE`].
///
/// The subject-keyed parts (the sticky decision and the latest picks) read the subject the
/// `subject` column names. `salt` and `stale_salts` are what [`privacy::salt`] would find for the
/// site's day, so decide can tell whether its guess was right; when it was not, it runs
/// [`privacy::salt`] exactly as before and reads again with the right subject.
///
/// Nothing here is cached: each read is the database's state at this statement, so an approval,
/// a retirement, a pause, a model update, a new experiment or a privacy-mode change applies to the
/// very next decide, as it did when these were separate statements.
const SNAPSHOT: &str = "
WITH t AS (
    SELECT privacy_mode, fields_allowed, COALESCE(sim_now, now()) AS at, holdout_fraction, paused_at IS NOT NULL AS paused,
           night_start_hour, night_end_hour
      FROM sites WHERE id = $1),
day AS (SELECT (at AT TIME ZONE 'UTC')::date AS d FROM t),
subj AS (
    SELECT CASE WHEN privacy_mode = 'full' OR (privacy_mode = 'consent_aware' AND $5::bool IS TRUE) THEN $3::text ELSE $4::text END AS v
      FROM t),
keys AS (
    SELECT COALESCE(array_agg(k ORDER BY k), '{}') AS k
      FROM (SELECT DISTINCT o->>'key' AS k FROM slots s, jsonb_array_elements(s.picker->'options') o
             WHERE s.site_id = $1 AND s.kind = 'picker') x)
SELECT t.privacy_mode, t.fields_allowed, t.at, t.holdout_fraction, t.paused, t.night_start_hour, t.night_end_hour,
       (SELECT p.salt FROM privacy_salts p, day WHERE p.site_id = $1 AND p.day >= day.d ORDER BY p.day LIMIT 1) AS salt,
       EXISTS (SELECT 1 FROM privacy_salts p, day WHERE p.site_id = $1 AND p.day < day.d) AS stale_salts,
       (SELECT v FROM subj) AS subject,
       s.id AS slot_id, s.cold_start_exposures,
       a.arm_ids, a.arm_keys, a.arm_payloads, a.arm_baselines, a.arm_shares,
       (SELECT jsonb_agg(jsonb_build_array(v.id, v.key, g.id, g.name, g.predicate, g.status) ORDER BY g.id, v.id)
          FROM variants v JOIN segments g ON g.id = v.audience_id
         WHERE v.slot_id = s.id AND v.status = 'approved') AS gates,
       keys.k AS site_keys,
       CASE WHEN cardinality(keys.k) > 0 THEN
           (SELECT jsonb_agg(to_jsonb(p.picks))
              FROM (SELECT DISTINCT ON (slot_id) picks FROM live_events
                     WHERE site_id = $1 AND subject = (SELECT v FROM subj) AND picks IS NOT NULL
                     ORDER BY slot_id, received_at DESC) p)
       END AS picks,
       (SELECT jsonb_object_agg(f.key, f.pii) FROM site_features f WHERE f.site_id = $1) AS declared,
       x.id AS exp_id, x.name AS exp_name, x.arms AS exp_arms,
       r.id AS st_id, r.variant_id AS st_variant, r.features AS st_features, r.scores AS st_scores, r.seed AS st_seed,
       r.policy_version AS st_version, r.in_holdout AS st_held, r.policy AS st_policy, r.audiences AS st_audiences,
       r.cold_start AS st_cold, r.experiment_id AS st_experiment, r.allocation AS st_allocation,
       m.version AS model_version, m.state AS model_state,
       c.exposures AS cold_exposures,
       CASE WHEN c.exposures < s.cold_start_exposures THEN
           (SELECT jsonb_object_agg(variant_id, n)
              FROM (SELECT variant_id, count(*) AS n FROM decisions WHERE slot_id = s.id AND cold_start IS NOT NULL GROUP BY variant_id) q)
       END AS cold_served
  FROM t
  CROSS JOIN keys
  LEFT JOIN slots s ON s.site_id = $1 AND s.key = $2
  LEFT JOIN LATERAL (
      SELECT array_agg(id ORDER BY id) AS arm_ids, array_agg(key ORDER BY id) AS arm_keys, array_agg(payload ORDER BY id) AS arm_payloads,
             array_agg(is_baseline ORDER BY id) AS arm_baselines, array_agg(traffic_share ORDER BY id) AS arm_shares
        FROM variants WHERE slot_id = s.id AND status = 'approved') a ON true
  LEFT JOIN LATERAL (
      SELECT id, name, arms FROM experiments
       WHERE slot_id = s.id AND starts_at <= t.at AND t.at < least(stops_at, coalesce(stopped_at, 'infinity'))
       ORDER BY starts_at LIMIT 1) x ON true
  LEFT JOIN LATERAL (
      SELECT id, variant_id, features, scores, seed, policy_version, in_holdout, policy, audiences, cold_start, experiment_id, allocation
        FROM decisions
       WHERE slot_id = s.id AND subject = (SELECT v FROM subj)
         AND created_at > t.at - make_interval(secs => s.decision_ttl_secs::float8) AND created_at <= t.at
         AND NOT (source = 'client' AND COALESCE(divergence, 0) > $6::float8)
       ORDER BY created_at DESC LIMIT 1) r ON true
  LEFT JOIN models m ON m.slot_id = s.id
  LEFT JOIN LATERAL (
      SELECT CASE WHEN s.cold_start_exposures > 0 THEN
                 (SELECT count(*) FROM (SELECT 1 FROM decisions
                                         WHERE slot_id = s.id AND exposed_at IS NOT NULL AND NOT in_holdout
                                         LIMIT s.cold_start_exposures) e)
             END AS exposures) c ON true";

/// What [`SNAPSHOT`] read.
struct Snapshot {
    policy: privacy::Policy,
    holdout_fraction: f32,
    paused: bool,
    night_start: i16,
    night_end: i16,
    salt: Option<String>,
    stale_salts: bool,
    subject: Option<String>,
    slot: Option<SlotSnapshot>,
}

/// The slot's part of [`Snapshot`]; absent for an unknown slot.
struct SlotSnapshot {
    id: i64,
    cold_until: i32,
    arms: Vec<ArmRow>,
    gates: Vec<GateRow>,
    site_keys: Vec<String>,
    picks: Vec<Vec<String>>,
    declared: std::collections::BTreeMap<String, bool>,
    experiment: Option<experiments::Running>,
    sticky: Option<StickyRow>,
    model: Option<(i64, Value)>,
    /// `(exposures so far, served per arm)` while the slot is in cold start, else `None`.
    cold: Option<(i64, Vec<(i64, i64)>)>,
}

async fn snapshot(conn: &mut sqlx::PgConnection, req: &DecideRequest, identified_subject: &str, pseudonym: Option<&str>) -> Result<Option<Snapshot>, ApiError> {
    use sqlx::Row;
    let Some(row) = sqlx::query(SNAPSHOT)
        .bind(&req.site)
        .bind(&req.slot)
        .bind(identified_subject)
        .bind(pseudonym)
        .bind(req.ctx.consent)
        .bind(crate::adapt::DIVERGENCE)
        .fetch_optional(&mut *conn)
        .await?
    else {
        return Ok(None);
    };
    let bad = |what: &str| ApiError::Internal(anyhow::anyhow!("decide snapshot: malformed {what}"));
    let slot = match row.try_get::<Option<i64>, _>("slot_id")? {
        None => None,
        Some(id) => {
            let ids: Vec<i64> = row.try_get::<Option<Vec<i64>>, _>("arm_ids")?.unwrap_or_default();
            let keys: Vec<String> = row.try_get::<Option<Vec<String>>, _>("arm_keys")?.unwrap_or_default();
            let payloads: Vec<Option<String>> = row.try_get::<Option<Vec<Option<String>>>, _>("arm_payloads")?.unwrap_or_default();
            let baselines: Vec<bool> = row.try_get::<Option<Vec<bool>>, _>("arm_baselines")?.unwrap_or_default();
            let shares: Vec<Option<f32>> = row.try_get::<Option<Vec<Option<f32>>>, _>("arm_shares")?.unwrap_or_default();
            let arms: Vec<ArmRow> = ids
                .into_iter()
                .zip(keys)
                .zip(payloads)
                .zip(baselines)
                .zip(shares)
                .map(|((((id, key), payload), base), share)| (id, key, payload, base, share))
                .collect();
            let gates: Vec<GateRow> = match row.try_get::<Option<Value>, _>("gates")? {
                None => vec![],
                Some(v) => serde_json::from_value(v).map_err(|_| bad("gates"))?,
            };
            let picks: Vec<Vec<String>> = match row.try_get::<Option<Value>, _>("picks")? {
                None => vec![],
                Some(v) => serde_json::from_value(v).map_err(|_| bad("picks"))?,
            };
            let declared = match row.try_get::<Option<Value>, _>("declared")? {
                None => Default::default(),
                Some(v) => serde_json::from_value(v).map_err(|_| bad("declared"))?,
            };
            let experiment = match row.try_get::<Option<i64>, _>("exp_id")? {
                None => None,
                Some(id) => Some(experiments::Running {
                    id,
                    name: row.try_get("exp_name")?,
                    arms: experiments::parse_arms(&row.try_get::<Value, _>("exp_arms")?),
                }),
            };
            let sticky: Option<StickyRow> = match row.try_get::<Option<Uuid>, _>("st_id")? {
                None => None,
                Some(sid) => Some((
                    sid,
                    row.try_get("st_variant")?,
                    row.try_get("st_features")?,
                    row.try_get("st_scores")?,
                    row.try_get("st_seed")?,
                    row.try_get("st_version")?,
                    row.try_get("st_held")?,
                    row.try_get("st_policy")?,
                    row.try_get("st_audiences")?,
                    row.try_get("st_cold")?,
                    row.try_get("st_experiment")?,
                    row.try_get("st_allocation")?,
                )),
            };
            let model = match row.try_get::<Option<i64>, _>("model_version")? {
                None => None,
                Some(v) => Some((v, row.try_get::<Value, _>("model_state")?)),
            };
            let cold = match row.try_get::<Option<i64>, _>("cold_exposures")? {
                None => None,
                Some(n) => {
                    let served: std::collections::BTreeMap<String, i64> = match row.try_get::<Option<Value>, _>("cold_served")? {
                        None => Default::default(),
                        Some(v) => serde_json::from_value(v).map_err(|_| bad("cold_served"))?,
                    };
                    let served = served.into_iter().map(|(k, n)| k.parse::<i64>().map(|k| (k, n)).map_err(|_| bad("cold_served"))).collect::<Result<_, _>>()?;
                    Some((n, served))
                }
            };
            Some(SlotSnapshot {
                id,
                cold_until: row.try_get("cold_start_exposures")?,
                arms,
                gates,
                site_keys: row.try_get("site_keys")?,
                picks,
                declared,
                experiment,
                sticky,
                model,
                cold,
            })
        }
    };
    Ok(Some(Snapshot {
        policy: privacy::Policy { mode: row.try_get("privacy_mode")?, fields: row.try_get("fields_allowed")?, at: row.try_get("at")? },
        holdout_fraction: row.try_get("holdout_fraction")?,
        paused: row.try_get("paused")?,
        night_start: row.try_get("night_start_hour")?,
        night_end: row.try_get("night_end_hour")?,
        salt: row.try_get("salt")?,
        stale_salts: row.try_get("stale_salts")?,
        subject: row.try_get("subject")?,
        slot,
    }))
}

/// What [`decide`] reads for `(site, slot)` that is the same for every visitor, as JSON: the
/// edge snapshot ([`crate::edge`]) is built from this, so it holds exactly what decide reads.
/// It runs [`SNAPSHOT`] itself with an empty subject (no real subject is empty, so the sticky
/// decision and the latest picks come back empty) and drops the clock, the salt and the subject.
/// `None` for an unknown site; the slot part is `null` for an unknown slot.
pub(crate) async fn shared_inputs(conn: &mut sqlx::PgConnection, site: &str, slot: &str) -> Result<Option<(Value, Value)>, ApiError> {
    let req = DecideRequest { site: site.to_string(), slot: slot.to_string(), subject: String::new(), ctx: Ctx::default() };
    let Some(snap) = snapshot(conn, &req, "", None).await? else {
        return Ok(None);
    };
    let settings = json!({
        "privacy_mode": snap.policy.mode,
        "fields_allowed": snap.policy.fields,
        "holdout_fraction": snap.holdout_fraction,
        "paused": snap.paused,
        "night": { "start": snap.night_start, "end": snap.night_end },
    });
    let slot = match snap.slot {
        None => Value::Null,
        Some(s) => json!({
            "id": s.id,
            "cold_start_exposures": s.cold_until,
            "arms": s.arms.iter().map(|(id, key, payload, baseline, share)| json!({
                "id": id, "key": key, "payload": payload, "baseline": baseline, "traffic_share": share,
            })).collect::<Vec<_>>(),
            "gates": s.gates.iter().map(|(vid, vkey, gid, name, predicate, status)| json!({
                "variant_id": vid, "variant": vkey, "audience_id": gid, "audience": name, "predicate": predicate, "status": status,
            })).collect::<Vec<_>>(),
            "picker_keys": s.site_keys,
            "declared_features": s.declared,
            "experiment": s.experiment.map(|e| json!({
                "id": e.id, "name": e.name,
                "arms": e.arms.iter().map(|(v, w)| json!({ "variant_id": v, "weight": w })).collect::<Vec<_>>(),
            })),
            "model": s.model.map(|(version, state)| json!({ "version": version, "state": state })),
            "cold_start": s.cold.map(|(exposures, served)| json!({
                "exposures": exposures,
                "served": served.into_iter().map(|(v, n)| (v.to_string(), json!(n))).collect::<Map<String, Value>>(),
            })),
        }),
    };
    Ok(Some((settings, slot)))
}

/// Decide for one slot.
///
/// Round trips (a warm decide that writes a decision makes four, one that reuses a sticky
/// decision makes one, plus the ping sqlx makes when the connection goes back to the pool,
/// after the response is built): [`SNAPSHOT`] reads everything; then the write is `BEGIN` together with
/// the subject's shared erase lock, one statement that checks the tombstone and inserts the
/// decision and its webhook rows, and `COMMIT`. The tombstone is read in a statement of its own
/// after the lock is held, so its snapshot sees an erase that committed while decide waited.
/// A cookieless site adds the salt statement only when decide's guess of the day's salt was
/// wrong (the first decide of a day per process), plus one more read with the right subject.
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
    // One connection for the whole decide: handing one back costs sqlx a ping, so each extra
    // acquire would be one more round trip.
    let mut conn = state.pool.acquire().await?;
    let raw = req.subject.as_str();
    let guess = state.salts.lock().expect("salt cache poisoned").get(&req.site).cloned();
    let guessed = guess.as_deref().map(|s| privacy::pseudonym(s, &req.site, raw));
    let unknown = || ApiError::NotFound(format!("unknown slot {:?} on site {:?}", req.slot, req.site));
    // Privacy first: from here on the subject is the one stored (a pseudonym unless the request
    // is identified) and the context holds only the fields this site collects.
    let Some(mut snap) = snapshot(&mut conn, req, raw, guessed.as_deref()).await? else {
        return Err(unknown());
    };
    let policy = snap.policy.clone();
    let identified = policy.identified(req.ctx.consent);
    let subject = if identified {
        raw.to_string()
    } else if guess.is_some() && snap.salt == guess && !snap.stale_salts {
        // The guess is the salt `privacy::salt` would return, and it would delete nothing.
        guessed.clone().expect("a guessed salt has a pseudonym")
    } else {
        let s = privacy::salt_on(&mut conn, &req.site, privacy::day_of(policy.at)).await?;
        state.salts.lock().expect("salt cache poisoned").insert(req.site.clone(), s.clone());
        privacy::pseudonym(&s, &req.site, raw)
    };
    if snap.subject.as_deref() != Some(subject.as_str()) {
        snap = snapshot(&mut conn, req, &subject, Some(&subject)).await?.ok_or_else(unknown)?;
    }
    let req = &DecideRequest { site: req.site.clone(), slot: req.slot.clone(), subject, ctx: policy.ctx(&req.ctx, identified) };
    // A named zone is the visitor's clock too: collected only with `tz_offset`.
    let zone = zone.filter(|_| req.ctx.tz.is_some());
    // `at` is the site's clock: the database's now, or a sandbox's simulated time.
    let at = snap.policy.at;
    let (holdout_fraction, paused, night_start, night_end) = (snap.holdout_fraction, snap.paused, snap.night_start, snap.night_end);
    let Some(slot) = snap.slot else {
        return Err(unknown());
    };
    let (slot_id, cold_until) = (slot.id, slot.cold_until);
    // The visitor's local clock: a named zone (resolved at the site's clock) wins over an offset.
    let ctx = Ctx {
        tz_offset: zone.map(|z| crate::insights::localtime::zone_offset(z, at)).or(req.ctx.tz_offset),
        ..req.ctx.clone()
    };
    let mut arms = slot.arms;
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
    let audiences = gate_by_audience(slot.gates, &ctx, at, night, &mut arms);
    if arms.is_empty() {
        return Err(ApiError::Conflict(format!("no variant of slot {:?} is open to this visitor's audiences", req.slot)));
    }
    // The `stated` feature group: present on every decision of a site that has a picker.
    let site_keys = slot.site_keys;
    let said = stated::resolve_with(&site_keys, &ctx.stated, slot.picks)?;
    let said_now = stated::from_features(&stated::with_stated(json!({}), &site_keys, &said));
    let declared = crate::features::values(&slot.declared, &ctx.features, identified)?;
    let key_of = |id: i64| arms.iter().find(|a| a.0 == id).map(|a| (a.1.clone(), a.2.clone()));
    // A running experiment overlays the bandit (see `crate::experiments`); the kill switch wins.
    let experiment = if paused { None } else { slot.experiment };
    let experiment_id = experiment.as_ref().map(|e| e.id);

    // Sticky: a reload inside the TTL gets the same decision, so the hero does not reroll.
    if let Some((id, variant_id, features, scores, seed, version, held, policy, auds, cold, exp_id, allocation)) = slot.sticky {
        // A visitor who has stated something new since gets a fresh decision, and so does one
        // decided before an experiment started or stopped.
        let same_statement = stated::from_features(&features) == said_now;
        if let (true, true, Some((key, payload))) = (same_statement, exp_id == experiment_id, key_of(variant_id)) {
            let auds = auds.unwrap_or_else(|| json!([]));
            let alloc = Allocation { kind: &allocation, experiment: experiment.as_ref() };
            return Ok(response(id, &req.slot, key, payload, &policy, version, features, &scores, seed as u64, held, &arms, auds, cold, alloc));
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

    let (version, state_json) = slot.model.ok_or(sqlx::Error::RowNotFound)?;
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
    // The experiment picks among its arms still open to this visitor, reweighted.
    let assigned = match (&experiment, held) {
        (Some(e), false) => {
            let open: Vec<(i64, f64)> = e.arms.iter().copied().filter(|a| ids.contains(&a.0)).collect();
            (!open.is_empty()).then(|| experiments::assign(e.id, &req.subject, &open))
        }
        _ => None,
    };
    let cold = if held || assigned.is_some() { None } else { cold_start(cold_until, slot.cold, &ids, seed) };
    let chosen = match (held, baseline, assigned, &cold) {
        (true, Some(b), _, _) => b,
        (_, _, Some(arm), _) => arm,
        (_, _, _, Some((arm, _))) => *arm,
        _ => picked,
    };
    let allocation = if assigned.is_some() { "experiment" } else { "bandit" };
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
    // The tombstone check and the insert share one transaction under the subject's lock. The
    // lock is taken with BEGIN, in a statement before the check, so the check's snapshot is
    // taken once the lock is held (an erase holds it exclusively until it commits).
    let mut tx = sqlx::Connection::begin_with(&mut *conn, format!("BEGIN; SELECT pg_advisory_xact_lock_shared({})", privacy::lock_key(&req.site, raw))).await?;
    let erased: bool = sqlx::query_scalar(
        "WITH gone AS (SELECT EXISTS (SELECT 1 FROM erasures WHERE site_id = $2 AND subject_hash = $20) AS g),
              ins AS (
             INSERT INTO decisions (id, site_id, slot_id, variant_id, subject, pool, features, scores, policy, policy_version, seed, in_holdout, created_at, audiences, cold_start,
                                    experiment_id, allocation)
             SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $16, $17, $18, $19 FROM gone WHERE NOT gone.g RETURNING 1),
              hooks AS (
             INSERT INTO webhook_deliveries (subscription_id, event, idempotency_key, payload)
             SELECT id, 'decision', $14, $15 FROM webhook_subscriptions WHERE site_id = $2 AND 'decision' = ANY(events) AND EXISTS (SELECT 1 FROM ins))
         SELECT g FROM gone",
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
    .bind(experiment_id)
    .bind(allocation)
    .bind(privacy::subject_hash(&req.site, raw))
    .fetch_one(&mut *tx)
    .await?;
    if erased {
        return Err(privacy::gone());
    }
    tx.commit().await?;
    let alloc = Allocation { kind: allocation, experiment: experiment.as_ref() };
    Ok(response(id, &req.slot, key, payload, policy, version, features, &scores_json, seed, in_holdout, &arms, audiences, cold, alloc))
}

/// Who chose the treated arm, for the explanation.
struct Allocation<'a> {
    kind: &'a str,
    experiment: Option<&'a experiments::Running>,
}

/// Cold start's pick, `(arm, {exposures, until})`, or `None` once the slot has had `until`
/// learned-from exposures (or `until` is 0). The pick is the arm of `ids` that cold start has
/// served least, ties broken by the decision's seed, so the arms take equal turns. `counts` is
/// [`SNAPSHOT`]'s `(exposures, served per arm)`.
fn cold_start(until: i32, counts: Option<(i64, Vec<(i64, i64)>)>, ids: &[i64], seed: u64) -> Option<(i64, Value)> {
    if until <= 0 {
        return None;
    }
    let (exposures, served) = counts?;
    if exposures >= i64::from(until) {
        return None;
    }
    let turns = |id: i64| served.iter().find(|s| s.0 == id).map_or(0, |s| s.1);
    let fewest = ids.iter().map(|id| turns(*id)).min().unwrap_or(0);
    let due: Vec<i64> = ids.iter().copied().filter(|id| turns(*id) == fewest).collect();
    let arm = due[(seed % due.len() as u64) as usize];
    Some((arm, json!({ "exposures": exposures, "until": until })))
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
    alloc: Allocation<'_>,
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
            // `holdout`, `experiment` (the running experiment's hash chose) or `bandit`.
            "allocation": if in_holdout { "holdout" } else { alloc.kind },
            // The experiment running on the slot when this was decided, if any.
            "experiment": alloc.experiment.map(|e| json!({ "id": e.id, "name": e.name })),
        }),
    }
}

/// A variant's gate: `(variant_id, variant key, audience id, name, predicate, status)`, as
/// [`SNAPSHOT`] reads them for the slot's approved variants, ordered by audience then variant.
type GateRow = (i64, String, i64, String, Value, String);

/// Drop from `arms` every variant whose audience this visitor is outside, and say so:
/// `[{id, name, matched, variants}]`, one entry per audience, ascending id. Fails closed: an
/// audience that is no longer promoted, or reads a local clock the visitor did not report,
/// matches nobody. The baseline is never gated (a CHECK holds it), so the pool is never empty.
fn gate_by_audience(
    gates: Vec<GateRow>,
    ctx: &Ctx,
    at: chrono::DateTime<chrono::Utc>,
    night: crate::insights::localtime::NightWindow,
    arms: &mut Vec<ArmRow>,
) -> Value {
    use crate::insights::{localtime::LocalTime, segments};
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
    Value::Array(out)
}
