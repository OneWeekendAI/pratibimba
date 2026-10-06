//! The learning worker: drains the event outbox, credits conversions and updates bandits.
//!
//! Each decision teaches its slot's model exactly once, and only if the visitor saw it:
//!
//! - **Positive.** A goal event (any name other than `exposure` or `missed`) whose name is the
//!   slot's metric credits an exposed, unsettled decision inside its attribution window. The
//!   event names the decision (`decision_id`) or falls back to last-touch: the subject's latest
//!   exposed decision per slot in the window.
//! - **Negative.** An exposed decision whose window closes with no conversion settles as 0.
//! - **Skipped.** An unexposed decision (decided but never shown, e.g. it arrived after the
//!   client's timeout) settles with no outcome and teaches nothing.
//!
//! `decisions.settled_at` is the exactly-once guard. Events are drained BEFORE settling, so a
//! conversion that arrived inside the window but is processed late still counts. The model row
//! is locked `FOR UPDATE` around each read-modify-write, so concurrent workers never lose an
//! update.

use crate::bandit::LinTs;
use crate::http::decide::from_named;
use crate::http::events::CHANNEL;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction, postgres::PgListener};
use std::time::Duration;
use uuid::Uuid;

pub const BATCH: i64 = 500;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub events: u64,
    pub credited: u64,
    pub settled_negative: u64,
    pub skipped: u64,
}

impl Stats {
    fn add(&mut self, o: Stats) {
        self.events += o.events;
        self.credited += o.credited;
        self.settled_negative += o.settled_negative;
        self.skipped += o.skipped;
    }
}

pub async fn run_once(pool: &PgPool, now: DateTime<Utc>) -> Result<Stats> {
    run_once_with(pool, now, BATCH).await
}

pub async fn run_once_with(pool: &PgPool, now: DateTime<Utc>, batch: i64) -> Result<Stats> {
    let mut stats = Stats::default();
    loop {
        let (n, s) = retry(|| drain_batch(pool, now, batch)).await?;
        stats.add(s);
        if n < batch as u64 {
            break;
        }
    }
    loop {
        let (n, s) = retry(|| settle_batch(pool, now, batch)).await?;
        stats.add(s);
        if n < batch as u64 {
            break;
        }
    }
    Ok(stats)
}

/// Two workers can deadlock on a decision row and a model row; Postgres aborts one, and the
/// batch (still in the outbox) is simply tried again.
async fn retry<T, F, Fut>(mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut attempt = 0;
    loop {
        match f().await {
            Err(e) if attempt < 5 && is_retryable(&e) => {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(5 * attempt)).await;
            }
            other => return other,
        }
    }
}

fn is_retryable(e: &anyhow::Error) -> bool {
    e.chain()
        .filter_map(|c| c.downcast_ref::<sqlx::Error>())
        .filter_map(|s| s.as_database_error())
        .any(|d| matches!(d.code().as_deref(), Some("40P01") | Some("40001")))
}

type PendingRow = (i64, Uuid, String, String, Option<Uuid>, DateTime<Utc>);

async fn drain_batch(pool: &PgPool, now: DateTime<Utc>, batch: i64) -> Result<(u64, Stats)> {
    let mut tx = pool.begin().await?;
    let rows: Vec<PendingRow> = sqlx::query_as(
        "SELECT p.id, e.id, e.site_id, e.subject || '|' || e.name, e.decision_id, e.received_at
           FROM pending_events p JOIN events e ON e.id = p.event_id
          ORDER BY p.id LIMIT $1
          FOR UPDATE OF p SKIP LOCKED",
    )
    .bind(batch)
    .fetch_all(&mut *tx)
    .await?;
    let mut local = Stats::default();
    for (_, _, site, subject_name, decision_id, received_at) in &rows {
        let (subject, name) = subject_name.rsplit_once('|').expect("joined above");
        local.events += 1;
        match name {
            "exposure" => {
                if let Some(d) = decision_id {
                    sqlx::query("UPDATE decisions SET exposed_at = $4 WHERE id = $1 AND site_id = $2 AND subject = $3 AND exposed_at IS NULL")
                        .bind(d)
                        .bind(site)
                        .bind(subject)
                        .bind(received_at)
                        .execute(&mut *tx)
                        .await?;
                }
            }
            "missed" => {} // Recorded in `events` for the Phase 1 dashboard; nothing to learn.
            goal => {
                for d in candidates(&mut tx, site, subject, goal, *decision_id, *received_at).await? {
                    if credit(&mut tx, d, *received_at, now).await? {
                        local.credited += 1;
                    }
                }
            }
        }
    }
    let ids: Vec<i64> = rows.iter().map(|r| r.0).collect();
    sqlx::query("DELETE FROM pending_events WHERE id = ANY($1)").bind(&ids).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok((rows.len() as u64, local))
}

