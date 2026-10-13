//! The insight routes under `/api` (admin token). Rule 1 of backend.md's query performance:
//! **these read rollups only, never `events` or `decisions`.** `tests/insights.rs` renames both
//! raw tables away and calls every route here.
//!
//! - `GET  /sites/:site/insights/heatmap?axis=local|utc&by=hour|dow_hour&segment=&slot=&days=`
//! - `GET  /sites/:site/insights/journeys?outcome=converted|bounced&days=&limit=`
//! - `GET  /sites/:site/insights/retention?segment=&weeks=`
//! - `GET  /sites/:site/insights/status`
//! - `GET  /sites/:site/segments`            `POST /sites/:site/segments`
//! - `POST /segments/:id/promote`            `POST /segments/:id/archive`

use super::segments::Predicate;
use crate::http::{ApiError, AppState};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{Duration, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/sites/{site}/insights/heatmap", get(heatmap))
        .route("/sites/{site}/insights/journeys", get(journeys))
        .route("/sites/{site}/insights/retention", get(retention))
        .route("/sites/{site}/insights/status", get(status))
        .route("/sites/{site}/segments", get(list_segments).post(create_segment))
        .route("/segments/{id}/promote", post(promote))
        .route("/segments/{id}/archive", post(archive))
}

type St = State<Arc<AppState>>;

async fn site_exists(state: &AppState, site: &str) -> Result<(), ApiError> {
    let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sites WHERE id = $1)").bind(site).fetch_one(&state.pool).await?;
    if ok { Ok(()) } else { Err(ApiError::NotFound(format!("unknown site {site:?}"))) }
}

fn days(d: Option<i64>) -> Result<i64, ApiError> {
    match d.unwrap_or(28) {
        d @ 1..=730 => Ok(d),
        _ => Err(ApiError::BadRequest("days must be 1-730".into())),
    }
}

async fn variant_keys(state: &AppState, site: &str) -> Result<HashMap<i64, (String, String)>, ApiError> {
    let rows: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT v.id, s.key, v.key FROM variants v JOIN slots s ON s.id = v.slot_id WHERE s.site_id = $1").bind(site).fetch_all(&state.pool).await?;
    Ok(rows.into_iter().map(|(id, s, v)| (id, (s, v))).collect())
}

