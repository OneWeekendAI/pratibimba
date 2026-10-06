//! Segment discovery: the nightly job that suggests segments. Suggestion only: what it finds is
//! a `draft` until an owner promotes it.
//!
//! **What it clusters.** backend.md says discovery clusters visitor profile summaries. Core
//! keeps no per-visitor profile table, and the hourly rollup already is that summary with the
//! subject removed: every exposure lands in exactly one cell of (local hour, local weekday,
//! context bits, slot, variant, outcome). The behaviour/situation features are all boolean, so
//! the exact clusters are the conjunctions of one or two of them; the job scores every one of
//! those over the last `WINDOW_DAYS` of segment-0 rollups. It never reads raw events, so a rare
//! feature cannot leak into a name, and the names are written from the predicate, not by a
//! model (Core makes no LLM calls).
//!
//! **What counts as a finding.** For each candidate and slot, two one-sided z statistics:
//! - *converts more*: the group's conversion rate against everyone else's, all variants pooled;
//! - *variant lift*: a variant's lift over the slot's baseline inside the group, minus the same
//!   lift outside it (the "night visitors convert more on gift" shape).
//!
//! A candidate needs `MIN_SUPPORT` settled exposures on both sides and a score of `Z_MIN`.
//! About 72 candidates x variants x slots are tested a night, so the bar is Bonferroni-sized
//! (z 4.0 is p ~ 3e-5 one-sided). A two-feature candidate must beat both its halves by a full
//! z, or it is the single feature again with a narrower name.

use super::localtime::{LocalTime, NightWindow};
use super::segments::{BOOL_FEATURES, Predicate, Visit};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::collections::HashMap;

pub const WINDOW_DAYS: i64 = 28;
pub const MIN_SUPPORT: i64 = 200;
pub const Z_MIN: f64 = 4.0;
pub const MAX_DRAFTS: usize = 5;

#[derive(Debug, Clone)]
pub struct Finding {
    pub predicate: Predicate,
    pub score: f64,
    pub evidence: Value,
}

#[derive(Default, Clone, Copy)]
struct Tally {
    n: i64,
    c: i64,
}

impl Tally {
    fn add(&mut self, n: i64, converted: bool) {
        self.n += n;
        if converted {
            self.c += n;
        }
    }
    /// Smoothed rate and its variance, so an empty or all-or-nothing cell cannot divide by zero.
    fn rate(&self) -> (f64, f64) {
        let p = (self.c as f64 + 0.5) / (self.n as f64 + 1.0);
        (p, p * (1.0 - p) / (self.n as f64 + 1.0))
    }
    fn json(&self) -> Value {
        json!({ "exposures": self.n, "conversions": self.c, "rate": if self.n > 0 { self.c as f64 / self.n as f64 } else { 0.0 } })
    }
}

fn candidates() -> Vec<Predicate> {
    let atoms: Vec<(&str, bool)> = BOOL_FEATURES.iter().flat_map(|f| [(*f, true), (*f, false)]).collect();
    let mut out: Vec<Predicate> = atoms.iter().map(|a| Predicate::from_bools(&[*a])).collect();
    for (i, a) in atoms.iter().enumerate() {
        for b in &atoms[i + 1..] {
            if a.0 != b.0 {
                out.push(Predicate::from_bools(&[*a, *b]));
            }
        }
    }
    out
}

/// One aggregated cell of the window: (slot, variant, hour_local, dow_local, ctx, converted) -> n.
type Cell = (i64, i64, i16, i16, i16, bool);

