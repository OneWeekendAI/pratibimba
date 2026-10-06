//! The simulator: synthetic visitors and time-travel against a **sandbox** copy of a site.
//!
//! Isolation is structural, not a flag on rows. `reset` copies a site's configuration (site
//! settings, slots, variants) into a separate site row, `<site>~sim`, with a fresh model per
//! slot. Every simulated decision, event and model update is keyed by the sandbox's own site
//! and slot ids, and real results and models are read per site and per slot, so there is no
//! query that could mix the two. The id is never a valid slug, so no real site can be named
//! like a sandbox. The real worker and the site list skip sandboxes; the sandbox's
//! own worker pass (`worker::run_sandbox`) runs at the sandbox's clock.
//!
//! The clock is `sites.sim_now`. `/decide` and `/e` stamp a sandbox's rows with it, so a
//! developer can also point a local SDK at `<site>~sim` and see what thirty simulated days
//! taught the model. `advance` moves the clock forward and runs the sandbox's worker pass, which
//! closes attribution windows exactly as the real worker would at that time.
//!
//! What the sandbox does not copy: the real model (it learns from scratch), the kill switch,
//! and the conversion sources (simulated conversions are the simulator's ground truth, so no
//! goal is platform-authoritative in a sandbox).

use crate::http::events::{EventIn, EventsRequest};
use crate::http::{ApiError, AppState, decide};
use crate::worker;
use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::collections::BTreeMap;
use std::sync::Arc;
use uuid::Uuid;

pub const SUFFIX: &str = "~sim";
pub const MAX_VISITORS: u32 = 10_000;
pub const MAX_SECS: i64 = 366 * 86_400;
pub const DAY: i64 = 86_400;

pub fn sandbox_id(site: &str) -> String {
    format!("{site}{SUFFIX}")
}

/// slot key -> variant key -> conversion probability.
pub type Rates = BTreeMap<String, BTreeMap<String, f64>>;

/// One batch of synthetic visitors. Each visitor gets a decision for every slot, is exposed to
/// it, and converts on the slot's metric with the planted probability for what they were shown.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Cohort {
    pub visitors: u32,
    pub seed: u64,
    /// The planted truth. A variant not listed converts at `default_rate`.
    pub rates: Rates,
    /// Overrides for returning visitors, same shape: plant a segment that prefers another arm.
    pub returning_rates: Rates,
    pub default_rate: f64,
    /// Fractions of visitors with each context flag set.
    pub returning: f64,
    pub mobile: f64,
    pub from_ad: f64,
}

impl Cohort {
    fn validate(&self) -> Result<(), ApiError> {
        if self.visitors > MAX_VISITORS {
            return Err(ApiError::BadRequest(format!("at most {MAX_VISITORS} visitors per batch")));
        }
        let p = |x: f64| (0.0..=1.0).contains(&x);
        let all_rates = self.rates.values().chain(self.returning_rates.values()).flat_map(|m| m.values());
        if !p(self.default_rate) || !p(self.returning) || !p(self.mobile) || !p(self.from_ad) || !all_rates.clone().all(|r| p(*r)) {
            return Err(ApiError::BadRequest("rates and fractions must be between 0 and 1".into()));
        }
        Ok(())
    }

    fn rate(&self, slot: &str, variant: &str, returning: bool) -> f64 {
        let pick = |r: &Rates| r.get(slot).and_then(|m| m.get(variant)).copied();
        (if returning { pick(&self.returning_rates) } else { None }).or_else(|| pick(&self.rates)).unwrap_or(self.default_rate)
    }
}

/// What one simulated day (or one batch) did.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Day {
    pub day: u32,
    /// The clock when the batch arrived.
    pub at: DateTime<Utc>,
    pub visitors: u32,
    /// slot -> variant -> decisions served.
    pub served: BTreeMap<String, BTreeMap<String, u64>>,
    /// slot -> conversion events sent.
    pub conversions: BTreeMap<String, u64>,
    /// What the sandbox's worker passes did: credited, settled as misses.
    pub credited: u64,
    pub settled_negative: u64,
}

impl Day {
    fn absorb(&mut self, s: worker::Stats) {
        self.credited += s.credited;
        self.settled_negative += s.settled_negative;
    }
}

