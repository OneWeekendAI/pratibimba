//! Lift against the holdout, per site, per slot and per variant.
//!
//! A decision counts once it was **exposed** and its attribution window has **closed**: a
//! conversion is known at once but a miss only when the window ends, so counting open windows
//! would inflate every live rate. A closed, exposed decision the worker has not settled yet
//! counts as a miss, which is what the worker will write.
//!
//! The slot-level number (the policy against the holdout) is causal: the holdout is a random
//! slice of visitors. A per-variant number is not: the bandit chose who saw the variant by
//! context. The dashboard labels per-variant lift as descriptive.
//!
//! Rates and lift count **credit**, not converted decisions: under multi-touch attribution one
//! conversion is shared among several decisions, and its shares sum to one, so a conversion counts
//! once whichever attributor ran. Under last touch credit and conversions are the same number.
//! A shared outcome lies in [0, 1] with mean p, so its variance is at most p(1-p) and the
//! Bernoulli confidence sequence stays valid.

use crate::stats::{self, ALPHA, Interval};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use std::collections::BTreeMap;

/// "Since the beginning": Postgres cannot hold chrono's minimum date.
pub const ALL_TIME: DateTime<Utc> = DateTime::<Utc>::UNIX_EPOCH;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Counts {
    pub n: u64,
    /// Decisions that took any share of a conversion.
    pub conversions: u64,
    /// The conversions credited to those decisions: the sum of their shares.
    pub credit: f64,
}

impl Counts {
    fn add(&mut self, n: u64, s: u64, credit: f64) {
        self.n += n;
        self.conversions += s;
        self.credit += credit;
    }
    fn merge(&mut self, o: Counts) {
        self.add(o.n, o.conversions, o.credit);
    }
    fn pair(&self) -> (f64, u64) {
        (self.credit, self.n)
    }
    pub fn rate(&self) -> Option<f64> {
        (self.n > 0).then(|| self.credit / self.n as f64)
    }
}

/// A decision's credit: its recorded share, or the whole conversion for a row written before
/// attribution was pluggable.
const CREDIT: &str = "coalesce(sum(CASE WHEN d.outcome = 1 THEN coalesce(d.credit, 1) ELSE 0 END), 0)::float8";

