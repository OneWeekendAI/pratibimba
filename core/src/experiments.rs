//! Experiments: a classical A/B test laid over one slot (backend.md's `experiments` table).
//!
//! An experiment names two or more of the slot's approved variants with weights, a start, a
//! stop and a primary metric (the slot's own: it is the only goal a decision's outcome records).
//! While it runs, decide assigns each visitor the arm a hash of (experiment, subject) lands on,
//! in proportion to the weights, instead of asking the bandit; the decision records
//! `allocation = 'experiment'`. The order of the gates is unchanged:
//!
//! - the **kill switch** wins: a paused site serves its baseline and ignores the experiment;
//! - the **holdout** still gets the baseline (its decisions carry the experiment id, so a stop
//!   ends their stickiness too, but not its allocation);
//! - planned **traffic shares** still close arms: the hash picks among the arms still open to
//!   this visitor, reweighted; with none open the bandit decides. A share closes an arm by a hash
//!   of the subject, so the visitors left on each arm are still a random draw;
//! - **audiences** never meet an experiment: an audience closes an arm by who the visitor is, so
//!   its lift would compare different populations. Creating an experiment refuses an arm with an
//!   audience, and an arm of an experiment that has not ended cannot be given one.
//!
//! The bandit **learns from** experiment decisions. LinTS fits each arm's reward against the
//! context; an assignment that depends only on a hash of the subject is independent of the
//! outcome, so the fit stays unbiased and the test's traffic is not wasted.
//!
//! Stopping (or reaching `stops_at`) hands the slot back to the bandit at once: a sticky decision
//! made under a different experiment state is not reused.
//!
//! Results count the experiment's own assignments (closed, exposed windows) per arm, with the
//! same anytime-valid confidence sequences as `results`, and each arm's lift over the control:
//! the baseline when it is an arm, else the first arm.

use crate::http::ApiError;
use crate::stats::{self, ALPHA, Interval};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::PgPool;

/// The arm a subject lands on: `arms` is `(variant_id, weight)`, weights positive.
pub fn assign(experiment: i64, subject: &str, arms: &[(i64, f64)]) -> i64 {
    let total: f64 = arms.iter().map(|a| a.1).sum();
    let u = crate::hash::unit(&[b"experiment", &experiment.to_le_bytes(), subject.as_bytes()]) * total;
    let mut acc = 0.0;
    for (id, w) in arms {
        acc += w;
        if u < acc {
            return *id;
        }
    }
    arms.last().expect("an experiment has arms").0
}

/// A running experiment as decide sees it.
#[derive(Clone, Debug)]
pub struct Running {
    pub id: i64,
    pub name: String,
    pub arms: Vec<(i64, f64)>,
}

/// The experiment running on `slot_id` at `at`, if any.
pub async fn running(pool: &PgPool, slot_id: i64, at: DateTime<Utc>) -> Result<Option<Running>, ApiError> {
    let row: Option<(i64, String, Value)> = sqlx::query_as(
        "SELECT id, name, arms FROM experiments
          WHERE slot_id = $1 AND starts_at <= $2 AND $2 < least(stops_at, coalesce(stopped_at, 'infinity'))
          ORDER BY starts_at LIMIT 1",
    )
    .bind(slot_id)
    .bind(at)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id, name, arms)| Running { id, name, arms: parse_arms(&arms) }))
}

pub(crate) fn parse_arms(v: &Value) -> Vec<(i64, f64)> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| Some((a["variant_id"].as_i64()?, a["weight"].as_f64()?)))
        .collect()
}

pub struct NewArm {
    pub variant: String,
    pub weight: f64,
}

pub struct New {
    pub site: String,
    pub slot: String,
    pub name: String,
    pub metric: Option<String>,
    pub arms: Vec<NewArm>,
    pub starts_at: Option<DateTime<Utc>>,
    pub stops_at: DateTime<Utc>,
    pub actor: String,
}

/// The site's clock: a sandbox's simulated time, else the database's.
async fn site_clock(conn: &mut sqlx::PgConnection, site: &str) -> Result<DateTime<Utc>, ApiError> {
    sqlx::query_scalar("SELECT COALESCE(sim_now, now()) FROM sites WHERE id = $1")
        .bind(site)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("unknown site {site:?}")))
}

