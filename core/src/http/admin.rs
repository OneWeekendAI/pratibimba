//! `/api/*`: the dashboard's API. Every route needs the admin token, as a bearer token or as
//! the `prb_admin` cookie `POST /api/session` sets. Owners proxy `/_prb/*` to Core from the
//! public internet, so nothing here may be reachable without it. With no `PRB_ADMIN_TOKEN`
//! configured the whole surface answers 503 and says why.
//!
//! The approval gate lives here: `POST /api/variants/{id}/approve` takes exactly one variant,
//! a named approver and a planned traffic share of at most 50%, and refuses a draft whose
//! constraint checks fail. There is no bulk form.

use super::{ApiError, AppState};
use crate::{checks, explain, probes, results, worker};
use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use chrono::{DateTime, Duration, Utc};
use futures_util::stream::Stream;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

pub const COOKIE: &str = "prb_admin";

pub fn router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    let guarded = Router::new()
        .route("/sites", get(sites))
        .route("/sites/{site}/overview", get(overview_handler))
        .route("/sites/{site}/stream", get(stream))
        .route("/sites/{site}/results", get(site_results))
        .route("/sites/{site}/slots/{slot}", get(slot_detail))
        .route("/sites/{site}/slots/{slot}/variants", post(create_draft))
        .route("/sites/{site}/decisions", get(decisions))
        .route("/sites/{site}/approvals", get(approvals))
        .route("/sites/{site}/install", get(install))
        .route("/sites/{site}/mcp-tokens", post(issue_mcp_token))
        .route("/sites/{site}/mcp-calls", get(mcp_calls))
        .route("/sites/{site}/pause", post(pause_site))
        .route("/mcp-calls/{id}", get(mcp_call))
        .route("/decisions/{id}", get(decision))
        .route("/variants/{id}/approve", post(approve))
        .route("/variants/{id}/reject", post(reject))
        .route("/variants/{id}/comment", post(comment))
        .route("/models/{slot}/rollback", post(rollback))
        .route("/models/{slot}/versions", get(versions))
        .merge(super::sim::routes())
        .merge(super::webhooks::admin_routes())
        .layer(middleware::from_fn_with_state(state, require_admin));
    Router::new().route("/session", post(login).delete(logout)).merge(guarded)
}

/// Equal-time comparison, so the token cannot be guessed one byte at a time.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn presented(headers: &HeaderMap) -> Option<String> {
    if let Some(b) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")) {
        return Some(b.trim().to_string());
    }
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|c| c.trim().split_once('='))
        .find(|(k, _)| *k == COOKIE)
        .map(|(_, v)| v.to_string())
}

async fn require_admin(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let Some(expected) = state.admin_token.as_deref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "the dashboard API is off: set PRB_ADMIN_TOKEN to turn it on" })))
            .into_response();
    };
    match presented(req.headers()) {
        Some(t) if same(&t, expected) => next.run(req).await,
        _ => (StatusCode::UNAUTHORIZED, Json(json!({ "error": "admin token required" }))).into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Login {
    token: String,
}

async fn login(State(state): State<Arc<AppState>>, Json(body): Json<Login>) -> Response {
    let Some(expected) = state.admin_token.as_deref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "the dashboard API is off: set PRB_ADMIN_TOKEN to turn it on" })))
            .into_response();
    };
    if !same(&body.token, expected) {
        return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "wrong token" }))).into_response();
    }
    // SameSite=Strict: no other site can make the browser send it, so no CSRF token is needed.
    let cookie = format!("{COOKIE}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000", body.token);
    (StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response()
}

async fn logout() -> Response {
    (StatusCode::NO_CONTENT, [(header::SET_COOKIE, format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"))]).into_response()
}

async fn site_origin(state: &AppState, site: &str) -> Result<String, ApiError> {
    sqlx::query_scalar("SELECT origin FROM sites WHERE id = $1")
        .bind(site)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("unknown site {site:?}")))
}

async fn sites(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!(site_list(&state.pool).await?)))
}