/// Decisions a goal event may credit: the one it names (if it belongs to this site, this
/// subject, and a slot measuring this goal), or else the latest exposed in-window decision per
/// slot measuring this goal.
async fn candidates(
    tx: &mut Transaction<'_, Postgres>,
    site: &str,
    subject: &str,
    goal: &str,
    decision_id: Option<Uuid>,
    at: DateTime<Utc>,
) -> Result<Vec<Uuid>> {
    Ok(match decision_id {
        Some(d) => sqlx::query_scalar(
            "SELECT d.id FROM decisions d JOIN slots s ON s.id = d.slot_id
              WHERE d.id = $1 AND d.site_id = $2 AND d.subject = $3 AND s.metric = $4",
        )
        .bind(d)
        .bind(site)
        .bind(subject)
        .bind(goal)
        .fetch_all(&mut **tx)
        .await?,
        None => sqlx::query_scalar(
            "SELECT DISTINCT ON (d.slot_id) d.id FROM decisions d JOIN slots s ON s.id = d.slot_id
              WHERE d.site_id = $1 AND d.subject = $2 AND s.metric = $3 AND d.exposed_at IS NOT NULL
                AND d.created_at <= $4 AND d.created_at + make_interval(secs => s.attribution_window_secs) >= $4
              ORDER BY d.slot_id, d.created_at DESC",
        )
        .bind(site)
        .bind(subject)
        .bind(goal)
        .bind(at)
        .fetch_all(&mut **tx)
        .await?,
    })
}

/// Credit one decision as converted, at most once, and teach its model. Returns whether it
/// was credited.
async fn credit(tx: &mut Transaction<'_, Postgres>, decision: Uuid, at: DateTime<Utc>, now: DateTime<Utc>) -> Result<bool> {
    let row: Option<(i64, i64, Value)> = sqlx::query_as(
        "UPDATE decisions d SET outcome = 1, outcome_at = $2, settled_at = $3
           FROM slots s
          WHERE d.id = $1 AND s.id = d.slot_id
            AND d.settled_at IS NULL AND d.exposed_at IS NOT NULL
            AND $2 >= d.created_at AND $2 <= d.created_at + make_interval(secs => s.attribution_window_secs)
          RETURNING d.slot_id, d.variant_id, d.features",
    )
    .bind(decision)
    .bind(at)
    .bind(now)
    .fetch_optional(&mut **tx)
    .await?;
    match row {
        Some((slot, variant, features)) => {
            learn(tx, slot, variant, &features, true).await?;
            Ok(true)
        }
        None => Ok(false),
    }
}

async fn settle_batch(pool: &PgPool, now: DateTime<Utc>, batch: i64) -> Result<(u64, Stats)> {
    let mut tx = pool.begin().await?;
    let rows: Vec<(Uuid, i64, i64, Value, bool)> = sqlx::query_as(
        "SELECT d.id, d.slot_id, d.variant_id, d.features, d.exposed_at IS NOT NULL
           FROM decisions d JOIN slots s ON s.id = d.slot_id
          WHERE d.settled_at IS NULL AND d.created_at + make_interval(secs => s.attribution_window_secs) < $1
          ORDER BY d.created_at LIMIT $2
          FOR UPDATE OF d SKIP LOCKED",
    )
    .bind(now)
    .bind(batch)
    .fetch_all(&mut *tx)
    .await?;
    let mut local = Stats::default();
    for (id, slot, variant, features, exposed) in &rows {
        if *exposed {
            sqlx::query("UPDATE decisions SET outcome = 0, settled_at = $2 WHERE id = $1").bind(id).bind(now).execute(&mut *tx).await?;
            learn(&mut tx, *slot, *variant, features, false).await?;
            local.settled_negative += 1;
        } else {
            sqlx::query("UPDATE decisions SET settled_at = $2 WHERE id = $1").bind(id).bind(now).execute(&mut *tx).await?;
            local.skipped += 1;
        }
    }
    tx.commit().await?;
    Ok((rows.len() as u64, local))
}

async fn learn(tx: &mut Transaction<'_, Postgres>, slot: i64, variant: i64, features: &Value, converted: bool) -> Result<()> {
    let x = from_named(features).context("decision features are missing a named feature")?;
    let state: Value = sqlx::query_scalar("SELECT state FROM models WHERE slot_id = $1 FOR UPDATE").bind(slot).fetch_one(&mut **tx).await?;
    let mut model: LinTs = serde_json::from_value(state)?;
    model.update(variant, &x, converted);
    sqlx::query("UPDATE models SET state = $2, version = $3, updated_at = now() WHERE slot_id = $1")
        .bind(slot)
        .bind(serde_json::to_value(&model)?)
        .bind(model.n as i64)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Run forever: wake on `NOTIFY prb_events` from the ingestor, and on a timer so windows close
/// even when no events arrive.
pub async fn run_loop(pool: PgPool, tick: Duration) -> Result<()> {
    let mut listener = PgListener::connect_with(&pool).await?;
    listener.listen(CHANNEL).await?;
    loop {
        match run_once(&pool, Utc::now()).await {
            Ok(s) if s != Stats::default() => tracing::info!(?s, "worker pass"),
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "worker pass failed"),
        }
        tokio::select! {
            r = listener.recv() => if let Err(e) = r {
                tracing::warn!(error = %e, "listener dropped; it reconnects on the next recv");
                tokio::time::sleep(tick).await;
            },
            _ = tokio::time::sleep(tick) => {}
        }
    }
}