pub async fn create(pool: &PgPool, n: New) -> Result<Value, ApiError> {
    let name = n.name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(ApiError::BadRequest("name the experiment (1-200 characters)".into()));
    }
    let mut tx = pool.begin().await?;
    let now = site_clock(&mut tx, &n.site).await?;
    // The slot row lock serialises two creates racing for the same window.
    let slot: Option<(i64, String, String)> = sqlx::query_as("SELECT id, metric, kind FROM slots WHERE site_id = $1 AND key = $2 FOR UPDATE")
        .bind(&n.site)
        .bind(&n.slot)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((slot_id, slot_metric, kind)) = slot else {
        return Err(ApiError::NotFound(format!("unknown slot {:?} on site {:?}", n.slot, n.site)));
    };
    // An experiment assigns one arm; a sorted slot serves an order (see `crate::sorted`).
    if kind == crate::sorted::KIND {
        return Err(ApiError::BadRequest(format!("slot {:?} is sorted: an experiment cannot assign an order", n.slot)));
    }
    let metric = n.metric.unwrap_or_else(|| slot_metric.clone());
    if metric != slot_metric {
        return Err(ApiError::BadRequest(format!(
            "slot {:?} records {slot_metric:?} as its outcome; an experiment on it can only measure that",
            n.slot
        )));
    }
    if n.arms.len() < 2 {
        return Err(ApiError::BadRequest("an experiment needs at least two arms".into()));
    }
    let mut arms: Vec<(i64, String, f64)> = Vec::new();
    for a in &n.arms {
        if !(a.weight.is_finite() && a.weight > 0.0) {
            return Err(ApiError::BadRequest(format!("arm {:?}: weight must be a positive number", a.variant)));
        }
        let status: Option<(i64, String, Option<i64>)> = sqlx::query_as("SELECT id, status, audience_id FROM variants WHERE slot_id = $1 AND key = $2")
            .bind(slot_id)
            .bind(&a.variant)
            .fetch_optional(&mut *tx)
            .await?;
        match status {
            Some((_, s, Some(_))) if s == "approved" => {
                return Err(ApiError::BadRequest(format!(
                    "arm {:?} is gated by an audience; its visitors are not comparable with the other arms'. Clear the audience first",
                    a.variant
                )));
            }
            Some((id, s, None)) if s == "approved" => {
                if arms.iter().any(|x| x.0 == id) {
                    return Err(ApiError::BadRequest(format!("arm {:?} is named twice", a.variant)));
                }
                arms.push((id, a.variant.clone(), a.weight));
            }
            Some((_, s, _)) => return Err(ApiError::BadRequest(format!("arm {:?} is {s}; only approved variants can be arms", a.variant))),
            None => return Err(ApiError::BadRequest(format!("slot {:?} has no variant {:?}", n.slot, a.variant))),
        }
    }
    arms.sort_by_key(|a| a.0);
    let starts = n.starts_at.unwrap_or(now);
    if n.stops_at <= starts {
        return Err(ApiError::BadRequest("stops_at must be after starts_at".into()));
    }
    if n.stops_at <= now {
        return Err(ApiError::BadRequest("stops_at is already past".into()));
    }
    let clash: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM experiments WHERE slot_id = $1
            AND starts_at < $3 AND $2 < least(stops_at, coalesce(stopped_at, 'infinity')) LIMIT 1",
    )
    .bind(slot_id)
    .bind(starts)
    .bind(n.stops_at)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(other) = clash {
        return Err(ApiError::Conflict(format!("experiment {other} already covers that time on slot {:?}", n.slot)));
    }
    let arms_json = Value::Array(arms.iter().map(|(id, _, w)| json!({ "variant_id": id, "weight": w })).collect());
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO experiments (site_id, slot_id, name, metric, arms, starts_at, stops_at, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
    )
    .bind(&n.site)
    .bind(slot_id)
    .bind(name)
    .bind(&metric)
    .bind(&arms_json)
    .bind(starts)
    .bind(n.stops_at)
    .bind(&n.actor)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    one(pool, id).await
}

/// The experiment, not yet ended at its site's clock, that has `variant` as an arm. Call it with
/// the slot row locked, as `create` holds it, so the answer cannot change before the caller acts.
pub async fn unended_with_arm(conn: &mut sqlx::PgConnection, variant: i64) -> Result<Option<i64>, ApiError> {
    Ok(sqlx::query_scalar(
        "SELECT e.id FROM experiments e JOIN variants v ON v.slot_id = e.slot_id JOIN sites t ON t.id = e.site_id
          WHERE v.id = $1 AND e.arms @> jsonb_build_array(jsonb_build_object('variant_id', $1::int8))
            AND COALESCE(t.sim_now, now()) < least(e.stops_at, coalesce(e.stopped_at, 'infinity'))
          ORDER BY e.id LIMIT 1",
    )
    .bind(variant)
    .fetch_optional(conn)
    .await?)
}

/// End an experiment now. Its slot returns to the bandit on the next decision.
pub async fn stop(pool: &PgPool, id: i64, actor: &str) -> Result<Value, ApiError> {
    let row: Option<(String, Option<DateTime<Utc>>, DateTime<Utc>)> =
        sqlx::query_as("SELECT site_id, stopped_at, stops_at FROM experiments WHERE id = $1").bind(id).fetch_optional(pool).await?;
    let Some((site, stopped, stops_at)) = row else {
        return Err(ApiError::NotFound(format!("unknown experiment {id}")));
    };
    let now = site_clock(&mut *pool.acquire().await?, &site).await?;
    if stopped.is_some() || stops_at <= now {
        return Err(ApiError::Conflict(format!("experiment {id} has already stopped")));
    }
    sqlx::query("UPDATE experiments SET stopped_at = $2, stopped_by = $3 WHERE id = $1 AND stopped_at IS NULL")
        .bind(id)
        .bind(now)
        .bind(actor)
        .execute(pool)
        .await?;
    one(pool, id).await
}