/// Every site with its slot count and pending drafts. MCP filters it to the token's site.
pub async fn site_list(pool: &sqlx::PgPool) -> Result<Vec<Value>, ApiError> {
    let rows: Vec<(String, String, f32, i64, i64)> = sqlx::query_as(
        "SELECT t.id, t.origin, t.holdout_fraction,
                (SELECT count(*) FROM slots s WHERE s.site_id = t.id),
                (SELECT count(*) FROM variants v JOIN slots s ON s.id = v.slot_id WHERE s.site_id = t.id AND v.status = 'draft')
           FROM sites t WHERE t.sandbox_of IS NULL ORDER BY t.id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, origin, h, slots, pending)| json!({ "id": id, "origin": origin, "holdout_fraction": h, "slots": slots, "pending": pending }))
        .collect())
}

/// The variant a slot is "on": the most-shown approved one, else the approved baseline.
pub fn champion(variants: &[results::VariantResult]) -> Option<&results::VariantResult> {
    let live: Vec<&results::VariantResult> = variants.iter().filter(|v| v.status == "approved").collect();
    live.iter().max_by_key(|v| v.counts.n).filter(|v| v.counts.n > 0).or_else(|| live.iter().find(|v| v.is_baseline)).copied()
}

/// The site home's numbers: three live tiles, a silence diagnosis, and the slot list.
pub async fn overview(state: &AppState, site: &str, now: DateTime<Utc>) -> Result<Value, ApiError> {
    let pool = &state.pool;
    let origin = site_origin(state, site).await?;
    let (last_minute, last_hour_conv, recent5, prev_hour, last_at): (i64, i64, i64, i64, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE created_at > $2 - interval '1 minute' AND created_at <= $2),
                (SELECT count(*) FROM decisions WHERE site_id = $1 AND outcome = 1 AND outcome_at > $2 - interval '1 hour' AND outcome_at <= $2),
                count(*) FILTER (WHERE created_at > $2 - interval '5 minutes' AND created_at <= $2),
                count(*) FILTER (WHERE created_at > $2 - interval '65 minutes' AND created_at <= $2 - interval '5 minutes'),
                max(created_at) FILTER (WHERE created_at <= $2)
           FROM decisions WHERE site_id = $1 AND created_at > $2 - interval '1 day'",
    )
    .bind(site)
    .bind(now)
    .fetch_one(pool)
    .await?;
    let spark: Vec<(f64, i64)> = sqlx::query_as(
        "SELECT extract(epoch FROM date_trunc('minute', created_at))::float8, count(*) FROM decisions
          WHERE site_id = $1 AND created_at > $2 - interval '30 minutes' AND created_at <= $2 GROUP BY 1 ORDER BY 1",
    )
    .bind(site)
    .bind(now)
    .fetch_all(pool)
    .await?;
    let start = (now - Duration::minutes(29)).timestamp() / 60 * 60;
    let sparkline: Vec<i64> =
        (0..30).map(|i| spark.iter().find(|(t, _)| *t as i64 == start + i * 60).map(|(_, n)| *n).unwrap_or(0)).collect();
    let stale = (recent5 == 0 && prev_hour > 0).then(|| {
        let mins = last_at.map(|t| (now - t).num_minutes()).unwrap_or(0);
        format!("No decisions from {origin} in {mins} min, though it was busy before. Is the SDK reachable?")
    });
    let today = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let r_today = results::site(pool, site, today, now).await?.expect("site exists");
    let r_week = results::site(pool, site, now - Duration::days(7), now).await?.expect("site exists");
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM variants v JOIN slots s ON s.id = v.slot_id WHERE s.site_id = $1 AND v.status = 'draft'")
        .bind(site)
        .fetch_one(pool)
        .await?;
    let paused: Option<(DateTime<Utc>, String)> = sqlx::query_as("SELECT paused_at, paused_by FROM sites WHERE id = $1 AND paused_at IS NOT NULL")
        .bind(site)
        .fetch_optional(pool)
        .await?;
    let slots: Vec<Value> = r_week
        .slots
        .iter()
        .map(|s| {
            let live: Vec<&results::VariantResult> = s.variants.iter().filter(|v| v.status == "approved").collect();
            let champion = live
                .iter()
                .max_by_key(|v| v.counts.n)
                .filter(|v| v.counts.n > 0)
                .or_else(|| live.iter().find(|v| v.is_baseline))
                .map(|v| v.key.clone());
            json!({
                "key": s.key, "metric": s.metric, "champion": champion, "lift_7d": s.lift,
                "treated": s.treated, "holdout": s.holdout, "variants": live.len(),
                // `index` is the variant's place among all of the slot's variants, so its colour
                // matches the workbench whatever else is paused or retired.
                "traffic": live.iter().map(|v| json!({
                    "key": v.key, "share": v.share,
                    "index": s.variants.iter().position(|x| x.id == v.id).unwrap_or(0),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({
        "site": site, "origin": origin, "holdout_fraction": r_today.holdout_fraction, "now": now,
        "decisions_last_minute": last_minute, "conversions_last_hour": last_hour_conv,
        "lift_today": r_today.lift, "treated_today": r_today.treated, "holdout_today": r_today.holdout,
        "sparkline": sparkline, "stale": stale, "pending_approvals": pending, "slots": slots,
        "paused": paused.map(|(at, by)| json!({ "at": at, "by": by })),
    }))
}

async fn overview_handler(State(state): State<Arc<AppState>>, Path(site): Path<String>) -> Result<Json<Value>, ApiError> {
    overview(&state, &site, Utc::now()).await.map(Json)
}

/// Server-sent events: the overview every few seconds. One-way is all a dashboard needs, and
/// SSE survives proxies that eat WebSockets.
async fn stream(State(state): State<Arc<AppState>>, Path(site): Path<String>) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    site_origin(&state, &site).await?;
    let every = state.stream_every;
    let s = futures_util::stream::unfold((state, site, true), move |(state, site, first)| async move {
        if !first {
            tokio::time::sleep(every).await;
        }
        let ev = match overview(&state, &site, Utc::now()).await {
            Ok(v) => Event::default().event("overview").data(v.to_string()),
            Err(_) => Event::default().event("error").data("overview failed"),
        };
        Some((Ok(ev), (state, site, false)))
    });
    Ok(Sse::new(s).keep_alive(axum::response::sse::KeepAlive::default()))
}

async fn site_results(State(state): State<Arc<AppState>>, Path(site): Path<String>) -> Result<Json<Value>, ApiError> {
    let now = Utc::now();
    let r = results::site(&state.pool, &site, results::ALL_TIME, now).await?.ok_or_else(|| ApiError::NotFound(format!("unknown site {site:?}")))?;
    let learnings: Vec<(i64, String, Option<String>, Option<String>, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT l.id, l.kind, s.key, v.key, l.sentence, l.created_at FROM learnings l
           LEFT JOIN slots s ON s.id = l.slot_id LEFT JOIN variants v ON v.id = l.variant_id
          WHERE l.site_id = $1 ORDER BY l.created_at DESC LIMIT 50",
    )
    .bind(&site)
    .fetch_all(&state.pool)
    .await?;
    let mut curves = serde_json::Map::new();
    for s in &r.slots {
        curves.insert(s.key.clone(), json!(results::curve(&state.pool, s.id, now).await?));
    }
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM decisions WHERE site_id = $1").bind(&site).fetch_one(&state.pool).await?;
    Ok(Json(json!({
        "results": r, "curves": curves, "decisions_total": total,
        "learnings": learnings.into_iter().map(|(id, kind, slot, variant, sentence, at)| json!({
            "id": id, "kind": kind, "slot": slot, "variant": variant, "sentence": sentence, "at": at })).collect::<Vec<_>>(),
    })))
}

async fn slot_detail(State(state): State<Arc<AppState>>, Path((site, slot)): Path<(String, String)>) -> Result<Json<Value>, ApiError> {
    let now = Utc::now();
    let row: Option<(i64, String, String, i32, i32, i64)> = sqlx::query_as(
        "SELECT s.id, s.selector, s.metric, s.attribution_window_secs, s.decision_ttl_secs, m.version
           FROM slots s JOIN models m ON m.slot_id = s.id WHERE s.site_id = $1 AND s.key = $2",
    )
    .bind(&site)
    .bind(&slot)
    .fetch_optional(&state.pool)
    .await?;
    let Some((slot_id, selector, metric, window, ttl, version)) = row else {
        return Err(ApiError::NotFound(format!("unknown slot {slot:?} on site {site:?}")));
    };
    let r = results::site(&state.pool, &site, results::ALL_TIME, now).await?.expect("site exists");
    let sr = r.slots.into_iter().find(|s| s.id == slot_id).expect("slot is in its site");
    let extra: Vec<(i64, Option<String>, String, Option<String>, Option<DateTime<Utc>>, Option<DateTime<Utc>>, Option<Value>, DateTime<Utc>)> =
        sqlx::query_as("SELECT id, payload, author, approved_by, approved_at, paused_at, pause_reason, created_at FROM variants WHERE slot_id = $1")
            .bind(slot_id)
            .fetch_all(&state.pool)
            .await?;
    let variants: Vec<Value> = sr
        .variants
        .iter()
        .map(|v| {
            let e = extra.iter().find(|e| e.0 == v.id);
            let mut j = serde_json::to_value(v).unwrap();
            if let Some((_, payload, author, by, at, paused_at, reason, created)) = e {
                let o = j.as_object_mut().unwrap();
                o.insert("payload".into(), json!(payload));
                o.insert("author".into(), json!(author));
                o.insert("approved_by".into(), json!(by));
                o.insert("approved_at".into(), json!(at));
                o.insert("paused_at".into(), json!(paused_at));
                o.insert("pause_reason".into(), json!(reason));
                o.insert("created_at".into(), json!(created));
            }
            j
        })
        .collect();
    let live: Vec<&results::VariantResult> = sr.variants.iter().filter(|v| v.status == "approved").collect();
    let champion = live.iter().max_by_key(|v| v.counts.n).filter(|v| v.counts.n > 0).or_else(|| live.iter().find(|v| v.is_baseline)).map(|v| v.key.clone());
    Ok(Json(json!({
        "site": site, "slot": slot, "id": slot_id, "selector": selector, "metric": metric,
        "attribution_window_secs": window, "decision_ttl_secs": ttl, "model_version": version,
        "treated": sr.treated, "holdout": sr.holdout, "lift": sr.lift, "champion": champion,
        "variants": variants, "curve": results::curve(&state.pool, slot_id, now).await?,
        "audiences": [],
    })))
}

#[derive(Deserialize)]
struct DecisionFilter {
    slot: Option<String>,
    variant: Option<String>,
    outcome: Option<String>,
    limit: Option<i64>,
}

async fn decisions(State(state): State<Arc<AppState>>, Path(site): Path<String>, Query(f): Query<DecisionFilter>) -> Result<Json<Value>, ApiError> {
    site_origin(&state, &site).await?;
    let rows = decision_list(&state.pool, &site, f.slot.as_deref(), f.variant.as_deref(), f.outcome.as_deref(), f.limit.unwrap_or(500).clamp(1, 5000)).await?;
    Ok(Json(json!(rows)))
}

/// Decisions on a site, newest first, filtered by slot, variant and outcome.
pub async fn decision_list(
    pool: &sqlx::PgPool,
    site: &str,
    slot: Option<&str>,
    variant: Option<&str>,
    outcome: Option<&str>,
    limit: i64,
) -> Result<Vec<Value>, ApiError> {
    let outcome = outcome.unwrap_or("all");
    let outcome_sql = match outcome {
        "all" => "true",
        "converted" => "d.outcome = 1",
        "missed" => "d.outcome = 0",
        "open" => "d.exposed_at IS NOT NULL AND d.settled_at IS NULL",
        "unexposed" => "d.exposed_at IS NULL",
        "holdout" => "d.in_holdout",
        other => return Err(ApiError::BadRequest(format!("unknown outcome filter {other:?}"))),
    };
    let rows: Vec<(Uuid, DateTime<Utc>, String, String, String, bool, Option<DateTime<Utc>>, Option<i16>, Option<DateTime<Utc>>, Value)> =
        sqlx::query_as(&format!(
            "SELECT d.id, d.created_at, s.key, v.key, d.subject, d.in_holdout, d.exposed_at, d.outcome, d.settled_at, d.features
               FROM decisions d JOIN slots s ON s.id = d.slot_id JOIN variants v ON v.id = d.variant_id
              WHERE d.site_id = $1 AND ($2::text IS NULL OR s.key = $2) AND ($3::text IS NULL OR v.key = $3) AND {outcome_sql}
              ORDER BY d.created_at DESC LIMIT $4"
        ))
        .bind(site)
        .bind(slot)
        .bind(variant)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(id, at, slot, variant, subject, held, exposed, outcome, settled, features)| json!({
            "id": id, "created_at": at, "slot": slot, "variant": variant, "subject": subject, "in_holdout": held,
            "exposed": exposed.is_some(), "outcome": outcome, "settled": settled.is_some(), "features": features,
        }))
        .collect())
}

async fn decision(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>) -> Result<Json<Value>, ApiError> {
    let d = explain::detail(&state.pool, id, Utc::now()).await?.ok_or_else(|| ApiError::NotFound(format!("unknown decision {id}")))?;
    Ok(Json(json!(d)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewDraft {
    key: String,
    html: String,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    author_name: Option<String>,
}

pub const MAX_PAYLOAD: usize = 64 * 1024;

async fn create_draft(
    State(state): State<Arc<AppState>>,
    Path((site, slot)): Path<(String, String)>,
    Json(b): Json<NewDraft>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !crate::site_file::is_slug(&b.key) {
        return Err(ApiError::BadRequest("key must be a lowercase slug (a-z, 0-9, - and _)".into()));
    }
    if b.html.trim().is_empty() || b.html.len() > MAX_PAYLOAD {
        return Err(ApiError::BadRequest(format!("html must be 1 to {MAX_PAYLOAD} bytes")));
    }
    let slot_id: i64 = sqlx::query_scalar("SELECT id FROM slots WHERE site_id = $1 AND key = $2")
        .bind(&site)
        .bind(&slot)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("unknown slot {slot:?} on site {site:?}")))?;
    let mut tx = state.pool.begin().await?;
    let id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO variants (slot_id, key, payload, author, status) VALUES ($1, $2, $3, 'human', 'draft')
         ON CONFLICT (slot_id, key) DO NOTHING RETURNING id",
    )
    .bind(slot_id)
    .bind(&b.key)
    .bind(&b.html)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(id) = id else {
        return Err(ApiError::Conflict(format!("slot {slot:?} already has a variant called {:?}", b.key)));
    };
    if let Some(note) = b.note.as_deref().filter(|n| !n.trim().is_empty()) {
        let actor = b.author_name.clone().filter(|a| !a.trim().is_empty()).unwrap_or_else(|| "dashboard".into());
        sqlx::query("INSERT INTO approvals (variant_id, action, actor, comment) VALUES ($1, 'comment', $2, $3)")
            .bind(id)
            .bind(actor.chars().take(128).collect::<String>())
            .bind(note)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "status": "draft" }))))
}