/// The sandbox of `site`, and its clock. 404 when there is none.
async fn sandbox(pool: &PgPool, site: &str) -> Result<(String, DateTime<Utc>), ApiError> {
    let id = sandbox_id(site);
    let at: Option<DateTime<Utc>> = sqlx::query_scalar("SELECT sim_now FROM sites WHERE id = $1 AND sandbox_of = $2")
        .bind(&id)
        .bind(site)
        .fetch_optional(pool)
        .await?;
    at.map(|at| (id, at)).ok_or_else(|| ApiError::NotFound(format!("site {site:?} has no sandbox; reset one first")))
}

pub async fn clock(pool: &PgPool, site: &str) -> Result<DateTime<Utc>, ApiError> {
    Ok(sandbox(pool, site).await?.1)
}

/// Columns of `table` to copy, minus the ones the copy sets itself.
async fn columns(tx: &mut sqlx::PgConnection, table: &str, skip: &[&str]) -> Result<String, ApiError> {
    let cols: Vec<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns
          WHERE table_schema = current_schema() AND table_name = $1 AND is_generated = 'NEVER' AND identity_generation IS NULL
          ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(&mut *tx)
    .await?;
    Ok(cols.into_iter().filter(|c| !skip.contains(&c.as_str())).map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", "))
}

/// Create (or wipe and recreate) the sandbox of a real site, its clock set to `at`.
///
/// Columns are copied by name from the live schema, so a column another migration adds to
/// `sites`, `slots` or `variants` comes along without this code knowing about it.
pub async fn reset(pool: &PgPool, site: &str, at: DateTime<Utc>) -> Result<String, ApiError> {
    let real: Option<Option<String>> = sqlx::query_scalar("SELECT sandbox_of FROM sites WHERE id = $1").bind(site).fetch_optional(pool).await?;
    match real {
        None => return Err(ApiError::NotFound(format!("unknown site {site:?}"))),
        Some(Some(_)) => return Err(ApiError::BadRequest(format!("{site:?} is already a sandbox"))),
        Some(None) => {}
    }
    let id = sandbox_id(site);
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM sites WHERE id = $1 AND sandbox_of = $2").bind(&id).bind(site).execute(&mut *tx).await?;

    let site_cols = columns(
        &mut tx,
        "sites",
        &[
            "id", "created_at", "sandbox_of", "sim_now", "paused_at", "paused_by",
            "stripe_signing_secret", "conversion_signing_secret", "authoritative_goals", "platform_grace_secs",
        ],
    )
    .await?;
    sqlx::query(&format!("INSERT INTO sites (id, sandbox_of, sim_now, {site_cols}) SELECT $2, $1, $3, {site_cols} FROM sites WHERE id = $1"))
        .bind(site)
        .bind(&id)
        .bind(at)
        .execute(&mut *tx)
        .await?;

    let slot_cols = columns(&mut tx, "slots", &["id", "site_id", "created_at"]).await?;
    let variant_cols = columns(&mut tx, "variants", &["id", "slot_id", "created_at", "approved_model_version"]).await?;
    let slots: Vec<(i64, String)> = sqlx::query_as("SELECT id, key FROM slots WHERE site_id = $1 ORDER BY id").bind(site).fetch_all(&mut *tx).await?;
    for (real_slot, _) in &slots {
        let new_slot: i64 = sqlx::query_scalar(&format!(
            "INSERT INTO slots (site_id, {slot_cols}) SELECT $2, {slot_cols} FROM slots WHERE id = $1 RETURNING id"
        ))
        .bind(real_slot)
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
        // A live variant restores to the sandbox model's first version on rollback.
        sqlx::query(&format!(
            "INSERT INTO variants (slot_id, approved_model_version, {variant_cols})
             SELECT $2, CASE WHEN approved_model_version IS NULL THEN NULL ELSE 0 END, {variant_cols} FROM variants WHERE slot_id = $1 ORDER BY id"
        ))
        .bind(real_slot)
        .bind(new_slot)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO models (slot_id) VALUES ($1)").bind(new_slot).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO model_versions (slot_id, version, state, reason) SELECT slot_id, version, state, 'sandbox' FROM models WHERE slot_id = $1")
            .bind(new_slot)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(id)
}

/// Delete a site's sandbox and everything simulated in it.
pub async fn delete(pool: &PgPool, site: &str) -> Result<bool, ApiError> {
    let r = sqlx::query("DELETE FROM sites WHERE id = $1 AND sandbox_of = $2").bind(sandbox_id(site)).bind(site).execute(pool).await?;
    Ok(r.rows_affected() > 0)
}

