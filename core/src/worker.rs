//! The learning worker: drains the event outbox, credits conversions and updates bandits.
//!
//! Each decision teaches its slot's model exactly once, only if the visitor saw it, and only
//! when its attribution window has closed (see [`settle_batch`] for why not sooner):
//!
//! - **Positive.** A goal event (any name other than `exposure` or `missed`) whose name is the
//!   slot's metric credits exposed, unsettled decisions inside their attribution window. Which
//!   ones, and with what share, is the slot's attributor's call ([`crate::attribution`]): under
//!   `last_touch` (the default) the decision the event names (`decision_id`), or else the
//!   subject's latest exposed decision per slot in the window, takes the whole conversion; under
//!   `linear` every open exposed decision of the subject in the slot's window takes an equal
//!   share, recorded in `decisions.credit`. The record (outcome, credit, webhook) is written at
//!   once; the model learns the share as a fractional reward when the window closes
//!   (`decisions.learn_pending` until then).
//! - **Negative.** An exposed decision whose window closes with no conversion settles as 0.
//! - **Holdout.** A decision in the holdout gets its outcome recorded like any other (it is the
//!   control group's measurement) but never updates the model.
//! - **Skipped.** An unexposed decision (decided but never shown, e.g. it arrived after the
//!   client's timeout) settles with no outcome and teaches nothing.
//!
//! Phase 3 adds two rules:
//!
//! - **Platform goals are authoritative.** A goal named in the site's `authoritative_goals` is
//!   credited only by a platform event (Stripe, the generic conversion webhook). A browser event
//!   for it is a hint: it credits nothing, but a platform event that names no decision follows
//!   the decision the subject's latest hint named. Such a decision waits `platform_grace_secs`
//!   past its window before it settles as a miss, so a late webhook still lands. A platform
//!   event is judged at the time the platform says it happened, not when it arrived.
//! - **Sandboxes are invisible to the real pass.** `run_once` skips sandbox sites entirely (their
//!   events stay in the outbox); `run_sandbox` runs one sandbox at its own simulated clock.
//!
//! Each pass also sends due webhooks (see `webhooks`): the first exposure of a decision and
//! each credited conversion write their outbox rows inside the same transaction.
//!
//! `decisions.settled_at` is the exactly-once guard. Events are drained BEFORE settling, so a
//! conversion that arrived inside the window but is processed late still counts. The model row
//! is locked `FOR UPDATE` around each read-modify-write, so concurrent workers never lose an
//! update.

use crate::attribution::Touch;
use crate::bandit::LinTs;
use crate::http::decide::from_named;
use crate::http::events::CHANNEL;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use crate::webhooks;
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction, postgres::PgListener};
use std::time::Duration;
use uuid::Uuid;

pub const BATCH: i64 = 500;

/// How long an event that names a decision Core does not have yet waits in the outbox for it.
/// The edge queue (EC3) does not keep order, so an exposure or a click can land before the
/// decision record it names; drained then, it would find nothing to expose or credit and be
/// lost. It waits instead, up to this long (longer than the queue's whole retry schedule, see
/// deploy/proxy-worker), and is then drained as before. Events that name no decision, or one
/// Core has, are never held.
pub const ORPHAN_HOLD_SECS: i64 = 1800;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub events: u64,
    pub credited: u64,
    pub settled_negative: u64,
    pub skipped: u64,
    /// Browser events for an authoritative goal: recorded, never credited.
    pub hints: u64,
    /// Settled misses a late platform conversion turned into conversions.
    pub corrected: u64,
}

impl Stats {
    fn add(&mut self, o: Stats) {
        self.events += o.events;
        self.credited += o.credited;
        self.settled_negative += o.settled_negative;
        self.skipped += o.skipped;
        self.hints += o.hints;
        self.corrected += o.corrected;
    }
}

/// One pass over every real site. Sandbox sites are never touched.
pub async fn run_once(pool: &PgPool, now: DateTime<Utc>) -> Result<Stats> {
    run_once_with(pool, now, BATCH).await
}

pub async fn run_once_with(pool: &PgPool, now: DateTime<Utc>, batch: i64) -> Result<Stats> {
    run_scoped(pool, now, batch, None).await
}

/// One pass over one sandbox site, at its simulated clock `now`.
pub async fn run_sandbox(pool: &PgPool, sandbox: &str, now: DateTime<Utc>) -> Result<Stats> {
    run_scoped(pool, now, BATCH, Some(sandbox)).await
}