async fn slot_id(state: &AppState, site: &str, key: &Option<String>) -> Result<Option<i64>, ApiError> {
    let Some(k) = key else { return Ok(None) };
    let id: Option<i64> = sqlx::query_scalar("SELECT id FROM slots WHERE site_id = $1 AND key = $2").bind(site).bind(k).fetch_optional(&state.pool).await?;
    id.map(Some).ok_or_else(|| ApiError::NotFound(format!("unknown slot {k:?}")))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeatmapQuery {
    axis: Option<String>,
    by: Option<String>,
    segment: Option<i64>,
    slot: Option<String>,
    days: Option<i64>,
}

/// Exposures and conversions per (hour[, weekday], variant). `exposures` counts settled
/// decisions only, so `rate` is honest; `pending` are the ones still inside their window.
async fn heatmap(State(state): St, Path(site): Path<String>, Query(q): Query<HeatmapQuery>) -> Result<Json<Value>, ApiError> {
    site_exists(&state, &site).await?;
    let local = match q.axis.as_deref().unwrap_or("local") {
        "local" => true,
        "utc" => false,
        _ => return Err(ApiError::BadRequest("axis is local or utc".into())),
    };
    let by_dow = match q.by.as_deref().unwrap_or("hour") {
        "hour" => false,
        "dow_hour" => true,
        _ => return Err(ApiError::BadRequest("by is hour or dow_hour".into())),
    };
    let days = days(q.days)?;
    let segment = q.segment.unwrap_or(0);
    let slot = slot_id(&state, &site, &q.slot).await?;
    let hour = if local { "hour_local" } else { "extract(hour from hour_utc AT TIME ZONE 'UTC')::int2" };
    let dow = match (by_dow, local) {
        (false, _) => "NULL::int2",
        (true, true) => "dow_local",
        (true, false) => "extract(dow from hour_utc AT TIME ZONE 'UTC')::int2",
    };
    let rows: Vec<(i16, Option<i16>, i64, i64, f64, i64, i64)> = sqlx::query_as(&format!(
        "SELECT {hour} AS h, {dow} AS d, variant_id,
                coalesce(sum(n) FILTER (WHERE outcome <> 'pending'), 0)::int8,
                coalesce(sum(credit) FILTER (WHERE outcome = 'converted'), 0)::float8,
                coalesce(sum(n) FILTER (WHERE outcome = 'pending'), 0)::int8,
                sum(tz_known)::int8
           FROM rollup_hourly
          WHERE site_id = $1 AND segment_id = $2 AND hour_utc >= $3 AND ($4::int8 IS NULL OR slot_id = $4)
          GROUP BY 1, 2, 3 ORDER BY 2 NULLS FIRST, 1, 3"
    ))
    .bind(&site)
    .bind(segment)
    .bind(Utc::now() - Duration::days(days))
    .bind(slot)
    .fetch_all(&state.pool)
    .await?;
    let names = variant_keys(&state, &site).await?;
    let (mut total, mut known) = (0i64, 0i64);
    let cells: Vec<Value> = rows
        .iter()
        .map(|(h, d, v, n, c, p, k)| {
            total += n + p;
            known += k;
            let (slot, variant) = names.get(v).cloned().unwrap_or_default();
            let mut cell = json!({ "hour": h, "slot": slot, "variant": variant, "variant_id": v, "exposures": n, "conversions": super::count_json(*c),
                                   "rate": if *n > 0 { *c / *n as f64 } else { 0.0 }, "pending": p });
            if let Some(d) = d {
                cell["dow"] = json!(d);
            }
            cell
        })
        .collect();
    Ok(Json(json!({
        "site": site, "axis": if local { "local" } else { "utc" }, "segment": segment, "days": days, "cells": cells,
        // How much of the local axis rests on a reported offset rather than the UTC fallback.
        "tz_known_share": if total > 0 { known as f64 / total as f64 } else { 0.0 },
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JourneyQuery {
    outcome: Option<String>,
    days: Option<i64>,
    limit: Option<i64>,
}

/// The top paths that end in a conversion (or in none), summed over the window.
async fn journeys(State(state): St, Path(site): Path<String>, Query(q): Query<JourneyQuery>) -> Result<Json<Value>, ApiError> {
    site_exists(&state, &site).await?;
    let converted = match q.outcome.as_deref().unwrap_or("converted") {
        "converted" => true,
        "bounced" => false,
        _ => return Err(ApiError::BadRequest("outcome is converted or bounced".into())),
    };
    let days = days(q.days)?;
    let limit = q.limit.unwrap_or(10).clamp(1, 100);
    let rows: Vec<(i64, Value, i64)> = sqlx::query_as(
        "SELECT fingerprint, (array_agg(steps))[1], sum(visitors)::int8 FROM rollup_journey
          WHERE site_id = $1 AND converted = $2 AND day >= $3
          GROUP BY fingerprint ORDER BY 3 DESC, fingerprint LIMIT $4",
    )
    .bind(&site)
    .bind(converted)
    .bind((Utc::now() - Duration::days(days)).date_naive())
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;
    let names = variant_keys(&state, &site).await?;
    let paths: Vec<Value> = rows
        .into_iter()
        .map(|(fp, steps, visitors)| {
            let steps: Vec<Value> = steps
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| {
                    let v = s["variant"].as_i64().unwrap_or(0);
                    let (slot, variant) = names.get(&v).cloned().unwrap_or_default();
                    json!({ "slot": slot, "variant": variant, "slot_id": s["slot"], "variant_id": v })
                })
                .collect();
            json!({ "fingerprint": format!("{:016x}", fp as u64), "steps": steps, "visitors": visitors })
        })
        .collect();
    Ok(Json(json!({ "site": site, "outcome": if converted { "converted" } else { "bounced" }, "days": days, "paths": paths })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionQuery {
    segment: Option<i64>,
    weeks: Option<i64>,
}

/// The cohort matrix: per cohort week, its size and how many came back n weeks later.
async fn retention(State(state): St, Path(site): Path<String>, Query(q): Query<RetentionQuery>) -> Result<Json<Value>, ApiError> {
    site_exists(&state, &site).await?;
    let weeks = q.weeks.unwrap_or(12).clamp(1, 104);
    let segment = q.segment.unwrap_or(0);
    let rows: Vec<(NaiveDate, NaiveDate, i64)> = sqlx::query_as(
        "SELECT cohort_week, return_week, visitors FROM rollup_retention
          WHERE site_id = $1 AND segment_id = $2 AND cohort_week >= $3 ORDER BY 1, 2",
    )
    .bind(&site)
    .bind(segment)
    .bind((Utc::now() - Duration::weeks(weeks)).date_naive())
    .fetch_all(&state.pool)
    .await?;
    let mut cohorts: Vec<Value> = Vec::new();
    for (cohort, ret, n) in rows {
        let offset = (ret - cohort).num_weeks();
        if cohorts.last().is_none_or(|c| c["cohort_week"] != json!(cohort)) {
            cohorts.push(json!({ "cohort_week": cohort, "size": 0, "returned": [] }));
        }
        let c = cohorts.last_mut().unwrap();
        if offset == 0 {
            c["size"] = json!(n);
        } else {
            c["returned"].as_array_mut().unwrap().push(json!({ "week": offset, "visitors": n }));
        }
    }
    Ok(Json(json!({ "site": site, "segment": segment, "cohorts": cohorts })))
}

async fn status(State(state): St, Path(site): Path<String>) -> Result<Json<Value>, ApiError> {
    site_exists(&state, &site).await?;
    let jobs: Vec<(String, chrono::DateTime<Utc>)> = sqlx::query_as("SELECT name, ran_at FROM insight_jobs ORDER BY name").fetch_all(&state.pool).await?;
    let backfills: Vec<(i64, String, NaiveDate)> =
        sqlx::query_as("SELECT id, name, backfill_from FROM segments WHERE site_id = $1 AND backfill_from IS NOT NULL ORDER BY id")
            .bind(&site)
            .fetch_all(&state.pool)
            .await?;
    Ok(Json(json!({
        "jobs": jobs.into_iter().map(|(n, at)| json!({ "job": n, "ran_at": at })).collect::<Vec<_>>(),
        "backfills": backfills.into_iter().map(|(id, name, from)| json!({ "segment": id, "name": name, "next_day": from })).collect::<Vec<_>>(),
    })))
}

type SegmentRow = (i64, String, Value, String, String, Value, Option<String>, Option<chrono::DateTime<Utc>>, Option<NaiveDate>, chrono::DateTime<Utc>);
const SEGMENT_COLS: &str = "id, name, predicate, origin, status, evidence, promoted_by, promoted_at, backfill_from, created_at";

fn segment_json(r: SegmentRow) -> Value {
    let (id, name, predicate, origin, status, evidence, promoted_by, promoted_at, backfill_from, created_at) = r;
    json!({ "id": id, "name": name, "predicate": predicate, "origin": origin, "status": status, "evidence": evidence,
            "promoted_by": promoted_by, "promoted_at": promoted_at, "backfilling": backfill_from.is_some(), "created_at": created_at })
}

/// Every segment but archived ones, with its last-28-day exposures and conversions from
/// `rollup_daily` (drafts are not rolled up; their numbers are in `evidence`).
async fn list_segments(State(state): St, Path(site): Path<String>) -> Result<Json<Value>, ApiError> {
    site_exists(&state, &site).await?;
    let rows: Vec<SegmentRow> = sqlx::query_as(&format!("SELECT {SEGMENT_COLS} FROM segments WHERE site_id = $1 AND status <> 'archived' ORDER BY id"))
        .bind(&site)
        .fetch_all(&state.pool)
        .await?;
    let totals: HashMap<i64, (i64, f64)> = sqlx::query_as::<_, (i64, i64, f64)>(
        "SELECT segment_id, coalesce(sum(n) FILTER (WHERE outcome <> 'pending'), 0)::int8, coalesce(sum(credit) FILTER (WHERE outcome = 'converted'), 0)::float8
           FROM rollup_daily WHERE site_id = $1 AND day >= $2 GROUP BY 1",
    )
    .bind(&site)
    .bind((Utc::now() - Duration::days(28)).date_naive())
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(s, n, c)| (s, (n, c)))
    .collect();
    let segs: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            let id = r.0;
            let mut v = segment_json(r);
            if let Some((n, c)) = totals.get(&id) {
                v["last_28_days"] = json!({ "exposures": n, "conversions": super::count_json(*c), "rate": if *n > 0 { *c / *n as f64 } else { 0.0 } });
            }
            v
        })
        .collect();
    Ok(Json(json!({ "site": site, "segments": segs })))
}

fn actor(a: &Option<String>) -> Result<String, ApiError> {
    match a.as_deref().map(str::trim) {
        Some(a) if (1..=128).contains(&a.len()) => Ok(a.to_string()),
        _ => Err(ApiError::BadRequest("actor (1-128 characters) is required".into())),
    }
}

/// Where a new or promoted segment's backfill starts: the first day the rollups hold.
async fn first_day(state: &AppState, site: &str) -> Result<NaiveDate, ApiError> {
    let d: Option<NaiveDate> = sqlx::query_scalar("SELECT min(day) FROM rollup_daily WHERE site_id = $1 AND segment_id = 0").bind(site).fetch_one(&state.pool).await?;
    Ok(d.unwrap_or_else(|| Utc::now().date_naive()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSegment {
    name: Option<String>,
    predicate: Value,
    actor: Option<String>,
}

/// A hand-authored segment: active at once, backfilled in the background.
async fn create_segment(State(state): St, Path(site): Path<String>, Json(b): Json<CreateSegment>) -> Result<(StatusCode, Json<Value>), ApiError> {
    site_exists(&state, &site).await?;
    let actor = actor(&b.actor)?;
    let p = Predicate::parse(&b.predicate).map_err(ApiError::BadRequest)?;
    let name = b.name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| p.describe());
    if name.len() > 200 {
        return Err(ApiError::BadRequest("name is at most 200 characters".into()));
    }
    let from = first_day(&state, &site).await?;
    let row: Option<SegmentRow> = sqlx::query_as(&format!(
        "INSERT INTO segments (site_id, name, predicate, predicate_key, origin, status, created_by, backfill_from)
         VALUES ($1, $2, $3, $4, 'authored', 'active', $5, $6) ON CONFLICT (site_id, predicate_key) DO NOTHING RETURNING {SEGMENT_COLS}"
    ))
    .bind(&site)
    .bind(&name)
    .bind(p.to_json())
    .bind(p.key())
    .bind(&actor)
    .bind(from)
    .fetch_optional(&state.pool)
    .await?;
    let row = row.ok_or_else(|| ApiError::Conflict("a segment with this predicate already exists".into()))?;
    Ok((StatusCode::CREATED, Json(segment_json(row))))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorBody {
    actor: Option<String>,
}

/// Promote a segment to an audience: a named owner gives it permission to be used in a
/// decision. A discovered draft becomes tracked by the rollups and is backfilled.
async fn promote(State(state): St, Path(id): Path<i64>, Json(b): Json<ActorBody>) -> Result<Json<Value>, ApiError> {
    let actor = actor(&b.actor)?;
    let cur: Option<(String, String)> = sqlx::query_as("SELECT site_id, status FROM segments WHERE id = $1").bind(id).fetch_optional(&state.pool).await?;
    let (site, status) = cur.ok_or_else(|| ApiError::NotFound(format!("unknown segment {id}")))?;
    if status != "draft" && status != "active" {
        return Err(ApiError::Conflict(format!("segment {id} is {status}; only a draft or active segment can be promoted")));
    }
    let from = if status == "draft" { Some(first_day(&state, &site).await?) } else { None };
    let row: SegmentRow = sqlx::query_as(&format!(
        "UPDATE segments SET status = 'audience', promoted_by = $2, promoted_at = now(), backfill_from = coalesce($3, backfill_from)
          WHERE id = $1 RETURNING {SEGMENT_COLS}"
    ))
    .bind(id)
    .bind(&actor)
    .bind(from)
    .fetch_one(&state.pool)
    .await?;
    crate::edge::changed(&state.pool, &state.edge, &site).await;
    Ok(Json(segment_json(row)))
}

async fn archive(State(state): St, Path(id): Path<i64>, Json(b): Json<ActorBody>) -> Result<Json<Value>, ApiError> {
    actor(&b.actor)?;
    let row: Option<SegmentRow> =
        sqlx::query_as(&format!("UPDATE segments SET status = 'archived', backfill_from = NULL WHERE id = $1 RETURNING {SEGMENT_COLS}"))
            .bind(id)
            .fetch_optional(&state.pool)
            .await?;
    crate::edge::changed_by(&state.pool, &state.edge, "SELECT site_id FROM segments WHERE id = $1", id).await;
    row.map(|r| Json(segment_json(r))).ok_or_else(|| ApiError::NotFound(format!("unknown segment {id}")))
}
