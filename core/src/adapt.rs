//! In-session adaptation: the server side of the SDK's on-device scorer.
//!
//! - **What ships.** For every `in_session` slot the manifest carries each approved variant's
//!   payload and its linear weights by feature name (the posterior mean of the slot's model,
//!   stated features included). The SDK re-scores with a dot product; no model download.
//! - **What comes back.** An on-device decision is still a decision. The SDK posts it as a
//!   `rescore` event (decision id, variant, features, score) and Core writes it to `decisions`
//!   with `source = 'client'` and policy [`POLICY`]. Exposure and clicks credit it like any
//!   other, and the worker learns from it.
//! - **Reconciliation.** `divergence` is |client score - server posterior mean| for the same
//!   features. Above [`DIVERGENCE`] the shipped weights were stale: the decision is logged and
//!   is never served as the sticky decision, so the server's next full decision wins.
//! - **Guards.** A holdout visitor, or a paused site, gets no client decision recorded, and a
//!   paused site ships no weights at all.
//! - **Picks.** A `pick` event carries 1 to `max_picks` distinct option keys of that picker
//!   slot; anything else refuses the batch.

use crate::bandit::{self, LinTs};
use crate::hash;
use crate::http::ApiError;
use crate::http::decide::from_named;
use crate::http::events::EventIn;
use crate::http::manifest::Manifest;
use crate::stated;
use serde_json::{Map, Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::{BTreeSet, HashMap};
use uuid::Uuid;

pub const POLICY: &str = "on_device";
/// Client and server disagree by more than this (in conversion probability): stale weights.
pub const DIVERGENCE: f64 = 0.15;

struct SlotInfo {
    id: i64,
    kind: String,
    in_session: bool,
    picker: Option<stated::PickerSpec>,
    /// A sorted slot's stored `{visible, order}`.
    sorted: Option<Value>,
}

async fn slots(pool: &PgPool, site: &str) -> Result<HashMap<String, SlotInfo>, ApiError> {
    let rows: Vec<(String, i64, String, bool, Option<Value>, Option<Value>)> =
        sqlx::query_as("SELECT key, id, kind, in_session, picker, sorted FROM slots WHERE site_id = $1").bind(site).fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|(key, id, kind, in_session, picker, sorted)| {
            let picker = picker.and_then(|p| serde_json::from_value(p).ok());
            (key, SlotInfo { id, kind, in_session, picker, sorted })
        })
        .collect())
}

/// A validated on-device decision, ready to write in the ingest transaction.
pub struct ClientDecision {
    id: Uuid,
    slot_id: i64,
    variant_id: i64,
    pool: Vec<i64>,
    features: Value,
    score: f64,
    version: i64,
    divergence: f64,
}