async fn approvals(State(state): State<Arc<AppState>>, Path(site): Path<String>) -> Result<Json<Value>, ApiError> {
    site_origin(&state, &site).await?;
    let forbidden: Vec<String> = sqlx::query_scalar("SELECT forbidden_claims FROM sites WHERE id = $1").bind(&site).fetch_one(&state.pool).await?;
    #[allow(clippy::type_complexity)]
    let drafts: Vec<(i64, String, String, Option<String>, String, DateTime<Utc>, i64, Option<Value>)> = sqlx::query_as(
        "SELECT v.id, s.key, v.key, v.payload, v.author, v.created_at, s.id,
                CASE WHEN c.id IS NOT NULL THEN jsonb_build_object('name', c.agent, 'justification', c.justification,
                     'on_behalf_of', c.on_behalf_of, 'token', k.name, 'call_id', c.call_id, 'at', c.created_at,
                     'audit_id', c.id, 'tool', c.tool) END
           FROM variants v JOIN slots s ON s.id = v.slot_id
           LEFT JOIN mcp_calls c ON c.id = v.mcp_call_id LEFT JOIN mcp_tokens k ON k.id = c.token_id
          WHERE s.site_id = $1 AND v.status = 'draft' ORDER BY v.created_at, v.id",
    )
    .bind(&site)
    .fetch_all(&state.pool)
    .await?;
    let now = Utc::now();
    let r = results::site(&state.pool, &site, now - Duration::days(7), now).await?.expect("site exists");
    let mut cards = vec![];
    for (id, slot, key, payload, author, created, slot_id, agent) in drafts {
        let sr = r.slots.iter().find(|s| s.id == slot_id);
        let live: Vec<&results::VariantResult> = sr.map(|s| s.variants.iter().filter(|v| v.status == "approved").collect()).unwrap_or_default();
        let champ = live.iter().max_by_key(|v| v.counts.n).filter(|v| v.counts.n > 0).or_else(|| live.iter().find(|v| v.is_baseline));
        let champion = match champ {
            Some(c) => {
                let p: Option<String> = sqlx::query_scalar("SELECT payload FROM variants WHERE id = $1").bind(c.id).fetch_one(&state.pool).await?;
                json!({ "key": c.key, "payload": p, "is_baseline": c.is_baseline })
            }
            None => Value::Null,
        };
        let comments: Vec<(String, String, Option<String>, DateTime<Utc>)> =
            sqlx::query_as("SELECT action, actor, comment, created_at FROM approvals WHERE variant_id = $1 ORDER BY created_at")
                .bind(id)
                .fetch_all(&state.pool)
                .await?;
        cards.push(json!({
            "id": id, "slot": slot, "key": key, "html": payload, "author": author, "created_at": created, "agent": agent,
            "champion": champion, "checks": checks::run(payload.as_deref().unwrap_or(""), &forbidden),
            "comments": comments.into_iter().map(|(a, by, c, at)| json!({ "action": a, "actor": by, "comment": c, "at": at })).collect::<Vec<_>>(),
        }));
    }
    let recent: Vec<(String, String, String, String, Option<String>, Option<f32>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT a.action, a.actor, s.key, v.key, a.comment, a.traffic_share, a.created_at
           FROM approvals a JOIN variants v ON v.id = a.variant_id JOIN slots s ON s.id = v.slot_id
          WHERE s.site_id = $1 AND a.action IN ('approve', 'reject') ORDER BY a.created_at DESC LIMIT 20",
    )
    .bind(&site)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({
        "drafts": cards, "max_share": 0.5, "default_share": 0.1,
        "recent": recent.into_iter().map(|(a, by, slot, key, c, share, at)| json!({
            "action": a, "actor": by, "slot": slot, "variant": key, "comment": c, "traffic_share": share, "at": at })).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Approve {
    approver: String,
    traffic_share: f32,
    #[serde(default)]
    comment: Option<String>,
}

fn actor(name: &str) -> Result<String, ApiError> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 128 {
        return Err(ApiError::BadRequest("name the person approving (1-128 characters)".into()));
    }
    Ok(n.to_string())
}

async fn approve(State(state): State<Arc<AppState>>, Path(id): Path<i64>, Json(b): Json<Approve>) -> Result<Response, ApiError> {
    let approver = actor(&b.approver)?;
    if !(b.traffic_share > 0.0 && b.traffic_share <= 0.5) {
        return Err(ApiError::BadRequest("the planned traffic share must be above 0 and at most 50%".into()));
    }
    let mut tx = state.pool.begin().await?;
    let row: Option<(i64, String, Option<String>, Vec<String>)> = sqlx::query_as(
        "SELECT v.slot_id, v.status, v.payload, t.forbidden_claims FROM variants v JOIN slots s ON s.id = v.slot_id JOIN sites t ON t.id = s.site_id
          WHERE v.id = $1 FOR UPDATE OF v",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((slot_id, status, payload, forbidden)) = row else {
        return Err(ApiError::NotFound(format!("unknown variant {id}")));
    };
    if status != "draft" {
        return Err(ApiError::Conflict(format!("variant {id} is {status}; only a draft can be approved")));
    }
    let report = checks::run(payload.as_deref().unwrap_or(""), &forbidden);
    if checks::blocks(&report) {
        return Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": "a constraint check failed", "checks": report }))).into_response());
    }
    sqlx::query(
        "UPDATE variants SET status = 'approved', approved_by = $2, approved_at = now(), traffic_share = $3,
                approved_model_version = (SELECT version FROM models WHERE slot_id = $4)
          WHERE id = $1",
    )
    .bind(id)
    .bind(&approver)
    .bind(b.traffic_share)
    .bind(slot_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO approvals (variant_id, action, actor, comment, traffic_share, checks) VALUES ($1, 'approve', $2, $3, $4, $5)")
        .bind(id)
        .bind(&approver)
        .bind(&b.comment)
        .bind(b.traffic_share)
        .bind(json!(report))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": "approved", "traffic_share": b.traffic_share })).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Remark {
    actor: String,
    comment: String,
}

async fn reject(State(state): State<Arc<AppState>>, Path(id): Path<i64>, Json(b): Json<Remark>) -> Result<Json<Value>, ApiError> {
    let who = actor(&b.actor)?;
    if b.comment.trim().is_empty() {
        return Err(ApiError::BadRequest("say why it is rejected".into()));
    }
    let mut tx = state.pool.begin().await?;
    let n = sqlx::query("UPDATE variants SET status = 'rejected' WHERE id = $1 AND status = 'draft'").bind(id).execute(&mut *tx).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::Conflict(format!("variant {id} is not a draft")));
    }
    sqlx::query("INSERT INTO approvals (variant_id, action, actor, comment) VALUES ($1, 'reject', $2, $3)")
        .bind(id)
        .bind(&who)
        .bind(&b.comment)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": "rejected" })))
}

async fn comment(State(state): State<Arc<AppState>>, Path(id): Path<i64>, Json(b): Json<Remark>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let who = actor(&b.actor)?;
    if b.comment.trim().is_empty() {
        return Err(ApiError::BadRequest("the comment is empty".into()));
    }
    let r = sqlx::query("INSERT INTO approvals (variant_id, action, actor, comment) SELECT id, 'comment', $2, $3 FROM variants WHERE id = $1")
        .bind(id)
        .bind(&who)
        .bind(&b.comment)
        .execute(&state.pool)
        .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("unknown variant {id}")));
    }
    Ok((StatusCode::CREATED, Json(json!({ "ok": true }))))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rollback {
    version: i64,
    actor: String,
}

