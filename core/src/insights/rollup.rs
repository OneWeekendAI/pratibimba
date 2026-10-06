//! The rollup job: exposed decisions into `rollup_hourly`, `rollup_daily` and `rollup_journey`,
//! and visitor activity into `insight_visitor_weeks` (which retention reads).
//!
//! **Dirty hours, recomputed whole.** A run finds every UTC hour that something touched since
//! the last run (an exposure, a settlement, a late conversion) and rebuilds that hour from the
//! raw rows: delete, aggregate, insert, in one transaction. Rebuilding is idempotent, so
//! overlapping runs and the cursor's slack can never double-count, and a decision that was
//! `pending` at one run moves to `converted` at the next. Days follow their hours.
//!
//! **Backfill.** A segment that becomes active carries `backfill_from`; each run rebuilds up to
//! `BACKFILL_DAYS` days from it and moves the cursor on, so a long history is resumable and
//! chunked by day. `GET /api/sites/:id/insights/status` reports the cursor.
//!
//! **Local time.** An exposure's offset is the one the SDK reported on its exposure event
//! (see `localtime`). The local hour and weekday are keys of the hourly row, so the time-of-day
//! chart and segment discovery never need the raw rows again.

use super::localtime::{LocalTime, NightWindow};
use super::segments::{Predicate, Visit, ctx_bits};
use crate::hash::fnv1a64;
use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Timelike, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Rows touched this long before the cursor are rebuilt too, so a transaction that committed
/// late with an older timestamp is still seen.
pub const SLACK: Duration = Duration::minutes(15);
pub const BACKFILL_DAYS: i64 = 7;
/// A journey keeps its first this-many exposures.
pub const MAX_STEPS: usize = 8;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RollupStats {
    pub hours: u64,
    pub days: u64,
    pub exposures: u64,
}

pub(crate) async fn cursor(pool: &PgPool, job: &str) -> Result<Option<(DateTime<Utc>, DateTime<Utc>)>> {
    Ok(sqlx::query_as("SELECT ran_at, cursor FROM insight_jobs WHERE name = $1").bind(job).fetch_optional(pool).await?)
}

pub(crate) async fn set_cursor(pool: &PgPool, job: &str, now: DateTime<Utc>) -> Result<()> {
    sqlx::query("INSERT INTO insight_jobs (name, ran_at, cursor) VALUES ($1, $2, $2) ON CONFLICT (name) DO UPDATE SET ran_at = $2, cursor = $2")
        .bind(job)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(())
}

fn day_of(t: DateTime<Utc>) -> NaiveDate {
    t.date_naive()
}

fn day_start(d: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
}

fn week_of(t: DateTime<Utc>) -> NaiveDate {
    let d = t.date_naive();
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

fn month_start(d: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap()
}

/// Create the month's partition of `rollup_hourly` and `rollup_daily` if it is missing.
pub async fn ensure_partitions(pool: &PgPool, d: NaiveDate) -> Result<()> {
    let from = month_start(d);
    let to = month_start(from + Duration::days(32));
    let suffix = format!("y{}m{:02}", from.year(), from.month());
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(7370726011)").execute(&mut *tx).await?;
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS rollup_hourly_{suffix} PARTITION OF rollup_hourly FOR VALUES FROM ('{from} 00:00:00+00') TO ('{to} 00:00:00+00')"
    ))
    .execute(&mut *tx)
    .await?;
    sqlx::query(&format!("CREATE TABLE IF NOT EXISTS rollup_daily_{suffix} PARTITION OF rollup_daily FOR VALUES FROM ('{from}') TO ('{to}')"))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

struct SiteCtx {
    night: NightWindow,
    segments: Vec<(i64, Predicate)>,
}

