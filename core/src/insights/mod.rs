//! The insights pipeline (backend.md, "Visitor insights — rollups and segment discovery").
//!
//! The worker aggregates raw rows into rollups; the dashboard's insight routes read only the
//! rollups. Jobs:
//!
//! - `rollup` (every `ROLLUP_EVERY`): `rollup_hourly`, `rollup_daily`, `rollup_journey`, the
//!   per-visitor activity weeks, and segment backfill. See `rollup`.
//! - `nightly` (every 24 h): `rollup_retention` and segment discovery. See `discover`.
//!
//! Simulator sandboxes (`sites.sandbox_of`) are skipped, as the learning worker skips them.
//!
//! Raw events are not pruned: the spec schedules removal only for right-to-erase, and drops
//! old *rollup* partitions, not raw rows. The rollups are partitioned by month for that.

pub mod api;
pub mod discover;
pub mod localtime;
pub mod rollup;
pub mod segments;

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;

pub const ROLLUP_EVERY: Duration = Duration::seconds(60);
pub const NIGHTLY_EVERY: Duration = Duration::hours(24);
/// Only one process runs the insight jobs at a time.
const LOCK: i64 = 0x70726269;

/// Rebuild `rollup_retention` for every site from the activity weeks.
pub async fn retention(pool: &PgPool) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM rollup_retention").execute(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO rollup_retention (site_id, cohort_week, return_week, segment_id, visitors)
         WITH first AS (SELECT site_id, subject, min(week) AS cohort FROM insight_visitor_weeks GROUP BY 1, 2)
         SELECT f.site_id, f.cohort, w.week, s.seg, count(*)
           FROM first f
           JOIN insight_visitor_weeks c ON c.site_id = f.site_id AND c.subject = f.subject AND c.week = f.cohort
           CROSS JOIN LATERAL (SELECT DISTINCT x FROM unnest(array_append(c.segments, 0::int8)) x) AS s(seg)
           JOIN insight_visitor_weeks w ON w.site_id = f.site_id AND w.subject = f.subject AND w.week >= f.cohort
          GROUP BY 1, 2, 3, 4",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NightlyStats {
    pub rollup: rollup::RollupStats,
    pub drafts: u64,
}

/// Rollups, then retention and discovery (which read the rollups).
pub async fn nightly(pool: &PgPool, now: DateTime<Utc>) -> Result<NightlyStats> {
    let r = rollup::run(pool, now).await?;
    retention(pool).await?;
    let drafts = discover::run(pool, now).await?;
    rollup::set_cursor(pool, "nightly", now).await?;
    Ok(NightlyStats { rollup: r, drafts })
}

/// Called from the worker loop: run whatever is due, if no other process holds the lock.
pub async fn maybe_run(pool: &PgPool, now: DateTime<Utc>) -> Result<()> {
    let mut conn = pool.acquire().await?;
    let got: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)").bind(LOCK).fetch_one(&mut *conn).await?;
    if !got {
        return Ok(());
    }
    let due = |ran: Option<(DateTime<Utc>, DateTime<Utc>)>, every: Duration| ran.is_none_or(|(at, _)| now - at >= every);
    let result = async {
        if due(rollup::cursor(pool, "nightly").await?, NIGHTLY_EVERY) {
            let s = nightly(pool, now).await?;
            tracing::info!(?s, "insights nightly pass");
        } else if due(rollup::cursor(pool, "rollup").await?, ROLLUP_EVERY) {
            let s = rollup::run(pool, now).await?;
            if s.hours > 0 {
                tracing::info!(?s, "insights rollup pass");
            }
        }
        anyhow::Ok(())
    }
    .await;
    sqlx::query("SELECT pg_advisory_unlock($1)").bind(LOCK).execute(&mut *conn).await?;
    result
}