/// Force-restore a model version, as a new version. The history keeps everything.
async fn rollback(State(state): State<Arc<AppState>>, Path(slot): Path<i64>, Json(b): Json<Rollback>) -> Result<Json<Value>, ApiError> {
    let who = actor(&b.actor)?;
    let mut tx = state.pool.begin().await?;
    let locked: Option<i64> =
        sqlx::query_scalar("SELECT version FROM models WHERE slot_id = $1 FOR UPDATE").bind(slot).fetch_optional(&mut *tx).await?;
    if locked.is_none() {
        return Err(ApiError::NotFound(format!("unknown slot id {slot}")));
    }
    let snapshot: Option<Value> = sqlx::query_scalar("SELECT state FROM model_versions WHERE slot_id = $1 AND version = $2")
        .bind(slot)
        .bind(b.version)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(snapshot) = snapshot else {
        return Err(ApiError::NotFound(format!("slot {slot} has no model version {}", b.version)));
    };
    let v = worker::write_model(&mut tx, slot, &snapshot, &format!("manual rollback to version {} by {who}", b.version)).await?;
    tx.commit().await?;
    Ok(Json(json!({ "slot": slot, "restored": b.version, "version": v })))
}

async fn versions(State(state): State<Arc<AppState>>, Path(slot): Path<i64>) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(i64, String, DateTime<Utc>)> =
        sqlx::query_as("SELECT version, reason, created_at FROM model_versions WHERE slot_id = $1 ORDER BY version DESC LIMIT 200")
            .bind(slot)
            .fetch_all(&state.pool)
            .await?;
    Ok(Json(json!(rows.into_iter().map(|(v, r, at)| json!({ "version": v, "reason": r, "at": at })).collect::<Vec<_>>())))
}