/// Score every candidate for one site from its rollups. Pure given the cells, so the statistics
/// are testable without a database.
pub fn score(cells: &[(Cell, i64)], baselines: &HashMap<i64, i64>, night: NightWindow) -> Vec<Finding> {
    let mut slots: Vec<i64> = cells.iter().map(|((s, ..), _)| *s).collect();
    slots.sort();
    slots.dedup();
    let visits: Vec<Visit> = cells
        .iter()
        .map(|((_, _, h, d, ctx, _), _)| Visit { ctx: *ctx as u8, local: LocalTime::from_parts(*h as u8, *d as u8, night) })
        .collect();

    let mut found: Vec<Finding> = Vec::new();
    for p in candidates() {
        let mut best: Option<Finding> = None;
        for &slot in &slots {
            let mut inside: HashMap<i64, Tally> = HashMap::new();
            let mut outside: HashMap<i64, Tally> = HashMap::new();
            for (((s, v, .., conv), n), visit) in cells.iter().zip(&visits) {
                if *s != slot {
                    continue;
                }
                let side = if p.matches(visit) { &mut inside } else { &mut outside };
                side.entry(*v).or_default().add(*n, *conv);
            }
            let pool = |m: &HashMap<i64, Tally>| m.values().fold(Tally::default(), |a, t| Tally { n: a.n + t.n, c: a.c + t.c });
            let (pin, pout) = (pool(&inside), pool(&outside));
            if pin.n < MIN_SUPPORT || pout.n < MIN_SUPPORT {
                continue;
            }
            let ((ri, vi), (ro, vo)) = (pin.rate(), pout.rate());
            let z_more = (ri - ro) / (vi + vo).sqrt();
            let mut f = Finding {
                predicate: p.clone(),
                score: z_more,
                evidence: json!({ "kind": "converts_more", "slot": slot,
                                  "converts_more": { "z": z_more, "inside": pin.json(), "outside": pout.json() } }),
            };
            // The interaction, when the slot has a baseline: the variant whose lift inside the
            // group most exceeds its lift outside. It names the finding when it clears the bar,
            // since "which variant" is what an owner can act on.
            if let Some(base) = baselines.get(&slot) {
                let (bi, bo) = (inside.get(base).copied().unwrap_or_default(), outside.get(base).copied().unwrap_or_default());
                let mut lift: Option<(f64, Value)> = None;
                for (v, ti) in &inside {
                    if v == base || ti.n.min(bi.n) < MIN_SUPPORT / 4 {
                        continue;
                    }
                    let to = outside.get(v).copied().unwrap_or_default();
                    let ((a, va), (b, vb), (c, vc), (d, vd)) = (ti.rate(), bi.rate(), to.rate(), bo.rate());
                    let z = ((a - b) - (c - d)) / (va + vb + vc + vd).sqrt();
                    if lift.as_ref().is_none_or(|(best, _)| z > *best) {
                        lift = Some((z, json!({ "z": z, "variant": v, "baseline": base,
                            "inside": { "variant": ti.json(), "baseline": bi.json() },
                            "outside": { "variant": to.json(), "baseline": bo.json() } })));
                    }
                }
                if let Some((z, block)) = lift {
                    f.evidence["variant_lift"] = block;
                    if z >= Z_MIN {
                        f.evidence["kind"] = json!("variant_lift");
                        f.evidence["variant"] = f.evidence["variant_lift"]["variant"].clone();
                    }
                    f.score = f.score.max(z);
                }
            }
            if best.as_ref().is_none_or(|b| f.score > b.score) {
                best = Some(f);
            }
        }
        if let Some(f) = best.filter(|f| f.score >= Z_MIN) {
            found.push(f);
        }
    }

    // A pair must beat both of its halves by a full z.
    let single: HashMap<String, f64> = found.iter().filter(|f| f.predicate.len() == 1).map(|f| (f.predicate.key(), f.score)).collect();
    found.retain(|f| {
        if f.predicate.len() == 1 {
            return true;
        }
        let halves = f.predicate.to_json();
        halves.as_object().unwrap().iter().all(|(k, v)| {
            let h = Predicate::from_bools(&[(k.as_str(), v.as_bool().unwrap())]);
            f.score >= single.get(&h.key()).copied().unwrap_or(f64::NEG_INFINITY).max(Z_MIN - 1.0) + 1.0
        })
    });
    found.sort_by(|a, b| b.score.total_cmp(&a.score));
    found.truncate(MAX_DRAFTS);
    found
}

/// Run discovery for every site; returns the number of drafts written or refreshed.
pub async fn run(pool: &PgPool, now: DateTime<Utc>) -> Result<u64> {
    let sites: Vec<(String, i16, i16)> = sqlx::query_as("SELECT id, night_start_hour, night_end_hour FROM sites WHERE sandbox_of IS NULL").fetch_all(pool).await?;
    let mut written = 0;
    for (site, ns, ne) in sites {
        let cells: Vec<(i64, i64, i16, i16, i16, bool, i64)> = sqlx::query_as(
            "SELECT slot_id, variant_id, hour_local, dow_local, ctx, outcome = 'converted', sum(n)::int8 FROM rollup_hourly
              WHERE site_id = $1 AND segment_id = 0 AND outcome <> 'pending' AND hour_utc >= $2
              GROUP BY 1, 2, 3, 4, 5, 6",
        )
        .bind(&site)
        .bind(now - Duration::days(WINDOW_DAYS))
        .fetch_all(pool)
        .await?;
        let cells: Vec<(Cell, i64)> = cells.into_iter().map(|(a, b, c, d, e, f, n)| ((a, b, c, d, e, f), n)).collect();
        let baselines: HashMap<i64, i64> = sqlx::query_as("SELECT s.id, v.id FROM slots s JOIN variants v ON v.slot_id = s.id AND v.is_baseline WHERE s.site_id = $1")
            .bind(&site)
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();
        for f in score(&cells, &baselines, NightWindow { start: ns as u8, end: ne as u8 }) {
            let mut evidence = f.evidence.clone();
            evidence["score"] = json!((f.score * 100.0).round() / 100.0);
            evidence["window_days"] = json!(WINDOW_DAYS);
            evidence["found_at"] = json!(now);
            sqlx::query(
                "INSERT INTO segments (site_id, name, predicate, predicate_key, origin, status, evidence, created_by)
                 VALUES ($1, $2, $3, $4, 'discovered', 'draft', $5, 'discovery')
                 ON CONFLICT (site_id, predicate_key) DO UPDATE SET evidence = EXCLUDED.evidence",
            )
            .bind(&site)
            .bind(f.predicate.describe())
            .bind(f.predicate.to_json())
            .bind(f.predicate.key())
            .bind(evidence)
            .execute(pool)
            .await?;
            written += 1;
        }
    }
    Ok(written)
}