/// Send one cohort at the sandbox's current clock, then run the sandbox's worker pass.
pub async fn visitors(pool: &PgPool, site: &str, cohort: &Cohort) -> Result<Day, ApiError> {
    cohort.validate()?;
    let (id, at) = sandbox(pool, site).await?;
    let slots: Vec<(String, String)> = sqlx::query_as("SELECT key, metric FROM slots WHERE site_id = $1 ORDER BY id").bind(&id).fetch_all(pool).await?;
    // Mixing the clock in makes each simulated day draw differently from one seed.
    let seed = cohort.seed ^ (at.timestamp() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let state: Arc<AppState> = AppState::with_seed(pool.clone(), seed);
    let mut rng = ChaCha8Rng::seed_from_u64(seed.rotate_left(17));
    let mut day = Day { at, visitors: cohort.visitors, ..Day::default() };

    for _ in 0..cohort.visitors {
        let subject = format!("sim-{:016x}", rng.random::<u64>());
        let ctx = decide::Ctx {
            returning: rng.random_bool(cohort.returning),
            mobile: rng.random_bool(cohort.mobile),
            from_ad: rng.random_bool(cohort.from_ad),
            ..Default::default()
        };
        let mut events = vec![];
        for (slot, metric) in &slots {
            let req = decide::DecideRequest { site: id.clone(), slot: slot.clone(), subject: subject.clone(), ctx: ctx.clone() };
            let d = match decide::decide(&state, &req).await {
                Ok(d) => d,
                // A slot with no approved variants serves nothing; the rest of the page still runs.
                Err(ApiError::Conflict(_)) => continue,
                Err(e) => return Err(e),
            };
            *day.served.entry(slot.clone()).or_default().entry(d.variant.clone()).or_default() += 1;
            events.push(EventIn { id: Uuid::new_v4(), name: "exposure".into(), slot: Some(slot.clone()), decision_id: Some(d.decision_id), ..Default::default() });
            if rng.random_bool(cohort.rate(slot, &d.variant, ctx.returning)) {
                *day.conversions.entry(slot.clone()).or_default() += 1;
                events.push(EventIn { id: Uuid::new_v4(), name: metric.clone(), slot: Some(slot.clone()), decision_id: Some(d.decision_id), ..Default::default() });
            }
        }
        if !events.is_empty() {
            let _accepted = crate::http::events::handler(State(state.clone()), Json(EventsRequest { site: id.clone(), subject, events, tz_offset: None })).await?;
        }
    }
    day.absorb(worker::run_sandbox(pool, &id, at).await?);
    Ok(day)
}

/// Move the sandbox's clock forward by `secs`. With a cohort, time passes a day at a time and
/// the cohort arrives at the start of each simulated day; one `Day` per day comes back. Without
/// one, the clock jumps and the worker closes whatever windows that ends.
pub async fn advance(pool: &PgPool, site: &str, secs: i64, cohort: Option<&Cohort>) -> Result<Vec<Day>, ApiError> {
    if !(1..=MAX_SECS).contains(&secs) {
        return Err(ApiError::BadRequest(format!("advance by 1 second to {} days", MAX_SECS / DAY)));
    }
    let (id, start) = sandbox(pool, site).await?;
    let mut out = vec![];
    let mut elapsed = 0;
    let mut n = 0;
    while elapsed < secs {
        let step = if cohort.is_some() { DAY.min(secs - elapsed) } else { secs };
        n += 1;
        let mut day = match cohort {
            Some(c) => visitors(pool, site, c).await?,
            None => Day { at: start + chrono::Duration::seconds(elapsed), ..Day::default() },
        };
        day.day = n;
        elapsed += step;
        let now = start + chrono::Duration::seconds(elapsed);
        sqlx::query("UPDATE sites SET sim_now = $2 WHERE id = $1").bind(&id).bind(now).execute(pool).await?;
        day.absorb(worker::run_sandbox(pool, &id, now).await?);
        out.push(day);
    }
    Ok(out)
}

/// The sandbox's clock, size and results as of its clock.
pub async fn status(pool: &PgPool, site: &str) -> Result<Value, ApiError> {
    let (id, at) = sandbox(pool, site).await?;
    let (decisions, conversions): (i64, i64) =
        sqlx::query_as("SELECT count(*), count(*) FILTER (WHERE outcome = 1) FROM decisions WHERE site_id = $1").bind(&id).fetch_one(pool).await?;
    let results = crate::results::site(pool, &id, crate::results::ALL_TIME, at).await?;
    Ok(json!({ "site": site, "sandbox": id, "clock": at, "decisions": decisions, "conversions": conversions, "results": results }))
}
