//! A1's nightly pass: per site, per slot, the doubly robust uplift of every approved non-baseline
//! arm against the baseline ([`super::uplift`]), judged and capped by [`super::pause`], and a
//! `pause_slot` card in `advisor_cards` for each slot that passes. It reads decisions and writes
//! cards; it never touches `variants` or anything a visitor is served.
//!
//! Rows: exposed, settled decisions with a recomputed propensity (`propensity_kind` `replay` or
//! `experiment`); holdout rows (served by hash, no propensity) and `none` rows are left out, and a
//! one-arm row can never hold both arms. The reward is the decision's conversion credit (1 under
//! last touch, its share under linear). Rows newer than the site's held-out window check the
//! claim and are not trained on.

use super::pause::{self, Candidate, Settings, State};
use super::uplift::{self, Estimate, Row};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};

/// One slot's night, whether or not it gets a card (Results shows "not enough data yet").
#[derive(Debug, Clone)]
pub struct Assessment {
    pub slot_id: i64,
    pub slot_key: String,
    pub candidate: Candidate,
    pub evidence: Value,
}

struct Arm {
    id: i64,
    key: String,
}

struct Slot {
    id: i64,
    key: String,
    metric: String,
    baseline: Option<i64>,
    arms: Vec<Arm>,
}

/// Every real site (sandboxes are run on their own clock with [`run_site`]).
pub async fn run(pool: &PgPool, now: DateTime<Utc>) -> Result<u64> {
    let sites: Vec<String> = sqlx::query_scalar("SELECT id FROM sites WHERE sandbox_of IS NULL ORDER BY id").fetch_all(pool).await?;
    let mut n = 0;
    for site in sites {
        n += run_site(pool, &site, now).await?;
    }
    Ok(n)
}

pub async fn settings(pool: &PgPool, site: &str) -> Result<Settings> {
    let v: Option<Value> = sqlx::query_scalar("SELECT advisor_settings FROM sites WHERE id = $1").bind(site).fetch_optional(pool).await?.flatten();
    Ok(Settings::parse(v.as_ref()))
}