async fn install(State(state): State<Arc<AppState>>, Path(site): Path<String>) -> Result<Json<Value>, ApiError> {
    let origin = site_origin(&state, &site).await?;
    let steps = probes::all(&state.pool, &state.http, &site, &origin).await?;
    Ok(Json(json!({ "site": site, "origin": origin, "steps": steps })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewMcpToken {
    name: String,
}

/// A per-site MCP token for a developer's agent. Shown once; only its hash is kept.
async fn issue_mcp_token(State(state): State<Arc<AppState>>, Path(site): Path<String>, Json(b): Json<NewMcpToken>) -> Result<(StatusCode, Json<Value>), ApiError> {
    site_origin(&state, &site).await?;
    let token = crate::mcp::issue_token(&state.pool, &site, &b.name).await.map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok((StatusCode::CREATED, Json(json!({ "site": site, "name": b.name.trim(), "token": token, "url": "/mcp" }))))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pause {
    actor: String,
    paused: bool,
}

/// The kill switch: pause every slot on the site (each serves only its baseline) or resume.
async fn pause_site(State(state): State<Arc<AppState>>, Path(site): Path<String>, Json(b): Json<Pause>) -> Result<Json<Value>, ApiError> {
    site_origin(&state, &site).await?;
    let who = actor(&b.actor)?;
    let row: (Option<DateTime<Utc>>, Option<String>) = if b.paused {
        sqlx::query_as("UPDATE sites SET paused_at = coalesce(paused_at, now()), paused_by = coalesce(paused_by, $2) WHERE id = $1 RETURNING paused_at, paused_by")
    } else {
        sqlx::query_as("UPDATE sites SET paused_at = NULL, paused_by = NULL WHERE id = $1 RETURNING paused_at, paused_by")
    }
    .bind(&site)
    .bind(&who)
    .fetch_one(&state.pool)
    .await?;
    tracing::info!(site, actor = who, paused = b.paused, "kill switch");
    Ok(Json(json!({ "site": site, "paused": row.0.map(|at| json!({ "at": at, "by": row.1 })) })))
}

const CALL_COLUMNS: &str = "c.id, c.tool, c.agent, c.on_behalf_of, c.justification, c.outcome, c.error, c.call_id, k.name, c.created_at";
type CallRow = (i64, String, String, Option<String>, Option<String>, String, Option<String>, Value, String, DateTime<Utc>);

fn call_json(r: CallRow) -> Value {
    let (id, tool, agent, on_behalf_of, justification, outcome, error, call_id, token, at) = r;
    json!({ "id": id, "tool": tool, "agent": agent, "on_behalf_of": on_behalf_of, "justification": justification,
            "outcome": outcome, "error": error, "call_id": call_id, "token": token, "at": at })
}

#[derive(Deserialize)]
struct CallsQuery {
    limit: Option<i64>,
}

/// The MCP tail for a site: its tokens (with when each was last used) and its newest tool calls.
async fn mcp_calls(State(state): State<Arc<AppState>>, Path(site): Path<String>, Query(q): Query<CallsQuery>) -> Result<Json<Value>, ApiError> {
    site_origin(&state, &site).await?;
    let limit = q.limit.unwrap_or(25).clamp(1, 200);
    let calls: Vec<CallRow> = sqlx::query_as(&format!(
        "SELECT {CALL_COLUMNS} FROM mcp_calls c JOIN mcp_tokens k ON k.id = c.token_id
          WHERE c.site_id = $1 ORDER BY c.created_at DESC, c.id DESC LIMIT $2"
    ))
    .bind(&site)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;
    let tokens: Vec<(String, DateTime<Utc>, Option<DateTime<Utc>>, Option<DateTime<Utc>>, i64)> = sqlx::query_as(
        "SELECT k.name, k.created_at, k.revoked_at, max(c.created_at), count(c.id) FROM mcp_tokens k
           LEFT JOIN mcp_calls c ON c.token_id = k.id WHERE k.site_id = $1 GROUP BY k.id ORDER BY k.created_at",
    )
    .bind(&site)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({
        "site": site, "endpoint": "/mcp",
        "tokens": tokens.into_iter().map(|(name, created, revoked, last, n)| json!({
            "name": name, "created_at": created, "revoked_at": revoked, "last_used": last, "calls": n })).collect::<Vec<_>>(),
        "calls": calls.into_iter().map(call_json).collect::<Vec<_>>(),
    })))
}

/// One tool call's audit record: who called what, why, with which arguments, and what it wrote.
async fn mcp_call(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    let row: Option<(CallRow, String, Value)> = sqlx::query_as::<_, (i64, String, String, Option<String>, Option<String>, String, Option<String>, Value, String, DateTime<Utc>, String, Value)>(&format!(
        "SELECT {CALL_COLUMNS}, c.site_id, c.arguments FROM mcp_calls c JOIN mcp_tokens k ON k.id = c.token_id WHERE c.id = $1"
    ))
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .map(|r| ((r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8, r.9), r.10, r.11));
    let Some((call, site, arguments)) = row else {
        return Err(ApiError::NotFound(format!("no MCP call {id}")));
    };
    let wrote: Vec<(i64, String, String, String)> = sqlx::query_as(
        "SELECT v.id, s.key, v.key, v.status FROM variants v JOIN slots s ON s.id = v.slot_id WHERE v.mcp_call_id = $1 ORDER BY v.id",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let mut v = call_json(call);
    let o = v.as_object_mut().expect("object");
    o.insert("site".into(), json!(site));
    o.insert("arguments".into(), arguments);
    o.insert("variants".into(), json!(wrote.into_iter().map(|(id, slot, key, status)| json!({ "id": id, "slot": slot, "key": key, "status": status })).collect::<Vec<_>>()));
    Ok(Json(v))
}