async fn site_ctx(pool: &PgPool, site: &str) -> Result<SiteCtx> {
    let (s, e): (i16, i16) = sqlx::query_as("SELECT night_start_hour, night_end_hour FROM sites WHERE id = $1").bind(site).fetch_one(pool).await?;
    let rows: Vec<(i64, Value)> = sqlx::query_as("SELECT id, predicate FROM segments WHERE site_id = $1 AND status IN ('active', 'audience') ORDER BY id")
        .bind(site)
        .fetch_all(pool)
        .await?;
    let segments = rows.into_iter().filter_map(|(id, p)| Predicate::parse(&p).ok().map(|p| (id, p))).collect();
    Ok(SiteCtx { night: NightWindow { start: s as u8, end: e as u8 }, segments })
}

pub async fn run(pool: &PgPool, now: DateTime<Utc>) -> Result<RollupStats> {
    let since = match cursor(pool, "rollup").await? {
        Some((_, c)) => c - SLACK,
        None => DateTime::<Utc>::UNIX_EPOCH,
    };
    let mut dirty: BTreeSet<(String, DateTime<Utc>)> = sqlx::query_as::<_, (String, DateTime<Utc>)>(
        "SELECT DISTINCT site_id, date_trunc('hour', exposed_at, 'UTC') FROM decisions
          WHERE exposed_at IS NOT NULL AND (exposed_at >= $1 OR settled_at >= $1 OR outcome_at >= $1 OR corrected_at >= $1)
            AND site_id IN (SELECT id FROM sites WHERE sandbox_of IS NULL)",
    )
    .bind(since)
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();

    // Backfill: a chunk of days per active segment, cursor saved as it goes.
    let backfills: Vec<(i64, String, NaiveDate)> =
        sqlx::query_as("SELECT id, site_id, backfill_from FROM segments WHERE backfill_from IS NOT NULL AND status IN ('active', 'audience')")
            .fetch_all(pool)
            .await?;
    let mut backfill_done: Vec<(i64, Option<NaiveDate>)> = Vec::new();
    for (id, site, from) in backfills {
        let to = from + Duration::days(BACKFILL_DAYS);
        let hours: Vec<(DateTime<Utc>,)> = sqlx::query_as(
            "SELECT DISTINCT date_trunc('hour', exposed_at, 'UTC') FROM decisions WHERE site_id = $1 AND exposed_at >= $2 AND exposed_at < $3",
        )
        .bind(&site)
        .bind(day_start(from))
        .bind(day_start(to))
        .fetch_all(pool)
        .await?;
        dirty.extend(hours.into_iter().map(|(h,)| (site.clone(), h)));
        backfill_done.push((id, if to > day_of(now) { None } else { Some(to) }));
    }

    let mut stats = RollupStats::default();
    let mut days: BTreeSet<(String, NaiveDate)> = BTreeSet::new();
    let mut ctxs: HashMap<String, SiteCtx> = HashMap::new();
    let mut months: BTreeSet<NaiveDate> = BTreeSet::new();
    for (site, hour) in &dirty {
        let m = month_start(day_of(*hour));
        if months.insert(m) {
            ensure_partitions(pool, m).await?;
        }
        if !ctxs.contains_key(site) {
            ctxs.insert(site.clone(), site_ctx(pool, site).await?);
        }
        stats.exposures += rebuild_hour(pool, site, *hour, &ctxs[site]).await?;
        stats.hours += 1;
        days.insert((site.clone(), day_of(*hour)));
    }
    for (site, day) in &days {
        rebuild_day(pool, site, *day).await?;
        stats.days += 1;
    }

    // Activity for retention: any event, exposure or not.
    sqlx::query(
        "INSERT INTO insight_visitor_weeks (site_id, subject, week)
         SELECT DISTINCT site_id, subject, date_trunc('week', received_at, 'UTC')::date FROM events WHERE received_at >= $1
            AND site_id IN (SELECT id FROM sites WHERE sandbox_of IS NULL)
         ON CONFLICT DO NOTHING",
    )
    .bind(since)
    .execute(pool)
    .await?;

    for (id, next) in backfill_done {
        sqlx::query("UPDATE segments SET backfill_from = $2 WHERE id = $1").bind(id).bind(next).execute(pool).await?;
    }
    set_cursor(pool, "rollup", now).await?;
    Ok(stats)
}