async fn one(pool: &PgPool, id: i64) -> Result<Value, ApiError> {
    let site: String = sqlx::query_scalar("SELECT site_id FROM experiments WHERE id = $1").bind(id).fetch_one(pool).await?;
    let all = describe(pool, &site, Some(id)).await?;
    Ok(all.into_iter().next().expect("the experiment exists"))
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Counts {
    pub n: u64,
    pub conversions: u64,
    pub credit: f64,
}

type Row = (i64, String, String, String, Value, DateTime<Utc>, DateTime<Utc>, Option<DateTime<Utc>>, Option<String>, String, DateTime<Utc>, DateTime<Utc>);

/// Every experiment of `site` (or just `only`), newest first, with results per arm.
async fn describe(pool: &PgPool, site: &str, only: Option<i64>) -> Result<Vec<Value>, ApiError> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT e.id, e.name, s.key, e.metric, e.arms, e.starts_at, e.stops_at, e.stopped_at, e.stopped_by, e.created_by, e.created_at,
                COALESCE(t.sim_now, now())
           FROM experiments e JOIN slots s ON s.id = e.slot_id JOIN sites t ON t.id = e.site_id
          WHERE e.site_id = $1 AND ($2::int8 IS NULL OR e.id = $2)
          ORDER BY e.created_at DESC, e.id DESC",
    )
    .bind(site)
    .bind(only)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, name, slot, metric, arms, starts, stops, stopped, stopped_by, by, created, now) in rows {
        let arms = parse_arms(&arms);
        let keys: Vec<(i64, String, bool)> =
            sqlx::query_as("SELECT id, key, is_baseline FROM variants WHERE id = ANY($1) ORDER BY id")
                .bind(arms.iter().map(|a| a.0).collect::<Vec<_>>())
                .fetch_all(pool)
                .await?;
        let key = |vid: i64| keys.iter().find(|k| k.0 == vid).map(|k| k.1.clone()).unwrap_or_default();
        let control = keys.iter().find(|k| k.2).map(|k| k.0).unwrap_or(arms[0].0);
        let ends = stopped.map_or(stops, |s| s.min(stops));
        let status = if now < starts {
            "scheduled"
        } else if now < ends {
            "running"
        } else {
            "stopped"
        };
        // Closed, exposed windows of the experiment's own assignments.
        let counted: Vec<(i64, i64, i64, f64)> = sqlx::query_as(
            "SELECT d.variant_id, count(*), count(*) FILTER (WHERE d.outcome = 1),
                    coalesce(sum(CASE WHEN d.outcome = 1 THEN coalesce(d.credit, 1) ELSE 0 END), 0)::float8
               FROM decisions d JOIN slots s ON s.id = d.slot_id
              WHERE d.experiment_id = $1 AND d.allocation = 'experiment' AND NOT d.in_holdout AND d.exposed_at IS NOT NULL
                AND d.created_at + make_interval(secs => s.attribution_window_secs) < $2
              GROUP BY 1",
        )
        .bind(id)
        .bind(now)
        .fetch_all(pool)
        .await?;
        let counts = |vid: i64| {
            counted
                .iter()
                .find(|c| c.0 == vid)
                .map(|c| Counts { n: c.1 as u64, conversions: c.2 as u64, credit: c.3 })
                .unwrap_or_default()
        };
        let base = counts(control);
        let per_arm: Vec<Value> = arms
            .iter()
            .map(|(vid, w)| {
                let c = counts(*vid);
                let rate: Option<Interval> = stats::rate_cs_credit(c.credit, c.n, ALPHA);
                let lift = if *vid == control { None } else { stats::lift_cs_credit((c.credit, c.n), (base.credit, base.n), ALPHA) };
                json!({ "variant": key(*vid), "weight": w, "counts": c, "rate": rate, "lift": lift })
            })
            .collect();
        out.push(json!({
            "id": id, "name": name, "slot": slot, "metric": metric, "status": status,
            "arms": arms.iter().map(|(vid, w)| json!({ "variant": key(*vid), "variant_id": vid, "weight": w })).collect::<Vec<_>>(),
            "control": key(control),
            "starts_at": starts, "stops_at": stops, "stopped_at": stopped, "stopped_by": stopped_by,
            "created_by": by, "created_at": created,
            "results": { "arms": per_arm },
        }));
    }
    Ok(out)
}

/// `GET /api/sites/:site/experiments` and MCP `list_experiments`.
pub async fn listing(pool: &PgPool, site: &str) -> Result<Value, ApiError> {
    site_clock(&mut *pool.acquire().await?, site).await?;
    Ok(json!({ "alpha": ALPHA, "experiments": describe(pool, site, None).await? }))
}