/// Validate the batch's picks and rescores. Returns the client decisions to record (none for a
/// holdout visitor or a paused site). Plain events pass through untouched.
///
/// A rescore may carry the site's declared features as `feature:<key>`; a PII one is dropped
/// from a request that is not `identified`, as it is from a server decision. It may carry the
/// time features too (`sin_hour` .. `is_business_hours_local`), which the SDK computes from the
/// browser clock once the slot's model has learned them.
pub async fn check(pool: &PgPool, site: &str, subject: &str, events: &[EventIn], identified: bool) -> Result<Vec<ClientDecision>, ApiError> {
    let bad = |m: String| Err(ApiError::BadRequest(m));
    let special = |e: &EventIn| e.name == "pick" || e.name == "rescore" || e.picks.is_some() || e.variant.is_some() || e.features.is_some() || e.score.is_some();
    if !events.iter().any(special) {
        return Ok(vec![]);
    }
    let slots = slots(pool, site).await?;
    let site_keys = stated::site_keys(pool, site).await?;
    let declared = crate::features::declared(pool, site).await?;
    let feature_name = |k: &String| format!("{}{k}", crate::features::PREFIX);
    let mut out = vec![];
    for e in events {
        if e.name != "pick" && e.picks.is_some() {
            return bad(format!("event {:?}: only a pick carries picks", e.name));
        }
        if e.name != "rescore" && (e.variant.is_some() || e.features.is_some() || e.score.is_some()) {
            return bad(format!("event {:?}: only a rescore carries variant, features and score", e.name));
        }
        let slot = e.slot.as_ref().and_then(|k| slots.get(k));
        if e.name == "pick" {
            let Some(p) = slot.and_then(|s| s.picker.as_ref()) else { return bad("a pick names a picker slot".into()) };
            let picks = e.picks.as_deref().unwrap_or_default();
            let distinct: BTreeSet<&String> = picks.iter().collect();
            if picks.is_empty() || picks.len() > p.max_picks as usize || distinct.len() != picks.len() {
                return bad(format!("a pick carries 1-{} distinct options", p.max_picks));
            }
            if let Some(k) = picks.iter().find(|k| !p.options.iter().any(|o| &o.key == *k)) {
                return bad(format!("{k:?} is not an option of this picker"));
            }
        }
        if e.name == "rescore" {
            let Some(s) = slot.filter(|s| s.in_session && s.kind == "swap") else { return bad("a rescore names an in_session slot".into()) };
            let (Some(id), Some(variant), Some(features), Some(score)) = (e.decision_id, &e.variant, &e.features, e.score) else {
                return bad("a rescore carries decision_id, variant, features and score".into());
            };
            if !(0.0..=1.0).contains(&score) {
                return bad("score must be a probability".into());
            }
            let Some(fm) = features.as_object() else { return bad("features must be an object".into()) };
            let allowed: BTreeSet<String> = bandit::FEATURE_NAMES
                .iter()
                .map(|n| n.to_string())
                .chain(site_keys.iter().map(|k| stated::feature_name(k)))
                .chain(declared.keys().map(feature_name))
                .chain(stated::TIME_FEATURES.iter().map(|n| n.to_string()))
                .collect();
            if let Some(k) = fm.keys().find(|k| !allowed.contains(*k)) {
                return bad(format!("feature {k:?} is not part of the decision context"));
            }
            // The hour's sine and cosine span -1..1; every other feature is a 0-1 value.
            let range = |k: &str| if k == "sin_hour" || k == "cos_hour" { -1.0..=1.0 } else { 0.0..=1.0 };
            if fm.iter().any(|(k, v)| !v.as_f64().is_some_and(|f| range(k).contains(&f))) {
                return bad("feature values must be numbers in 0-1 (sin_hour and cos_hour in -1..1)".into());
            }
            let Some(x) = from_named(features) else { return bad("features must include every base feature".into()) };
            // A PII feature never reaches scoring or storage on an unidentified request.
            let mut kept = fm.clone();
            if !identified {
                for (k, _) in declared.iter().filter(|(_, pii)| **pii) {
                    kept.remove(&feature_name(k));
                }
            }
            let features = &Value::Object(kept);
            let arms: Vec<(i64, String)> =
                sqlx::query_as("SELECT id, key FROM variants WHERE slot_id = $1 AND status = 'approved' ORDER BY id").bind(s.id).fetch_all(pool).await?;
            let Some(variant_id) = arms.iter().find(|a| &a.1 == variant).map(|a| a.0) else {
                return bad(format!("{variant:?} is not an approved variant of this slot"));
            };
            let (fraction, paused, has_baseline): (f32, bool, bool) = sqlx::query_as(
                "SELECT t.holdout_fraction, t.paused_at IS NOT NULL,
                        EXISTS (SELECT 1 FROM variants WHERE slot_id = $2 AND status = 'approved' AND is_baseline)
                   FROM sites t WHERE t.id = $1",
            )
            .bind(site)
            .bind(s.id)
            .fetch_one(pool)
            .await?;
            if paused || (has_baseline && hash::in_holdout(site, subject, f64::from(fraction))) {
                continue;
            }
            let (version, state): (i64, Value) = sqlx::query_as("SELECT version, state FROM models WHERE slot_id = $1").bind(s.id).fetch_one(pool).await?;
            let model: LinTs = serde_json::from_value(state).map_err(anyhow::Error::from)?;
            let server = stated::mean_score(&model, variant_id, &x, features);
            let divergence = (server - score).abs();
            if divergence > DIVERGENCE {
                tracing::warn!(site, slot = s.id, variant_id, server, client = score, "on-device score diverged; the shipped weights are stale");
            }
            out.push(ClientDecision {
                id,
                slot_id: s.id,
                variant_id,
                pool: arms.iter().map(|a| a.0).collect(),
                features: features.clone(),
                score,
                version,
                divergence,
            });
        }
    }
    Ok(out)
}

