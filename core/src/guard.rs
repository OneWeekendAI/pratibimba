//! Auto-rollback, and the learnings it and the results write.
//!
//! After every settle pass the worker checks each approved, non-baseline variant against its
//! slot's holdout. A variant is paused when, with at least `rollback_min_exposures` closed
//! exposed decisions behind it, the upper end of its lift sequence sits below
//! `-rollback_margin`: the evidence says it is worse than showing nothing new, by more than the
//! margin, at a confidence that survives peeking. Pausing:
//!
//! 1. sets the variant to `paused` and stores its death certificate (counts, rates, interval,
//!    time, the version restored);
//! 2. restores the slot's model to the version current when the variant was approved, as a new
//!    version, so nothing the bad variant taught survives and the history keeps both;
//! 3. writes a sentence to `learnings`.
//!
//! A variant whose interval sits wholly above zero gets one "beats the holdout" learning.

use crate::results::{self, VariantResult};
use crate::worker::write_model;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;

#[derive(Debug, Clone, PartialEq)]
pub struct Paused {
    pub variant_id: i64,
    pub restored_to: Option<i64>,
}

fn pp(x: f64) -> String {
    format!("{:+.1} pp", x * 100.0)
}

fn pct(x: Option<f64>) -> String {
    x.map(|r| format!("{:.1}%", r * 100.0)).unwrap_or_else(|| "n/a".into())
}

/// Check every site. Returns the variants paused in this pass.
pub async fn run(pool: &PgPool, now: DateTime<Utc>) -> Result<Vec<Paused>> {
    let sites: Vec<(String, f32, i32)> =
        sqlx::query_as("SELECT id, rollback_margin, rollback_min_exposures FROM sites ORDER BY id").fetch_all(pool).await?;
    let mut paused = vec![];
    for (site, margin, min_n) in sites {
        let Some(r) = results::site(pool, &site, results::ALL_TIME, now).await? else { continue };
        for slot in &r.slots {
            for v in slot.variants.iter().filter(|v| v.status == "approved" && !v.is_baseline) {
                let Some(lift) = v.lift else { continue };
                if v.counts.n >= min_n as u64 && lift.hi < -f64::from(margin) {
                    if let Some(p) = pause(pool, &site, slot.id, &slot.key, v, slot.holdout, now).await? {
                        paused.push(p);
                    }
                } else if lift.lo > 0.0 {
                    winning(pool, &site, slot.id, &slot.key, v).await?;
                }
            }
        }
    }
    Ok(paused)
}

async fn pause(
    pool: &PgPool,
    site: &str,
    slot: i64,
    slot_key: &str,
    v: &VariantResult,
    holdout: results::Counts,
    now: DateTime<Utc>,
) -> Result<Option<Paused>> {
    let lift = v.lift.expect("checked by the caller");
    let mut tx = pool.begin().await?;
    // Lock the model first, in the same order the worker does, so the two never deadlock.
    sqlx::query("SELECT 1 FROM models WHERE slot_id = $1 FOR UPDATE").bind(slot).execute(&mut *tx).await?;
    let approved_version: Option<Option<i64>> = sqlx::query_scalar(
        "SELECT approved_model_version FROM variants WHERE id = $1 AND status = 'approved' FOR UPDATE",
    )
    .bind(v.id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(approved_version) = approved_version else {
        return Ok(None); // Someone else paused or retired it first.
    };
    let snapshot: Option<Value> = match approved_version {
        Some(ver) => sqlx::query_scalar("SELECT state FROM model_versions WHERE slot_id = $1 AND version = $2")
            .bind(slot)
            .bind(ver)
            .fetch_optional(&mut *tx)
            .await?,
        None => None,
    };
    let restored_to = match &snapshot {
        Some(state) => {
            write_model(&mut tx, slot, state, &format!("rollback: {} paused, restored version {}", v.key, approved_version.unwrap()))
                .await?;
            approved_version
        }
        None => None,
    };
    let certificate = json!({
        "at": now,
        "variant": { "n": v.counts.n, "conversions": v.counts.conversions, "rate": v.counts.rate() },
        "holdout": { "n": holdout.n, "conversions": holdout.conversions, "rate": holdout.rate() },
        "lift": lift,
        "alpha": crate::stats::ALPHA,
        "restored_to_version": restored_to,
    });
    sqlx::query("UPDATE variants SET status = 'paused', paused_at = $2, pause_reason = $3 WHERE id = $1")
        .bind(v.id)
        .bind(now)
        .bind(&certificate)
        .execute(&mut *tx)
        .await?;
    let restored = match restored_to {
        Some(ver) => format!(" The model was restored to version {ver}, from when it was approved."),
        None => " There was no approval checkpoint to restore.".to_string(),
    };
    let sentence = format!(
        "`{}` on `{}` was paused by auto-rollback: it converted {} against {} for the holdout over {} and {} visitors (lift {}, 95% CS {} to {}).{}",
        v.key,
        slot_key,
        pct(v.counts.rate()),
        pct(holdout.rate()),
        v.counts.n,
        holdout.n,
        pp(lift.estimate),
        pp(lift.lo),
        pp(lift.hi),
        restored
    );
    sqlx::query("INSERT INTO learnings (site_id, slot_id, variant_id, kind, sentence, data) VALUES ($1, $2, $3, 'rollback', $4, $5)")
        .bind(site)
        .bind(slot)
        .bind(v.id)
        .bind(&sentence)
        .bind(&certificate)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::warn!(variant = v.id, %sentence, "auto-rollback");
    Ok(Some(Paused { variant_id: v.id, restored_to }))
}

async fn winning(pool: &PgPool, site: &str, slot: i64, slot_key: &str, v: &VariantResult) -> Result<()> {
    let lift = v.lift.expect("checked by the caller");
    let sentence = format!(
        "`{}` on `{}` beats the holdout: {} (95% CS {} to {}) over {} visitors. Per-variant lift is descriptive; the bandit chose who saw it.",
        v.key,
        slot_key,
        pp(lift.estimate),
        pp(lift.lo),
        pp(lift.hi),
        v.counts.n
    );
    sqlx::query(
        "INSERT INTO learnings (site_id, slot_id, variant_id, kind, sentence, data)
         SELECT $1, $2, $3, 'winning', $4, $5
          WHERE NOT EXISTS (SELECT 1 FROM learnings WHERE variant_id = $3 AND kind = 'winning')",
    )
    .bind(site)
    .bind(slot)
    .bind(v.id)
    .bind(&sentence)
    .bind(json!({ "lift": lift, "n": v.counts.n }))
    .execute(pool)
    .await?;
    Ok(())
}