/// Which rows a pass may touch: `None` is every real site, `Some(id)` is that sandbox only.
/// `$SCOPE` is the bind index of the scope in each query.
const SCOPE: &str = "(CASE WHEN $SCOPE::text IS NULL THEN t.sandbox_of IS NULL ELSE t.id = $SCOPE AND t.sandbox_of IS NOT NULL END)";

fn scoped(sql: &str, bind: usize) -> String {
    sql.replace("{scope}", &SCOPE.replace("$SCOPE", &format!("${bind}")))
}

async fn run_scoped(pool: &PgPool, now: DateTime<Utc>, batch: i64, scope: Option<&str>) -> Result<Stats> {
    let mut stats = Stats::default();
    loop {
        let (n, s) = retry(|| drain_batch(pool, now, batch, scope)).await?;
        stats.add(s);
        if n < batch as u64 {
            break;
        }
    }
    loop {
        let (n, s) = retry(|| settle_batch(pool, now, batch, scope)).await?;
        stats.add(s);
        if n < batch as u64 {
            break;
        }
    }
    // Right-to-erase: delete the events of erasures whose delay has passed.
    crate::privacy::purge_due(pool, now, &scoped("{scope}", 2), scope).await?;
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

/// `(outbox id, event id, site, subject|name, decision, at, from a platform, authoritative goal)`.
type PendingRow = (i64, Uuid, String, String, Option<Uuid>, DateTime<Utc>, bool, bool);

async fn drain_batch(pool: &PgPool, now: DateTime<Utc>, batch: i64, scope: Option<&str>) -> Result<(u64, Stats)> {
    let mut tx = pool.begin().await?;
    let rows: Vec<PendingRow> = sqlx::query_as(&scoped(
        "SELECT p.id, e.id, e.site_id, e.subject || '|' || e.name, e.decision_id, COALESCE(e.occurred_at, e.received_at),
                e.source <> 'browser', e.name = ANY(t.authoritative_goals)
           FROM pending_events p JOIN live_events e ON e.id = p.event_id JOIN sites t ON t.id = e.site_id
          WHERE {scope}
            AND (e.decision_id IS NULL OR e.received_at <= $3 - make_interval(secs => $4)
                 OR EXISTS (SELECT 1 FROM decisions d WHERE d.id = e.decision_id))
          ORDER BY p.id LIMIT $1
          FOR UPDATE OF p SKIP LOCKED",
        2,
    ))
    .bind(batch)
    .bind(scope)
    .bind(now)
    .bind(ORPHAN_HOLD_SECS as f64)
    .fetch_all(&mut *tx)
    .await?;
    let mut local = Stats::default();
    for (_, _, site, subject_name, decision_id, received_at, platform, authoritative) in &rows {
        let (subject, name) = subject_name.rsplit_once('|').expect("joined above");
        local.events += 1;
        match name {
            "exposure" => {
                if let Some(d) = decision_id {
                    let shown: Option<(String, String, bool)> = sqlx::query_as(
                        "UPDATE decisions d SET exposed_at = $4 FROM slots s, variants v
                          WHERE d.id = $1 AND d.site_id = $2 AND d.subject = $3 AND d.exposed_at IS NULL
                            AND s.id = d.slot_id AND v.id = d.variant_id
                          RETURNING s.key, v.key, d.in_holdout",
                    )
                    .bind(d)
                    .bind(site)
                    .bind(subject)
                    .bind(received_at)
                    .fetch_optional(&mut *tx)
                    .await?;
                    // Only the first exposure of a decision is news.
                    if let Some((slot, variant, held)) = shown {
                        let data = json!({ "decision_id": d, "slot": slot, "variant": variant, "subject": subject,
                                           "in_holdout": held, "exposed_at": received_at });
                        webhooks::enqueue(&mut *tx, site, "exposure", data).await?;
                    }
                }
            }
            "missed" => {} // Recorded in `events` for the Phase 1 dashboard; nothing to learn.
            _ if *authoritative && !*platform => local.hints += 1,
            goal => {
                let decision_id = match decision_id {
                    Some(d) => Some(*d),
                    None if *platform => hinted(&mut tx, site, subject, goal, *received_at).await?,
                    None => None,
                };
                for (d, share) in candidates(&mut tx, site, subject, goal, decision_id, *received_at, *platform).await? {
                    if credit(&mut tx, d, *received_at, now, share).await? {
                        local.credited += 1;
                    } else if *platform && correct(&mut tx, d, *received_at, now, share).await? {
                        local.corrected += 1;
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

/// The decision named by the subject's latest browser hint for this goal, up to `at`.
async fn hinted(tx: &mut Transaction<'_, Postgres>, site: &str, subject: &str, goal: &str, at: DateTime<Utc>) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT decision_id FROM live_events
          WHERE site_id = $1 AND subject = $2 AND name = $3 AND source = 'browser' AND decision_id IS NOT NULL AND received_at <= $4
          ORDER BY received_at DESC LIMIT 1",
    )
    .bind(site)
    .bind(subject)
    .bind(goal)
    .bind(at)
    .fetch_optional(&mut **tx)
    .await?)
}

/// Decisions a goal event credits, with each one's share. The touches are the subject's exposed
/// decisions whose window contains `at`, per slot measuring this goal (only the named decision's
/// slot when the event names one); each slot's attributor shares the conversion among them. A
/// touch is open while it can still take credit: unsettled, or (for a platform event) a settled
/// miss not yet corrected.
async fn candidates(
    tx: &mut Transaction<'_, Postgres>,
    site: &str,
    subject: &str,
    goal: &str,
    decision_id: Option<Uuid>,
    at: DateTime<Utc>,
    platform: bool,
) -> Result<Vec<(Uuid, f64)>> {
    let rows: Vec<(Uuid, i64, String, bool)> = sqlx::query_as(
        "SELECT d.id, d.slot_id,
                coalesce(s.attribution, (SELECT p.attributor FROM plugins p WHERE p.site_id = d.site_id AND p.kind = 'attributor'), t.attribution),
                d.settled_at IS NULL OR ($6 AND d.outcome = 0 AND d.corrected_at IS NULL)
           FROM decisions d JOIN slots s ON s.id = d.slot_id JOIN sites t ON t.id = d.site_id
          WHERE d.site_id = $1 AND d.subject = $2 AND s.metric = $3 AND d.exposed_at IS NOT NULL
            AND d.created_at <= $4 AND d.created_at + make_interval(secs => s.attribution_window_secs) >= $4
            AND ($5::uuid IS NULL OR d.slot_id = (SELECT slot_id FROM decisions WHERE id = $5))
          ORDER BY d.slot_id, d.created_at, d.id",
    )
    .bind(site)
    .bind(subject)
    .bind(goal)
    .bind(at)
    .bind(decision_id)
    .bind(platform)
    .fetch_all(&mut **tx)
    .await?;
    let mut out = vec![];
    for group in rows.chunk_by(|a, b| a.1 == b.1) {
        let attributor = crate::attribution::by_name(&group[0].2).context("unknown attributor")?;
        let touches: Vec<Touch> = group.iter().map(|r| Touch { id: r.0, open: r.3 }).collect();
        out.extend(attributor.credit(&touches, decision_id));
    }
    Ok(out)
}

/// Credit one decision as converted with its `share` of the conversion, at most once. The model
/// is not taught here: a treated decision is marked `learn_pending` and [`settle_batch`] teaches
/// the share when the window closes. Returns whether it was credited.
async fn credit(tx: &mut Transaction<'_, Postgres>, decision: Uuid, at: DateTime<Utc>, now: DateTime<Utc>, share: f64) -> Result<bool> {
    #[allow(clippy::type_complexity)]
    let row: Option<(i64, i64, Value, bool, String, String, String, String, String)> = sqlx::query_as(
        "UPDATE decisions d SET outcome = 1, outcome_at = $2, settled_at = $3, credit = $4, learn_pending = NOT d.in_holdout
           FROM slots s, variants v
          WHERE d.id = $1 AND s.id = d.slot_id AND v.id = d.variant_id
            AND d.settled_at IS NULL AND d.exposed_at IS NOT NULL
            AND $2 >= d.created_at AND $2 <= d.created_at + make_interval(secs => s.attribution_window_secs)
          RETURNING d.slot_id, d.variant_id, d.features, d.in_holdout, d.site_id, s.key, v.key, s.metric, d.subject",
    )
    .bind(decision)
    .bind(at)
    .bind(now)
    .bind(share)
    .fetch_optional(&mut **tx)
    .await?;
    match row {
        Some((_slot, _variant, _features, held, site, slot_key, variant_key, metric, subject)) => {
            let mut data = json!({ "decision_id": decision, "slot": slot_key, "variant": variant_key, "metric": metric,
                                   "subject": subject, "in_holdout": held, "converted_at": at });
            if share < 1.0 {
                data["credit"] = json!(share);
            }
            webhooks::enqueue(&mut **tx, &site, "conversion", data).await?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// A platform conversion for a decision that already settled as a miss, at a time inside its
/// attribution window: the platform's word wins. The record flips to converted and says when it
/// was corrected; the model turns the absorbed miss into a conversion (see
/// [`crate::bandit::Arm::flip_miss`]); the `conversion` webhook fires with `late: true`. At most
/// once per decision (`corrected_at`), and never for a browser event, so a late or replayed
/// browser click cannot rewrite history.
async fn correct(tx: &mut Transaction<'_, Postgres>, decision: Uuid, at: DateTime<Utc>, now: DateTime<Utc>, share: f64) -> Result<bool> {
    #[allow(clippy::type_complexity)]
    let row: Option<(i64, i64, Value, bool, String, String, String, String, String)> = sqlx::query_as(
        "UPDATE decisions d SET outcome = 1, outcome_at = $2, corrected_at = $3, credit = $4
           FROM slots s, variants v
          WHERE d.id = $1 AND s.id = d.slot_id AND v.id = d.variant_id
            AND d.settled_at IS NOT NULL AND d.outcome = 0 AND d.corrected_at IS NULL AND d.exposed_at IS NOT NULL
            AND $2 >= d.created_at AND $2 <= d.created_at + make_interval(secs => s.attribution_window_secs)
          RETURNING d.slot_id, d.variant_id, d.features, d.in_holdout, d.site_id, s.key, v.key, s.metric, d.subject",
    )
    .bind(decision)
    .bind(at)
    .bind(now)
    .bind(share)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((slot, variant, features, held, site, slot_key, variant_key, metric, subject)) = row else { return Ok(false) };
    let mut data = json!({ "decision_id": decision, "slot": slot_key, "variant": variant_key, "metric": metric,
                           "subject": subject, "in_holdout": held, "converted_at": at, "late": true });
    if share < 1.0 {
        data["credit"] = json!(share);
    }
    webhooks::enqueue(&mut **tx, &site, "conversion", data).await?;
    if !held {
        let x = from_named(&features).context("decision features are missing a named feature")?;
        let state: Value = sqlx::query_scalar("SELECT state FROM models WHERE slot_id = $1 FOR UPDATE").bind(slot).fetch_one(&mut **tx).await?;
        let mut model: LinTs = serde_json::from_value(state)?;
        crate::stated::correct_miss_by(&mut model, variant, &x, &features, share);
        write_model(tx, slot, &serde_json::to_value(&model)?, "learn: late conversion corrects a miss").await?;
    }
    Ok(true)
}

/// Close the windows that have ended: teach the model every treated, exposed decision whose
/// window closed, a credited one its share and the rest a miss, and settle the unexposed.
///
/// **Why the model learns a decision only when its window closes (E4).** The worker used to learn
/// a conversion the minute the click was drained and a miss only at window close, so for the
/// newest window of traffic the posterior held successes with no matching failures: E4 measured
/// an inferred 0.97 for an arm that converts at 5%, and the arm with the most recent traffic
/// looked best (rich get richer). The two fixes considered:
///
/// - *Provisional miss at exposure, flipped on a click.* Every pending decision is then counted
///   as a miss although it may still convert, so each arm is underrated by its share of
///   conversions still to come. That share is largest for the arm with the most recent traffic,
///   which is the arm the bandit just chose: the same recency bias with the sign flipped. It
///   vanishes only when conversions arrive instantly; for a purchase metric with a long window it
///   does not.
/// - *Delay both: learn each decision once, at window close, with its final outcome* (this one).
///   Which decisions the model has seen is decided by the clock alone, never by the outcome, so
///   the learned set is an unbiased sample of every arm's outcomes and the posterior is the exact
///   one for the data in it. The cost is lag, not bias: the model is one window behind. Thompson
///   sampling tolerates that well (Chapelle and Li 2011, delayed batch feedback), because an arm
///   with nothing learned keeps a wide posterior and is still explored. A slot that wants faster
///   learning should shorten its window, which is the honest trade.
///
/// A late platform conversion for a settled miss still flips it ([`correct`]): that is new
/// evidence about a decision already learned, not a pending one.
async fn settle_batch(pool: &PgPool, now: DateTime<Utc>, batch: i64, scope: Option<&str>) -> Result<(u64, Stats)> {
    let mut tx = pool.begin().await?;
    // A slot measuring an authoritative goal waits the grace period for a late webhook.
    // Credited decisions are already settled (their record is final); `learn_pending` brings them
    // back once, at the same close, for their lesson.
    #[allow(clippy::type_complexity)]
    let rows: Vec<(Uuid, i64, i64, Value, bool, bool, bool, f64)> = sqlx::query_as(&scoped(
        "SELECT d.id, d.slot_id, d.variant_id, d.features, d.exposed_at IS NOT NULL, d.in_holdout,
                d.learn_pending, coalesce(d.credit, 1)
           FROM decisions d JOIN slots s ON s.id = d.slot_id JOIN sites t ON t.id = d.site_id
          WHERE (d.settled_at IS NULL OR d.learn_pending) AND {scope}
            AND d.created_at + make_interval(secs => s.attribution_window_secs
                  + CASE WHEN s.metric = ANY(t.authoritative_goals) THEN t.platform_grace_secs ELSE 0 END) < $1
          ORDER BY d.created_at LIMIT $2
          FOR UPDATE OF d SKIP LOCKED",
        3,
    ))
    .bind(now)
    .bind(batch)
    .bind(scope)
    .fetch_all(&mut *tx)
    .await?;
    let mut local = Stats::default();
    for (id, slot, variant, features, exposed, held, pending, credit) in &rows {
        if *pending {
            sqlx::query("UPDATE decisions SET learn_pending = false WHERE id = $1").bind(id).execute(&mut *tx).await?;
            learn(&mut tx, *slot, *variant, features, *credit).await?;
        } else if *exposed {
            sqlx::query("UPDATE decisions SET outcome = 0, settled_at = $2 WHERE id = $1").bind(id).bind(now).execute(&mut *tx).await?;
            if !*held {
                learn(&mut tx, *slot, *variant, features, 0.0).await?;
            }
            local.settled_negative += 1;
        } else {
            sqlx::query("UPDATE decisions SET settled_at = $2 WHERE id = $1").bind(id).bind(now).execute(&mut *tx).await?;
            local.skipped += 1;
        }
    }
    tx.commit().await?;
    Ok((rows.len() as u64, local))
}

/// Teach the model one settled decision: `reward` 0 is a miss, 1 a conversion, and anything
/// between a shared conversion.
async fn learn(tx: &mut Transaction<'_, Postgres>, slot: i64, variant: i64, features: &Value, reward: f64) -> Result<()> {
    let x = from_named(features).context("decision features are missing a named feature")?;
    let state: Value = sqlx::query_scalar("SELECT state FROM models WHERE slot_id = $1 FOR UPDATE").bind(slot).fetch_one(&mut **tx).await?;
    let mut model: LinTs = serde_json::from_value(state)?;
    crate::stated::learn_credit(&mut model, variant, &x, features, reward);
    let reason = if reward >= 1.0 {
        "learn: conversion"
    } else if reward > 0.0 {
        "learn: shared conversion"
    } else {
        "learn: miss"
    };
    write_model(tx, slot, &serde_json::to_value(&model)?, reason).await?;
    Ok(())
}

/// Write a new model state as the next version and append it to the history. The caller holds
/// the `models` row lock. Returns the new version.
pub async fn write_model(tx: &mut Transaction<'_, Postgres>, slot: i64, state: &Value, reason: &str) -> Result<i64> {
    let version: i64 = sqlx::query_scalar(
        "UPDATE models SET state = $2, version = version + 1, updated_at = now() WHERE slot_id = $1 RETURNING version",
    )
    .bind(slot)
    .bind(state)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query("INSERT INTO model_versions (slot_id, version, state, reason) VALUES ($1, $2, $3, $4)")
        .bind(slot)
        .bind(version)
        .bind(state)
        .bind(reason)
        .execute(&mut **tx)
        .await?;
    Ok(version)
}

/// Run forever: wake on `NOTIFY prb_events` from the ingestor, and on a timer so windows close
/// even when no events arrive.
pub async fn run_loop(pool: PgPool, tick: Duration) -> Result<()> {
    run_loop_with(pool, tick, GUARD_EVERY).await
}

/// [`run_loop`] that also publishes the edge snapshot ([`crate::edge`]), at most every
/// [`EDGE_EVERY`]: Workers KV takes one write per key per second, and a learning site changes
/// its snapshot on almost every pass.
pub async fn run_loop_edge(pool: PgPool, tick: Duration, edge: crate::edge::Edge) -> Result<()> {
    run_loop_full(pool, tick, GUARD_EVERY, edge).await
}

pub const EDGE_EVERY: Duration = Duration::from_secs(10);

/// How often the loop runs the auto-rollback guard. It reads every closed decision of every
/// site, so it runs on a clock, not on every event.
pub const GUARD_EVERY: Duration = Duration::from_secs(30);

pub async fn run_loop_with(pool: PgPool, tick: Duration, guard_every: Duration) -> Result<()> {
    run_loop_full(pool, tick, guard_every, crate::edge::Edge::off()).await
}

async fn run_loop_full(pool: PgPool, tick: Duration, guard_every: Duration, edge: crate::edge::Edge) -> Result<()> {
    let mut last_edge: Option<std::time::Instant> = None;
    let mut listener = PgListener::connect_with(&pool).await?;
    listener.listen(CHANNEL).await?;
    let mut last_guard: Option<std::time::Instant> = None;
    let http = webhooks::client();
    loop {
        let guard = last_guard.is_none_or(|t| t.elapsed() >= guard_every);
        let _ = pass(&pool, &http, guard).await;
        if edge.is_on() && last_edge.is_none_or(|t| t.elapsed() >= EDGE_EVERY) {
            publish_edge(&pool, &edge).await;
            last_edge = Some(std::time::Instant::now());
        }
        if guard {
            last_guard = Some(std::time::Instant::now());
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

/// One full pass of everything the loop does: real sites, sandboxes, webhooks, insights, and the
/// guard and webhook prune when `guard` is set. Every step runs; each error is logged, and the
/// real-site pass's error (the outbox) is also returned.
pub async fn pass(pool: &PgPool, http: &reqwest::Client, guard: bool) -> Result<()> {
    let main = run_once(pool, Utc::now()).await;
    match &main {
        Ok(s) if *s != Stats::default() => tracing::info!(?s, "worker pass"),
        Ok(_) => {}
        Err(e) => tracing::error!(error = %e, "worker pass failed"),
    }
    // Traffic sent to a sandbox (by the simulator or a local SDK) learns at its own clock.
    match sqlx::query_as::<_, (String, DateTime<Utc>)>("SELECT id, sim_now FROM sites WHERE sandbox_of IS NOT NULL").fetch_all(pool).await {
        Ok(sandboxes) => {
            for (id, at) in sandboxes {
                if let Err(e) = run_sandbox(pool, &id, at).await {
                    tracing::error!(error = %e, sandbox = %id, "sandbox pass failed");
                }
            }
        }
        Err(e) => tracing::error!(error = %e, "listing sandboxes failed"),
    }
    match webhooks::deliver_once(pool, http, Utc::now()).await {
        Ok(p) if p != webhooks::Pass::default() => tracing::info!(?p, "webhook pass"),
        Ok(_) => {}
        Err(e) => tracing::error!(error = %e, "webhook pass failed"),
    }
    if guard {
        if let Err(e) = crate::guard::run(pool, Utc::now()).await {
            tracing::error!(error = %e, "guard pass failed");
        }
        if let Err(e) = webhooks::prune(pool, Utc::now()).await {
            tracing::error!(error = %e, "webhook prune failed");
        }
    }
    if let Err(e) = crate::insights::maybe_run(pool, Utc::now()).await {
        tracing::error!(error = %e, "insights pass failed");
    }
    main.map(|_| ())
}

/// `pratibimba worker --once`: drain, settle, purge and guard once, then return (a Cloud Run job).
pub async fn run_drain(pool: &PgPool) -> Result<()> {
    run_drain_edge(pool, &crate::edge::Edge::off()).await
}

/// [`run_drain`], then publish every site's edge snapshot that changed (learned models,
/// cold-start counts, guard pauses, an experiment's start or end by the clock, or a push a
/// handler failed to make).
pub async fn run_drain_edge(pool: &PgPool, edge: &crate::edge::Edge) -> Result<()> {
    let r = pass(pool, &webhooks::client(), true).await;
    publish_edge(pool, edge).await;
    r
}

async fn publish_edge(pool: &PgPool, edge: &crate::edge::Edge) {
    match crate::edge::publish_all(pool, edge).await {
        Ok(0) => {}
        Ok(n) => tracing::info!(sites = n, "edge snapshots published"),
        Err(e) => tracing::error!(error = %e, "edge publish pass failed"),
    }
}