/// Write the client decisions inside the ingest transaction, before the events that credit
/// them. A retried batch is absorbed by the primary key.
pub async fn record(tx: &mut Transaction<'_, Postgres>, site: &str, subject: &str, decisions: &[ClientDecision]) -> Result<(), ApiError> {
    for d in decisions {
        sqlx::query(
            "INSERT INTO decisions (id, site_id, slot_id, variant_id, subject, pool, features, scores, policy, policy_version, seed,
                                    in_holdout, source, divergence, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 0, false, 'client', $11,
                     (SELECT COALESCE(sim_now, now()) FROM sites WHERE id = $2))
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(d.id)
        .bind(site)
        .bind(d.slot_id)
        .bind(d.variant_id)
        .bind(subject)
        .bind(&d.pool)
        .bind(&d.features)
        .bind(json!({ d.variant_id.to_string(): d.score }))
        .bind(POLICY)
        .bind(d.version)
        .bind(d.divergence as f32)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Add the picker and in-session fields to a loaded manifest.
pub async fn enrich(pool: &PgPool, manifest: &mut Manifest) -> Result<(), ApiError> {
    let site = manifest.site.clone();
    let slots = slots(pool, &site).await?;
    let paused: bool = sqlx::query_scalar("SELECT paused_at IS NOT NULL FROM sites WHERE id = $1").bind(&site).fetch_one(pool).await?;
    // The model's extension, as the server scores it: the stated tastes and the declared features.
    let mut site_keys = stated::site_keys(pool, &site).await?;
    site_keys.extend(manifest.features.iter().map(|f| format!("{}{}", crate::features::PREFIX, f.key)));
    for ms in &mut manifest.slots {
        let Some(info) = slots.get(&ms.key) else { continue };
        if info.kind == "picker" {
            ms.kind = Some("picker".into());
            ms.picker = info.picker.clone();
        }
        // A copy slot (F4): the SDK writes its fields as text, never as HTML.
        if info.kind == crate::copy::KIND {
            ms.kind = Some(crate::copy::KIND.into());
        }
        // A sorted slot: its items in the declared default order (variant ids may disagree with
        // it once the file re-lists them) and how many positions a visitor reads.
        if info.kind == crate::sorted::KIND
            && let Some(spec) = &info.sorted
        {
            ms.kind = Some(crate::sorted::KIND.into());
            ms.sorted = Some(serde_json::json!({ "visible": spec["visible"] }));
            let order: Vec<&str> = spec["order"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            ms.variants.sort_by_key(|v| order.iter().position(|k| *k == v.key).unwrap_or(usize::MAX));
        }
        // The kill switch reaches the device too: a paused site ships nothing to re-score with.
        if !info.in_session || paused {
            continue;
        }
        ms.in_session = Some(true);
        if manifest.night.is_none() {
            let (start, end): (i16, i16) =
                sqlx::query_as("SELECT night_start_hour, night_end_hour FROM sites WHERE id = $1").bind(&site).fetch_one(pool).await?;
            manifest.night = Some(crate::http::manifest::NightSpec { start, end });
        }
        let state: Value = sqlx::query_scalar("SELECT state FROM models WHERE slot_id = $1").bind(info.id).fetch_one(pool).await?;
        let model: LinTs = serde_json::from_value(state).map_err(anyhow::Error::from)?;
        let arms: Vec<(String, i64, Option<String>)> =
            sqlx::query_as("SELECT key, id, payload FROM variants WHERE slot_id = $1 AND status = 'approved'").bind(info.id).fetch_all(pool).await?;
        for v in &mut ms.variants {
            let Some((_, id, payload)) = arms.iter().find(|a| a.0 == v.key) else { continue };
            let weights: Map<String, Value> = stated::weights(&model, &site_keys, *id);
            v.weights = Some(Value::Object(weights));
            v.payload = Some(payload.clone().map(Value::String).unwrap_or(Value::Null));
        }
    }
    Ok(())
}