type ExposureRow = (String, i64, i64, Value, DateTime<Utc>, Option<i16>, bool, Option<i16>);

/// Rebuild one site-hour. Returns the number of exposures it counted.
async fn rebuild_hour(pool: &PgPool, site: &str, hour: DateTime<Utc>, sc: &SiteCtx) -> Result<u64> {
    let rows: Vec<ExposureRow> = sqlx::query_as(
        "SELECT d.subject, d.slot_id, d.variant_id, d.features, d.exposed_at, d.outcome, d.settled_at IS NOT NULL, tz.tz_offset_min
           FROM decisions d
           LEFT JOIN LATERAL (
                SELECT e.tz_offset_min FROM events e
                 WHERE e.decision_id = d.id AND e.site_id = d.site_id
                 ORDER BY (e.tz_offset_min IS NULL), (e.name <> 'exposure'), e.received_at LIMIT 1) tz ON true
          WHERE d.site_id = $1 AND d.exposed_at >= $2 AND d.exposed_at < $2 + interval '1 hour'",
    )
    .bind(site)
    .bind(hour)
    .fetch_all(pool)
    .await?;

    type Key = (i16, i16, i64, i64, i64, i16, &'static str);
    let mut cells: BTreeMap<Key, (i64, i64)> = BTreeMap::new();
    let mut weeks: BTreeMap<(String, NaiveDate), BTreeSet<i64>> = BTreeMap::new();
    for (subject, slot, variant, features, exposed_at, outcome, settled, tz) in &rows {
        let local = LocalTime::at(*exposed_at, *tz, sc.night);
        let ctx = ctx_bits(features);
        let visit = Visit { ctx, local };
        let outcome = match (outcome, settled) {
            (Some(1), _) => "converted",
            (Some(_), _) | (None, true) => "not_converted",
            (None, false) => "pending",
        };
        let known = tz.is_some() as i64;
        let segs = std::iter::once(0).chain(sc.segments.iter().filter(|(_, p)| p.matches(&visit)).map(|(id, _)| *id));
        let w = weeks.entry((subject.clone(), week_of(*exposed_at))).or_default();
        for seg in segs {
            let c = cells.entry((local.hour_local as i16, local.dow_local as i16, seg, *slot, *variant, ctx as i16, outcome)).or_default();
            c.0 += 1;
            c.1 += known;
            if seg != 0 {
                w.insert(seg);
            }
        }
    }

    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM rollup_hourly WHERE site_id = $1 AND hour_utc = $2").bind(site).bind(hour).execute(&mut *tx).await?;
    if !cells.is_empty() {
        let mut cols: (Vec<i16>, Vec<i16>, Vec<i64>, Vec<i64>, Vec<i64>, Vec<i16>, Vec<String>, Vec<i64>, Vec<i64>) = Default::default();
        for ((hl, dow, seg, slot, var, ctx, out), (n, known)) in &cells {
            cols.0.push(*hl);
            cols.1.push(*dow);
            cols.2.push(*seg);
            cols.3.push(*slot);
            cols.4.push(*var);
            cols.5.push(*ctx);
            cols.6.push(out.to_string());
            cols.7.push(*n);
            cols.8.push(*known);
        }
        sqlx::query(
            "INSERT INTO rollup_hourly (site_id, hour_utc, hour_local, dow_local, segment_id, slot_id, variant_id, ctx, outcome, n, tz_known)
             SELECT $1, $2, * FROM UNNEST($3::int2[], $4::int2[], $5::int8[], $6::int8[], $7::int8[], $8::int2[], $9::text[], $10::int8[], $11::int8[])",
        )
        .bind(site)
        .bind(hour)
        .bind(&cols.0)
        .bind(&cols.1)
        .bind(&cols.2)
        .bind(&cols.3)
        .bind(&cols.4)
        .bind(&cols.5)
        .bind(&cols.6)
        .bind(&cols.7)
        .bind(&cols.8)
        .execute(&mut *tx)
        .await?;
    }
    for ((subject, week), segs) in weeks {
        let segs: Vec<i64> = segs.into_iter().collect();
        sqlx::query(
            "INSERT INTO insight_visitor_weeks AS w (site_id, subject, week, segments) VALUES ($1, $2, $3, $4)
             ON CONFLICT (site_id, subject, week)
             DO UPDATE SET segments = ARRAY(SELECT DISTINCT s FROM unnest(w.segments || EXCLUDED.segments) s ORDER BY s)",
        )
        .bind(site)
        .bind(subject)
        .bind(week)
        .bind(segs)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(rows.len() as u64)
}

/// Rebuild one site-day of `rollup_daily` (from the hourly rows) and `rollup_journey` (from the
/// day's exposures).
async fn rebuild_day(pool: &PgPool, site: &str, day: NaiveDate) -> Result<()> {
    let (from, to) = (day_start(day), day_start(day + Duration::days(1)));
    let journeys: Vec<(String, i64, i64, Option<i16>)> = sqlx::query_as(
        "SELECT subject, slot_id, variant_id, outcome FROM decisions
          WHERE site_id = $1 AND exposed_at >= $2 AND exposed_at < $3 ORDER BY subject, exposed_at, id",
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;
    let mut per_subject: BTreeMap<&str, (Vec<(i64, i64)>, bool)> = BTreeMap::new();
    for (subject, slot, variant, outcome) in &journeys {
        let j = per_subject.entry(subject.as_str()).or_default();
        if j.0.len() < MAX_STEPS {
            j.0.push((*slot, *variant));
        }
        j.1 |= *outcome == Some(1);
    }
    let mut sketch: BTreeMap<(i64, bool), (Value, i64)> = BTreeMap::new();
    for (steps, converted) in per_subject.into_values() {
        let bytes: Vec<u8> = steps.iter().flat_map(|(s, v)| s.to_le_bytes().into_iter().chain(v.to_le_bytes())).collect();
        let fp = fnv1a64(&[&bytes]) as i64;
        let e = sketch
            .entry((fp, converted))
            .or_insert_with(|| (json!(steps.iter().map(|(s, v)| json!({ "slot": s, "variant": v })).collect::<Vec<_>>()), 0));
        e.1 += 1;
    }

    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM rollup_daily WHERE site_id = $1 AND day = $2").bind(site).bind(day).execute(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO rollup_daily (site_id, day, segment_id, slot_id, variant_id, ctx, outcome, n)
         SELECT site_id, $2, segment_id, slot_id, variant_id, ctx, outcome, sum(n)::int8 FROM rollup_hourly
          WHERE site_id = $1 AND hour_utc >= $3 AND hour_utc < $4
          GROUP BY site_id, segment_id, slot_id, variant_id, ctx, outcome",
    )
    .bind(site)
    .bind(day)
    .bind(from)
    .bind(to)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM rollup_journey WHERE site_id = $1 AND day = $2").bind(site).bind(day).execute(&mut *tx).await?;
    for ((fp, converted), (steps, visitors)) in sketch {
        sqlx::query("INSERT INTO rollup_journey (site_id, day, fingerprint, steps, converted, visitors) VALUES ($1, $2, $3, $4, $5, $6)")
            .bind(site)
            .bind(day)
            .bind(fp)
            .bind(steps)
            .bind(converted)
            .bind(visitors)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Whole hours only; used by the tests and the status report.
pub fn hour_of(t: DateTime<Utc>) -> DateTime<Utc> {
    t.with_minute(0).and_then(|t| t.with_second(0)).and_then(|t| t.with_nanosecond(0)).unwrap()
}