#[derive(Clone, Debug, Serialize)]
pub struct VariantResult {
    pub id: i64,
    pub key: String,
    pub status: String,
    pub is_baseline: bool,
    pub traffic_share: Option<f32>,
    /// Treated (non-holdout) closed exposed decisions that showed this variant.
    pub counts: Counts,
    /// Share of the slot's treated decisions in the period.
    pub share: f64,
    /// This variant against the holdout. Descriptive, not causal.
    pub lift: Option<Interval>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SlotResult {
    pub id: i64,
    pub key: String,
    pub metric: String,
    pub treated: Counts,
    pub holdout: Counts,
    /// The policy against the holdout.
    pub lift: Option<Interval>,
    pub variants: Vec<VariantResult>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SiteResult {
    pub site: String,
    pub alpha: f64,
    pub holdout_fraction: f32,
    pub treated: Counts,
    pub holdout: Counts,
    pub lift: Option<Interval>,
    pub slots: Vec<SlotResult>,
}

/// One row per (slot, variant, holdout flag): closed exposed decisions created in `[since, now)`.
type CountRow = (i64, i64, bool, i64, i64, f64);

async fn counts(pool: &PgPool, site: &str, since: DateTime<Utc>, now: DateTime<Utc>) -> Result<Vec<CountRow>> {
    Ok(sqlx::query_as(&format!(
        "SELECT d.slot_id, d.variant_id, d.in_holdout, count(*), count(*) FILTER (WHERE d.outcome = 1), {CREDIT}
           FROM decisions d JOIN slots s ON s.id = d.slot_id
          WHERE d.site_id = $1 AND d.exposed_at IS NOT NULL
            AND d.created_at >= $2
            AND d.created_at + make_interval(secs => s.attribution_window_secs) < $3
          GROUP BY 1, 2, 3"
    ))
    .bind(site)
    .bind(since)
    .bind(now)
    .fetch_all(pool)
    .await?)
}

/// Results for a site over decisions created since `since`, judged at `now`.
pub async fn site(pool: &PgPool, site: &str, since: DateTime<Utc>, now: DateTime<Utc>) -> Result<Option<SiteResult>> {
    let Some(holdout_fraction): Option<f32> =
        sqlx::query_scalar("SELECT holdout_fraction FROM sites WHERE id = $1").bind(site).fetch_optional(pool).await?
    else {
        return Ok(None);
    };
    let slots: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id, key, metric FROM slots WHERE site_id = $1 ORDER BY key").bind(site).fetch_all(pool).await?;
    let variants: Vec<(i64, i64, String, String, bool, Option<f32>)> = sqlx::query_as(
        "SELECT v.id, v.slot_id, v.key, v.status, v.is_baseline, v.traffic_share
           FROM variants v JOIN slots s ON s.id = v.slot_id WHERE s.site_id = $1 ORDER BY v.id",
    )
    .bind(site)
    .fetch_all(pool)
    .await?;
    let rows = counts(pool, site, since, now).await?;

    let mut treated_by_variant: BTreeMap<i64, Counts> = BTreeMap::new();
    let mut by_slot: BTreeMap<i64, (Counts, Counts)> = BTreeMap::new();
    for (slot, variant, held, n, s, c) in rows {
        let entry = by_slot.entry(slot).or_default();
        if held {
            entry.1.add(n as u64, s as u64, c);
        } else {
            entry.0.add(n as u64, s as u64, c);
            treated_by_variant.entry(variant).or_default().add(n as u64, s as u64, c);
        }
    }

    let (mut site_t, mut site_h) = (Counts::default(), Counts::default());
    let mut out = vec![];
    for (slot_id, key, metric) in slots {
        let (treated, holdout) = by_slot.get(&slot_id).copied().unwrap_or_default();
        site_t.merge(treated);
        site_h.merge(holdout);
        let vs = variants
            .iter()
            .filter(|v| v.1 == slot_id)
            .map(|(id, _, vkey, status, is_baseline, share)| {
                let c = treated_by_variant.get(id).copied().unwrap_or_default();
                VariantResult {
                    id: *id,
                    key: vkey.clone(),
                    status: status.clone(),
                    is_baseline: *is_baseline,
                    traffic_share: *share,
                    counts: c,
                    share: if treated.n > 0 { c.n as f64 / treated.n as f64 } else { 0.0 },
                    lift: stats::lift_cs_credit(c.pair(), holdout.pair(), ALPHA),
                }
            })
            .collect();
        out.push(SlotResult {
            id: slot_id,
            key,
            metric,
            treated,
            holdout,
            lift: stats::lift_cs_credit(treated.pair(), holdout.pair(), ALPHA),
            variants: vs,
        });
    }
    Ok(Some(SiteResult {
        site: site.to_string(),
        alpha: ALPHA,
        holdout_fraction,
        treated: site_t,
        holdout: site_h,
        lift: stats::lift_cs_credit(site_t.pair(), site_h.pair(), ALPHA),
        slots: out,
    }))
}

#[derive(Clone, Debug, Serialize)]
pub struct CurvePoint {
    pub t: DateTime<Utc>,
    /// The policy against the holdout, cumulative up to `t`.
    pub policy: Option<Interval>,
    /// Each variant against the holdout, cumulative up to `t`, by key.
    pub variants: BTreeMap<String, Option<Interval>>,
}

/// The cumulative lift curve of one slot, one point per bucket (hour or day, whichever gives at
/// most ~200 points over the data's span).
pub async fn curve(pool: &PgPool, slot_id: i64, now: DateTime<Utc>) -> Result<Vec<CurvePoint>> {
    let span: Option<(DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT min(created_at), max(created_at) FROM decisions WHERE slot_id = $1 AND exposed_at IS NOT NULL HAVING count(*) > 0",
    )
    .bind(slot_id)
    .fetch_optional(pool)
    .await?;
    let Some((first, last)) = span else { return Ok(vec![]) };
    let unit = if last - first > chrono::Duration::days(8) { "day" } else { "hour" };
    let rows: Vec<(DateTime<Utc>, String, bool, i64, i64, f64)> = sqlx::query_as(&format!(
        "SELECT date_trunc('{unit}', d.created_at) b, v.key, d.in_holdout, count(*), count(*) FILTER (WHERE d.outcome = 1), {CREDIT}
           FROM decisions d JOIN slots s ON s.id = d.slot_id JOIN variants v ON v.id = d.variant_id
          WHERE d.slot_id = $1 AND d.exposed_at IS NOT NULL
            AND d.created_at + make_interval(secs => s.attribution_window_secs) < $2
          GROUP BY 1, 2, 3 ORDER BY 1"
    ))
    .bind(slot_id)
    .bind(now)
    .fetch_all(pool)
    .await?;
    let step = if unit == "day" { chrono::Duration::days(1) } else { chrono::Duration::hours(1) };
    let mut points = vec![];
    let (mut treated, mut holdout) = (Counts::default(), Counts::default());
    let mut per: BTreeMap<String, Counts> = BTreeMap::new();
    let mut i = 0;
    while i < rows.len() {
        let bucket = rows[i].0;
        while i < rows.len() && rows[i].0 == bucket {
            let (_, key, held, n, s, c) = &rows[i];
            if *held {
                holdout.add(*n as u64, *s as u64, *c);
            } else {
                treated.add(*n as u64, *s as u64, *c);
                per.entry(key.clone()).or_default().add(*n as u64, *s as u64, *c);
            }
            i += 1;
        }
        points.push(CurvePoint {
            t: bucket + step,
            policy: stats::lift_cs_credit(treated.pair(), holdout.pair(), ALPHA),
            variants: per.iter().map(|(k, c)| (k.clone(), stats::lift_cs_credit(c.pair(), holdout.pair(), ALPHA))).collect(),
        });
    }
    Ok(points)
}