/// The site's slots judged at `now`, one per slot with a baseline and an approved arm.
pub async fn assess(pool: &PgPool, site: &str, now: DateTime<Utc>) -> Result<Vec<Assessment>> {
    let s = settings(pool, site).await?;
    let mut slots: Vec<Slot> = Vec::new();
    let found: Vec<(i64, String, String, i64, String, bool)> = sqlx::query_as(
        "SELECT s.id, s.key, s.metric, v.id, v.key, v.is_baseline
           FROM slots s JOIN variants v ON v.slot_id = s.id
          WHERE s.site_id = $1 AND (v.is_baseline OR v.status = 'approved')
          ORDER BY s.id, v.id",
    )
    .bind(site)
    .fetch_all(pool)
    .await?;
    for (sid, skey, metric, vid, vkey, base) in found {
        if slots.last().is_none_or(|l| l.id != sid) {
            slots.push(Slot { id: sid, key: skey, metric, baseline: None, arms: vec![] });
        }
        let slot = slots.last_mut().expect("pushed");
        if base { slot.baseline = Some(vid) } else { slot.arms.push(Arm { id: vid, key: vkey }) }
    }
    let raw: Vec<(i64, i64, Vec<i64>, f64, f64, DateTime<Utc>)> = sqlx::query_as(
        "SELECT slot_id, variant_id, pool, propensity::float8,
                CASE WHEN outcome = 1 THEN coalesce(credit, 1) ELSE 0 END::float8, created_at
           FROM decisions
          WHERE site_id = $1 AND exposed_at IS NOT NULL AND settled_at IS NOT NULL AND created_at <= $2
            AND propensity_kind IN ('replay', 'experiment') AND propensity IS NOT NULL",
    )
    .bind(site)
    .bind(now)
    .fetch_all(pool)
    .await?;
    let mut by_slot: HashMap<i64, Vec<Row>> = HashMap::new();
    for (slot, arm, pool, propensity, reward, at) in raw {
        by_slot.entry(slot).or_default().push(Row { arm, pool, propensity, reward, at });
    }
    let mut out = Vec::new();
    for slot in slots {
        let (Some(base), false) = (slot.baseline, slot.arms.is_empty()) else { continue };
        let (train, held) = pause::split(by_slot.remove(&slot.id).unwrap_or_default(), now, &s);
        let t: Vec<Option<Estimate>> = slot.arms.iter().map(|a| uplift::dr(&train, a.id, base, s.min_rows())).collect();
        let h: Vec<Option<Estimate>> = slot.arms.iter().map(|a| uplift::dr(&held, a.id, base, 1)).collect();
        let (state, propose) = pause::judge(&t, &h, &s);
        let rows = train.iter().filter(|r| r.pool.contains(&base)).count() as u64;
        let held_rows = held.iter().filter(|r| r.pool.contains(&base)).count() as u64;
        let candidate = Candidate { slot_id: slot.id, state, propose, p: pause::slot_p(&t, &s), rows };
        let from = train.iter().map(|r| r.at).min();
        let evidence = evidence(&slot, &t, &h, &candidate, held_rows, from, now, &s);
        out.push(Assessment { slot_id: slot.id, slot_key: slot.key, candidate, evidence });
    }
    Ok(out)
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn arms_json(arms: &[Arm], es: &[Option<Estimate>]) -> Value {
    json!(
        arms.iter()
            .zip(es)
            .map(|(a, e)| match e {
                Some(e) => json!({ "key": a.key, "uplift": round3(e.uplift), "lo": round3(e.lo), "hi": round3(e.hi), "n": e.n_arm }),
                None => json!({ "key": a.key, "uplift": null, "lo": null, "hi": null, "n": null }),
            })
            .collect::<Vec<_>>()
    )
}

/// 9000 -> "9,000".
fn grouped(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The claim a confident slot's card makes, in one sentence.
pub fn claim(slot: &str, metric: &str, versions: usize, margin: f64, rows: u64) -> String {
    let unit = if matches!(metric, "purchase" | "order" | "orders") { "orders" } else { "conversions" };
    let who = if versions == 1 { "its one version is".to_string() } else { format!("all {versions} versions are") };
    format!("{slot}: {who} within ±{margin} {unit} per 100 of the page's own over {} visits. It makes no difference.", grouped(rows))
}

#[allow(clippy::too_many_arguments)]
fn evidence(slot: &Slot, t: &[Option<Estimate>], h: &[Option<Estimate>], c: &Candidate, held_rows: u64, from: Option<DateTime<Utc>>, now: DateTime<Utc>, s: &Settings) -> Value {
    json!({
        "claim": claim(&slot.key, &slot.metric, slot.arms.len(), s.margin(), c.rows),
        "rows": c.rows,
        "arms": arms_json(&slot.arms, t),
        "held_out": arms_json(&slot.arms, h),
        "visits_per_week": (held_rows as f64 * 7.0 / s.held_out_days() as f64).round() as u64,
        "state": c.state.as_str(),
        "p": c.p,
        "margin": s.margin(),
        "window": { "from": from, "to": now - Duration::days(s.held_out_days()), "held_out_to": now },
    })
}

/// One site at `now`: the cards written.
pub async fn run_site(pool: &PgPool, site: &str, now: DateTime<Utc>) -> Result<u64> {
    let s = settings(pool, site).await?;
    let assessed = assess(pool, site, now).await?;
    let open: Vec<i64> =
        sqlx::query_scalar("SELECT slot_id FROM advisor_cards WHERE site_id = $1 AND kind = 'pause_slot' AND status = 'pending'").bind(site).fetch_all(pool).await?;
    let rejected: Vec<(i64, DateTime<Utc>)> = sqlx::query_as(
        "SELECT slot_id, decided_at FROM advisor_cards WHERE site_id = $1 AND kind = 'pause_slot' AND status = 'rejected' AND decided_at IS NOT NULL",
    )
    .bind(site)
    .fetch_all(pool)
    .await?;
    let recent: i64 = sqlx::query_scalar("SELECT count(*) FROM advisor_cards WHERE site_id = $1 AND created_at > $2 AND created_at <= $3")
        .bind(site)
        .bind(now - Duration::days(7))
        .bind(now)
        .fetch_one(pool)
        .await?;
    let mut blocked: HashSet<i64> = pause::quiet(&rejected, now, &s);
    blocked.extend(open);
    let cands: Vec<Candidate> = assessed.iter().map(|a| a.candidate.clone()).collect();
    let keep = pause::cap(&cands, recent as usize, &blocked, &s);
    let mut tx = pool.begin().await?;
    for c in &keep {
        let a = assessed.iter().find(|a| a.slot_id == c.slot_id).expect("a kept candidate was assessed");
        debug_assert_eq!(c.state, State::Confident);
        sqlx::query("INSERT INTO advisor_cards (site_id, slot_id, kind, state, evidence, created_at) VALUES ($1, $2, 'pause_slot', $3, $4, $5)")
            .bind(site)
            .bind(c.slot_id)
            .bind(c.state.as_str())
            .bind(&a.evidence)
            .bind(now)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(keep.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mutation: the claim prints the rows ungrouped -> red.
    #[test]
    fn claim_reads_as_one_sentence() {
        assert_eq!(
            claim("footer_copy", "purchase", 3, 0.3, 9000),
            "footer_copy: all 3 versions are within ±0.3 orders per 100 of the page's own over 9,000 visits. It makes no difference."
        );
    }

    // Mutation: drop the one-version wording -> red.
    #[test]
    fn claim_for_one_version() {
        assert_eq!(
            claim("strip", "click", 1, 0.5, 1_234_567),
            "strip: its one version is within ±0.5 conversions per 100 of the page's own over 1,234,567 visits. It makes no difference."
        );
    }
}
