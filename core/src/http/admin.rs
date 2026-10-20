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
use std::collections::HashMap;
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
        .route(
            "/sites/{site}/assets",
            // One byte over the cap gets through, so the upload's own check answers 413 with a reason.
            get(list_assets).post(upload_asset).layer(axum::extract::DefaultBodyLimit::max(state.assets.max_bytes + 1)),
        )
        .route("/mcp-calls/{id}", get(mcp_call))
        .route("/decisions/{id}", get(decision))
        .route("/variants/{id}/approve", post(approve))
        .route("/variants/{id}/reject", post(reject))
        .route("/variants/{id}/comment", post(comment))
        .route("/variants/{id}/audience", post(set_audience))
        .route("/retire-requests/{id}/confirm", post(confirm_retire))
        .route("/retire-requests/{id}/decline", post(decline_retire))
        .route("/variants/{id}/traffic-share", post(edit_share))
        .route("/share-requests/{id}/confirm", post(confirm_share))
        .route("/share-requests/{id}/decline", post(decline_share))
        .route("/models/{slot}/rollback", post(rollback))
        .route("/models/{slot}/versions", get(versions))
        .merge(super::privacy::routes())
        .merge(super::sim::routes())
        .merge(super::webhooks::admin_routes())
        .merge(crate::insights::api::routes())
        .merge(super::hooks::router())
        .merge(super::plugins::routes())
        .merge(super::experiments::routes())
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadQuery {
    #[serde(default)]
    name: Option<String>,
}

/// `POST /api/sites/{site}/assets?name=banner.png`: the raw bytes as the body. The declared
/// content type and the name are not trusted; [`crate::assets::upload`] reads the bytes.
async fn upload_asset(
    State(state): State<Arc<AppState>>,
    Path(site): Path<String>,
    Query(q): Query<UploadQuery>,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let v = crate::assets::upload(&state.pool, &state.assets, &site, &body, q.name.as_deref(), "dashboard", None).await?;
    Ok((StatusCode::CREATED, Json(v)))
}

async fn list_assets(State(state): State<Arc<AppState>>, Path(site): Path<String>) -> Result<Json<Value>, ApiError> {
    site_origin(&state, &site).await?;
    Ok(Json(crate::assets::list(&state.pool, &site).await?))
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
        // One line per installed de-anonymiser plugin; the dashboard shows them as a banner.
        "warnings": crate::plugins::warnings(&state.pool, site).await?,
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

/// Where a variant's current share came from: "file" (the site file), "edited" (a person changed
/// it after approval) or "approval" (set when a person approved it), with who; None without one.
fn share_source(share: Option<f32>, file_share: Option<f32>, last: Option<String>, approver: Option<String>) -> (Option<&'static str>, Option<String>) {
    if share.is_none() {
        return (None, None);
    }
    match last {
        Some(a) if a == crate::site_file::APPROVER => (Some("file"), None),
        Some(a) => (Some("edited"), Some(a)),
        None if file_share.is_some() => (Some("file"), None),
        None => (Some("approval"), approver),
    }
}

async fn slot_detail(State(state): State<Arc<AppState>>, Path((site, slot)): Path<(String, String)>) -> Result<Json<Value>, ApiError> {
    let now = Utc::now();
    let row: Option<(i64, String, String, i32, i32, i64, String, Option<Value>)> = sqlx::query_as(
        "SELECT s.id, s.selector, s.metric, s.attribution_window_secs, s.decision_ttl_secs, m.version, s.kind, s.copy
           FROM slots s JOIN models m ON m.slot_id = s.id WHERE s.site_id = $1 AND s.key = $2",
    )
    .bind(&site)
    .bind(&slot)
    .fetch_optional(&state.pool)
    .await?;
    let Some((slot_id, selector, metric, window, ttl, version, kind, copy_spec)) = row else {
        return Err(ApiError::NotFound(format!("unknown slot {slot:?} on site {site:?}")));
    };
    let r = results::site(&state.pool, &site, results::ALL_TIME, now).await?.expect("site exists");
    let sr = r.slots.into_iter().find(|s| s.id == slot_id).expect("slot is in its site");
    let extra: Vec<(i64, Option<String>, String, Option<String>, Option<DateTime<Utc>>, Option<DateTime<Utc>>, Option<Value>, DateTime<Utc>)> =
        sqlx::query_as("SELECT id, payload, author, approved_by, approved_at, paused_at, pause_reason, created_at FROM variants WHERE slot_id = $1")
            .bind(slot_id)
            .fetch_all(&state.pool)
            .await?;
    // S9: where each variant's share came from. The latest 'share' row says who last set it (the
    // site file audits its own changes as cli:apply); with none, a file-stated share is the file's
    // and any other was set by the person who approved it.
    #[allow(clippy::type_complexity)]
    let shares: HashMap<i64, (Option<f32>, Option<String>)> = sqlx::query_as::<_, (i64, Option<f32>, Option<String>)>(
        "SELECT v.id, v.file_share,
                (SELECT a.actor FROM approvals a WHERE a.variant_id = v.id AND a.action = 'share' ORDER BY a.id DESC LIMIT 1)
           FROM variants v WHERE v.slot_id = $1",
    )
    .bind(slot_id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(id, f, last)| (id, (f, last)))
    .collect();
    // Which audience gates each variant, and the site's promoted audiences an owner can pick from.
    let gates: HashMap<i64, i64> =
        sqlx::query_as::<_, (i64, i64)>("SELECT id, audience_id FROM variants WHERE slot_id = $1 AND audience_id IS NOT NULL")
            .bind(slot_id)
            .fetch_all(&state.pool)
            .await?
            .into_iter()
            .collect();
    let audiences: Vec<Value> = sqlx::query_as::<_, (i64, String, Value)>(
        "SELECT id, name, predicate FROM segments WHERE site_id = $1 AND status = 'audience' ORDER BY id",
    )
    .bind(&site)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(id, name, predicate)| json!({ "id": id, "name": name, "predicate": predicate }))
    .collect();
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
            let (file_share, last) = shares.get(&v.id).cloned().unwrap_or_default();
            let approver = e.and_then(|e| e.3.clone());
            let (source, by) = share_source(v.traffic_share, file_share, last, approver);
            let o = j.as_object_mut().unwrap();
            o.insert("file_share".into(), json!(file_share));
            o.insert("share_source".into(), json!(source));
            o.insert("share_set_by".into(), json!(by));
            j.as_object_mut().unwrap().insert("audience".into(), json!(gates.get(&v.id)));
            j
        })
        .collect();
    let live: Vec<&results::VariantResult> = sr.variants.iter().filter(|v| v.status == "approved").collect();
    let champion = live.iter().max_by_key(|v| v.counts.n).filter(|v| v.counts.n > 0).or_else(|| live.iter().find(|v| v.is_baseline)).map(|v| v.key.clone());
    Ok(Json(json!({
        "site": site, "slot": slot, "id": slot_id, "kind": kind, "selector": selector, "metric": metric,
        // A copy slot's declared fields (F4), so a card can label each one; null for other kinds.
        "fields": copy_spec.as_ref().map(|s| s["fields"].clone()),
        "attribution_window_secs": window, "decision_ttl_secs": ttl, "model_version": version,
        "treated": sr.treated, "holdout": sr.holdout, "lift": sr.lift, "champion": champion,
        // Decisions still in their window: not in the counts above yet (S8).
        "pending": sr.pending,
        "variants": variants, "curve": results::curve(&state.pool, slot_id, now).await?,
        "audiences": audiences, "items": sr.items,
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
    let (slot_id, spec): (i64, Option<Value>) = sqlx::query_as("SELECT id, copy FROM slots WHERE site_id = $1 AND key = $2")
        .bind(&site)
        .bind(&slot)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("unknown slot {slot:?} on site {site:?}")))?;
    // A copy slot's draft is its fields as a JSON object (F4), checked as an agent's would be.
    if let Some(spec) = &spec {
        crate::copy::check_stored(spec, Some(&b.html)).map_err(|e| ApiError::BadRequest(format!("copy slot: {e}")))?;
        let text = crate::copy::read_text(Some(&b.html)).map_err(ApiError::BadRequest)?;
        crate::assets::check_refs(&state.pool, &site, &crate::copy::asset_refs(&text)).await?;
    }
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
    let (forbidden, facts): (Vec<String>, Option<String>) =
        sqlx::query_as("SELECT forbidden_claims, facts FROM sites WHERE id = $1").bind(&site).fetch_one(&state.pool).await?;
    #[allow(clippy::type_complexity)]
    let drafts: Vec<(i64, String, String, Option<String>, String, DateTime<Utc>, i64, Option<Value>, bool, Option<Value>)> = sqlx::query_as(
        "SELECT v.id, s.key, v.key, v.payload, v.author, v.created_at, s.id,
                CASE WHEN c.id IS NOT NULL THEN jsonb_build_object('name', c.agent, 'justification', c.justification,
                     'on_behalf_of', c.on_behalf_of, 'token', k.name, 'call_id', c.call_id, 'at', c.created_at,
                     'audit_id', c.id, 'tool', c.tool) END, v.is_baseline, s.copy
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
    for (id, slot, key, payload, author, created, slot_id, agent, activates_slot, copy_spec) in drafts {
        let sr = r.slots.iter().find(|s| s.id == slot_id);
        let live: Vec<&results::VariantResult> = sr.map(|s| s.variants.iter().filter(|v| v.status == "approved").collect()).unwrap_or_default();
        let champ = live.iter().max_by_key(|v| v.counts.n).filter(|v| v.counts.n > 0).or_else(|| live.iter().find(|v| v.is_baseline));
        let champion = match champ {
            Some(c) => {
                let p: Option<String> = sqlx::query_scalar("SELECT payload FROM variants WHERE id = $1").bind(c.id).fetch_one(&state.pool).await?;
                let copy = copy_spec.as_ref().and_then(|_| crate::copy::parse(p.as_deref()));
                json!({ "key": c.key, "payload": p, "is_baseline": c.is_baseline, "copy": copy })
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
            // Approving a draft baseline turns on a slot an agent created.
            "activates_slot": activates_slot,
            // A copy slot (F4): its declared fields and this draft's text, parsed.
            "fields": copy_spec.as_ref().map(|s| s["fields"].clone()),
            "copy": copy_spec.as_ref().and_then(|_| crate::copy::parse(payload.as_deref())),
            "champion": champion, "checks": checks::run(&if copy_spec.is_some() { crate::copy::plain(payload.as_deref()) } else { payload.clone().unwrap_or_default() }, &checks::Context { forbidden_claims: &forbidden, author: &author, facts: facts.as_deref() }),
            "comments": comments.into_iter().map(|(a, by, c, at)| json!({ "action": a, "actor": by, "comment": c, "at": at })).collect::<Vec<_>>(),
        }));
    }
    let recent: Vec<(String, String, String, String, Option<String>, Option<f32>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT a.action, a.actor, s.key, v.key, a.comment, a.traffic_share, a.created_at
           FROM approvals a JOIN variants v ON v.id = a.variant_id JOIN slots s ON s.id = v.slot_id
          WHERE s.site_id = $1 AND a.action IN ('approve', 'reject', 'retire', 'keep', 'share') ORDER BY a.created_at DESC LIMIT 20",
    )
    .bind(&site)
    .fetch_all(&state.pool)
    .await?;
    // Retirements an agent proposed; each waits for a person (confirm or decline).
    #[allow(clippy::type_complexity)]
    let retires: Vec<(i64, String, String, i64, String, String, DateTime<Utc>, Option<Value>)> = sqlx::query_as(
        "SELECT r.id, s.key, v.key, v.id, r.agent, r.reason, r.created_at,
                CASE WHEN c.id IS NOT NULL THEN jsonb_build_object('name', c.agent, 'justification', c.justification,
                     'on_behalf_of', c.on_behalf_of, 'token', k.name, 'audit_id', c.id, 'tool', c.tool) END
           FROM retire_requests r JOIN variants v ON v.id = r.variant_id JOIN slots s ON s.id = v.slot_id
           LEFT JOIN mcp_calls c ON c.id = r.mcp_call_id LEFT JOIN mcp_tokens k ON k.id = c.token_id
          WHERE s.site_id = $1 AND r.status = 'pending' ORDER BY r.created_at, r.id",
    )
    .bind(&site)
    .fetch_all(&state.pool)
    .await?;
    let retirements: Vec<Value> = retires
        .into_iter()
        .map(|(id, slot, key, vid, agent, reason, at, call)| {
            let mut a = call.unwrap_or_else(|| json!({ "name": agent }));
            a["name"] = json!(agent);
            json!({ "id": id, "slot": slot, "variant": key, "variant_id": vid, "reason": reason, "at": at, "agent": a })
        })
        .collect();
    // Share changes an agent proposed (G2); each waits for a person.
    #[allow(clippy::type_complexity)]
    let shares: Vec<(i64, String, String, i64, String, String, f32, Option<f32>, DateTime<Utc>, Option<Value>)> = sqlx::query_as(
        "SELECT r.id, s.key, v.key, v.id, r.agent, r.reason, r.traffic_share, v.traffic_share, r.created_at,
                CASE WHEN c.id IS NOT NULL THEN jsonb_build_object('name', c.agent, 'justification', c.justification,
                     'on_behalf_of', c.on_behalf_of, 'token', k.name, 'audit_id', c.id, 'tool', c.tool) END
           FROM share_requests r JOIN variants v ON v.id = r.variant_id JOIN slots s ON s.id = v.slot_id
           LEFT JOIN mcp_calls c ON c.id = r.mcp_call_id LEFT JOIN mcp_tokens k ON k.id = c.token_id
          WHERE s.site_id = $1 AND r.status = 'pending' ORDER BY r.created_at, r.id",
    )
    .bind(&site)
    .fetch_all(&state.pool)
    .await?;
    let share_requests: Vec<Value> = shares
        .into_iter()
        .map(|(id, slot, key, vid, agent, reason, share, current, at, call)| {
            let mut a = call.unwrap_or_else(|| json!({ "name": agent }));
            a["name"] = json!(agent);
            json!({ "id": id, "slot": slot, "variant": key, "variant_id": vid, "reason": reason, "traffic_share": share,
                    "current": current, "at": at, "agent": a })
        })
        .collect();
    Ok(Json(json!({
        "drafts": cards, "retirements": retirements, "share_requests": share_requests,
        "plugin_requests": crate::plugins::open_requests(&state.pool, &site).await?, "max_share": 0.5, "default_share": 0.1,
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

/// The cap a traffic share lives under, at approval and on every later edit (and in the schema).
pub(crate) fn check_share(share: f32) -> Result<(), ApiError> {
    if !(share > 0.0 && share <= 0.5) {
        return Err(ApiError::BadRequest("a traffic share must be above 0 and at most 50%".into()));
    }
    Ok(())
}

pub(crate) fn actor(name: &str) -> Result<String, ApiError> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 128 {
        return Err(ApiError::BadRequest("name the person approving (1-128 characters)".into()));
    }
    Ok(n.to_string())
}

async fn approve(State(state): State<Arc<AppState>>, Path(id): Path<i64>, Json(b): Json<Approve>) -> Result<Response, ApiError> {
    let approver = actor(&b.approver)?;
    check_share(b.traffic_share)?;
    let mut tx = state.pool.begin().await?;
    #[allow(clippy::type_complexity)]
    let row: Option<(i64, String, Option<String>, Vec<String>, String, Option<String>, bool, Option<Value>)> = sqlx::query_as(
        "SELECT v.slot_id, v.status, v.payload, t.forbidden_claims, v.author, t.facts, v.is_baseline, s.copy FROM variants v JOIN slots s ON s.id = v.slot_id JOIN sites t ON t.id = s.site_id
          WHERE v.id = $1 FOR UPDATE OF v",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((slot_id, status, payload, forbidden, author, facts, is_baseline, copy_spec)) = row else {
        return Err(ApiError::NotFound(format!("unknown variant {id}")));
    };
    if status != "draft" {
        return Err(ApiError::Conflict(format!("variant {id} is {status}; only a draft can be approved")));
    }
    // A slot an agent created stays dark until its baseline is approved; its variants wait.
    if !is_baseline {
        let dark: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM variants WHERE slot_id = $1 AND is_baseline AND status = 'draft')")
            .bind(slot_id)
            .fetch_one(&mut *tx)
            .await?;
        if dark {
            return Err(ApiError::Conflict("this slot is still a draft: approve its baseline first, which activates the slot".into()));
        }
    }
    // A draft baseline belongs to a slot an agent created (`create_slot`). It is the page's own
    // content, so there is no copy to check and no share to cap: approving it activates the slot.
    // A copy draft must still fit the slot's fields as the site file now declares them.
    if let (false, Some(spec)) = (is_baseline, &copy_spec)
        && let Err(e) = crate::copy::check_stored(spec, payload.as_deref())
    {
        return Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": format!("the copy no longer fits this slot: {e}") }))).into_response());
    }
    // An image field must still name one of this slot's site's uploads (S11).
    if let (false, Some(_)) = (is_baseline, &copy_spec) {
        let site: String = sqlx::query_scalar("SELECT site_id FROM slots WHERE id = $1").bind(slot_id).fetch_one(&mut *tx).await?;
        let refs = crate::copy::read_text(payload.as_deref()).map(|t| crate::copy::asset_refs(&t)).unwrap_or_default();
        if let Err(ApiError::BadRequest(e)) = crate::assets::check_refs(&state.pool, &site, &refs).await {
            return Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": e }))).into_response());
        }
    }
    let report = if is_baseline {
        vec![]
    } else {
        let text = if copy_spec.is_some() { crate::copy::plain(payload.as_deref()) } else { payload.clone().unwrap_or_default() };
        checks::run(&text, &checks::Context { forbidden_claims: &forbidden, author: &author, facts: facts.as_deref() })
    };
    let share = (!is_baseline).then_some(b.traffic_share);
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
    .bind(share)
    .bind(slot_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO approvals (variant_id, action, actor, comment, traffic_share, checks) VALUES ($1, 'approve', $2, $3, $4, $5)")
        .bind(id)
        .bind(&approver)
        .bind(&b.comment)
        .bind(share)
        .bind(json!(report))
        .execute(&mut *tx)
        .await?;
    let (site, slot_key, variant_key): (String, String, String) =
        sqlx::query_as("SELECT s.site_id, s.key, v.key FROM variants v JOIN slots s ON s.id = v.slot_id WHERE v.id = $1").bind(id).fetch_one(&mut *tx).await?;
    let data = json!({ "variant_id": id, "slot": slot_key, "variant": variant_key, "approved_by": approver, "traffic_share": share });
    crate::webhooks::enqueue(&mut *tx, &site, "variant.approved", data).await?;
    tx.commit().await?;
    crate::edge::changed(&state.pool, &state.edge, &site).await;
    Ok(Json(json!({ "id": id, "status": "approved", "traffic_share": share, "activated_slot": is_baseline })).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Remark {
    actor: String,
    comment: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetireDecision {
    actor: String,
    #[serde(default)]
    comment: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShareEdit {
    actor: String,
    traffic_share: f32,
    #[serde(default)]
    comment: Option<String>,
}

/// Set a live variant's share inside `tx` and record it: `(site, slot key, variant key, previous)`.
/// Only an approved or paused non-baseline variant has a share to change; a draft gets one when
/// it is approved, and the baseline is the page's own content.
async fn apply_share(tx: &mut sqlx::PgConnection, variant_id: i64, share: f32, who: &str, comment: Option<&str>) -> Result<(String, String, String, Option<f32>), ApiError> {
    check_share(share)?;
    let row: Option<(String, bool, Option<f32>, String, String, String)> = sqlx::query_as(
        "SELECT v.status, v.is_baseline, v.traffic_share, s.site_id, s.key, v.key FROM variants v JOIN slots s ON s.id = v.slot_id
          WHERE v.id = $1 FOR UPDATE OF v",
    )
    .bind(variant_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((status, is_baseline, previous, site, slot, key)) = row else {
        return Err(ApiError::NotFound(format!("unknown variant {variant_id}")));
    };
    if is_baseline {
        return Err(ApiError::BadRequest("the baseline is the page's own content and has no traffic share".into()));
    }
    if status != "approved" && status != "paused" {
        return Err(ApiError::Conflict(format!("variant {key:?} is {status}; only a live variant's share can change (a draft gets its share when approved)")));
    }
    sqlx::query("UPDATE variants SET traffic_share = $2 WHERE id = $1").bind(variant_id).bind(share).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO approvals (variant_id, action, actor, comment, traffic_share, previous_share) VALUES ($1, 'share', $2, $3, $4, $5)")
        .bind(variant_id)
        .bind(who)
        .bind(comment)
        .bind(share)
        .bind(previous)
        .execute(&mut *tx)
        .await?;
    let data = json!({ "variant_id": variant_id, "slot": slot, "variant": key, "traffic_share": share, "previous_share": previous, "changed_by": who });
    crate::webhooks::enqueue(&mut *tx, &site, "variant.share_changed", data).await?;
    Ok((site, slot, key, previous))
}

/// A person changes a live variant's traffic share. Decide reads it on the next request.
async fn edit_share(State(state): State<Arc<AppState>>, Path(id): Path<i64>, Json(b): Json<ShareEdit>) -> Result<Json<Value>, ApiError> {
    let who = actor(&b.actor)?;
    let comment = b.comment.as_deref().map(str::trim).filter(|c| !c.is_empty());
    let mut tx = state.pool.begin().await?;
    let (site, _, key, previous) = apply_share(&mut tx, id, b.traffic_share, &who, comment).await?;
    tx.commit().await?;
    crate::edge::changed(&state.pool, &state.edge, &site).await;
    Ok(Json(json!({ "id": id, "variant": key, "traffic_share": b.traffic_share, "previous_share": previous })))
}

/// Lock an open share proposal: `(variant_id, proposed share)`.
async fn open_share_request(tx: &mut sqlx::PgConnection, id: i64) -> Result<(i64, f32), ApiError> {
    let row: Option<(i64, f32, String)> = sqlx::query_as("SELECT variant_id, traffic_share, status FROM share_requests WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((variant_id, share, status)) = row else {
        return Err(ApiError::NotFound(format!("unknown share proposal {id}")));
    };
    if status != "pending" {
        return Err(ApiError::Conflict(format!("share proposal {id} is already {status}")));
    }
    Ok((variant_id, share))
}

/// A person confirms an agent's share proposal: applied and audited as their own edit.
async fn confirm_share(State(state): State<Arc<AppState>>, Path(id): Path<i64>, Json(b): Json<RetireDecision>) -> Result<Json<Value>, ApiError> {
    let who = actor(&b.actor)?;
    let comment = b.comment.as_deref().map(str::trim).filter(|c| !c.is_empty());
    let mut tx = state.pool.begin().await?;
    let (variant_id, share) = open_share_request(&mut tx, id).await?;
    let (site, _, key, previous) = apply_share(&mut tx, variant_id, share, &who, comment).await?;
    sqlx::query("UPDATE share_requests SET status = 'confirmed', decided_by = $2, decided_at = now() WHERE id = $1")
        .bind(id)
        .bind(&who)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    crate::edge::changed(&state.pool, &state.edge, &site).await;
    Ok(Json(json!({ "id": id, "status": "confirmed", "variant": key, "traffic_share": share, "previous_share": previous })))
}

/// A person declines it: the share stays, and any reason goes back to the agent on the thread.
async fn decline_share(State(state): State<Arc<AppState>>, Path(id): Path<i64>, Json(b): Json<RetireDecision>) -> Result<Json<Value>, ApiError> {
    let who = actor(&b.actor)?;
    let mut tx = state.pool.begin().await?;
    let (variant_id, _) = open_share_request(&mut tx, id).await?;
    sqlx::query("UPDATE share_requests SET status = 'declined', decided_by = $2, decided_at = now() WHERE id = $1")
        .bind(id)
        .bind(&who)
        .execute(&mut *tx)
        .await?;
    if let Some(c) = b.comment.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        sqlx::query("INSERT INTO approvals (variant_id, action, actor, comment) VALUES ($1, 'comment', $2, $3)")
            .bind(variant_id)
            .bind(&who)
            .bind(c)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": "declined" })))
}

/// Lock an open retirement proposal: `(variant_id, site, slot key, variant key)`.
async fn open_retire_request(tx: &mut sqlx::PgConnection, id: i64) -> Result<(i64, String, String, String), ApiError> {
    let row: Option<(i64, String, String, String, String)> = sqlx::query_as(
        "SELECT r.variant_id, r.status, s.site_id, s.key, v.key FROM retire_requests r
           JOIN variants v ON v.id = r.variant_id JOIN slots s ON s.id = v.slot_id
          WHERE r.id = $1 FOR UPDATE OF r",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((variant_id, status, site, slot, key)) = row else {
        return Err(ApiError::NotFound(format!("unknown retirement proposal {id}")));
    };
    if status != "pending" {
        return Err(ApiError::Conflict(format!("retirement proposal {id} is already {status}")));
    }
    Ok((variant_id, site, slot, key))
}

/// A person confirms an agent's retirement proposal: the variant leaves the arm set.
async fn confirm_retire(State(state): State<Arc<AppState>>, Path(id): Path<i64>, Json(b): Json<RetireDecision>) -> Result<Json<Value>, ApiError> {
    let who = actor(&b.actor)?;
    let mut tx = state.pool.begin().await?;
    let (variant_id, site, slot, key) = open_retire_request(&mut tx, id).await?;
    sqlx::query("UPDATE variants SET status = 'retired' WHERE id = $1 AND status IN ('approved', 'paused')").bind(variant_id).execute(&mut *tx).await?;
    sqlx::query("UPDATE retire_requests SET status = 'confirmed', decided_by = $2, decided_at = now() WHERE id = $1")
        .bind(id)
        .bind(&who)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO approvals (variant_id, action, actor, comment) VALUES ($1, 'retire', $2, $3)")
        .bind(variant_id)
        .bind(&who)
        .bind(&b.comment)
        .execute(&mut *tx)
        .await?;
    let data = json!({ "variant_id": variant_id, "slot": slot, "variant": key, "retired_by": who, "proposal": id });
    crate::webhooks::enqueue(&mut *tx, &site, "variant.retired", data).await?;
    tx.commit().await?;
    crate::edge::changed(&state.pool, &state.edge, &site).await;
    Ok(Json(json!({ "id": id, "status": "confirmed", "variant": key, "variant_status": "retired" })))
}

/// A person declines it: the variant keeps serving, and the reason goes back to the agent.
async fn decline_retire(State(state): State<Arc<AppState>>, Path(id): Path<i64>, Json(b): Json<RetireDecision>) -> Result<Json<Value>, ApiError> {
    let who = actor(&b.actor)?;
    let Some(comment) = b.comment.as_deref().map(str::trim).filter(|c| !c.is_empty()) else {
        return Err(ApiError::BadRequest("say why the variant stays".into()));
    };
    let mut tx = state.pool.begin().await?;
    let (variant_id, _, _, key) = open_retire_request(&mut tx, id).await?;
    sqlx::query("UPDATE retire_requests SET status = 'declined', decided_by = $2, decided_at = now() WHERE id = $1")
        .bind(id)
        .bind(&who)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO approvals (variant_id, action, actor, comment) VALUES ($1, 'keep', $2, $3)")
        .bind(variant_id)
        .bind(&who)
        .bind(comment)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": "declined", "variant": key })))
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
    crate::edge::changed_by(&state.pool, &state.edge, "SELECT site_id FROM slots WHERE id = $1", slot).await;
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
    crate::edge::changed(&state.pool, &state.edge, &site).await;
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetAudience {
    /// A promoted audience (a segment with status `audience`) of the variant's site, or null to
    /// lift the gate.
    audience: Option<i64>,
    actor: Option<String>,
}

/// Restrict a variant to a promoted audience, or lift the restriction. Only an audience can
/// gate (promotion is the owner's permission to use a segment in a decision), only on its own
/// site, and never the baseline, so every visitor always has something to see.
async fn set_audience(State(state): State<Arc<AppState>>, Path(id): Path<i64>, Json(b): Json<SetAudience>) -> Result<Json<Value>, ApiError> {
    let who = actor(b.actor.as_deref().unwrap_or(""))?;
    let row: Option<(String, bool)> =
        sqlx::query_as("SELECT s.site_id, v.is_baseline FROM variants v JOIN slots s ON s.id = v.slot_id WHERE v.id = $1").bind(id).fetch_optional(&state.pool).await?;
    let (site, baseline) = row.ok_or_else(|| ApiError::NotFound(format!("unknown variant {id}")))?;
    if let Some(aud) = b.audience {
        if baseline {
            return Err(ApiError::BadRequest("the baseline is shown to everyone; it cannot be restricted to an audience".into()));
        }
        let seg: Option<(String, String)> = sqlx::query_as("SELECT site_id, status FROM segments WHERE id = $1").bind(aud).fetch_optional(&state.pool).await?;
        match seg {
            None => return Err(ApiError::NotFound(format!("unknown segment {aud}"))),
            Some((s, _)) if s != site => return Err(ApiError::BadRequest(format!("segment {aud} belongs to another site"))),
            Some((_, st)) if st != "audience" => {
                return Err(ApiError::Conflict(format!("segment {aud} is {st}; promote it to an audience before it can gate a variant")));
            }
            _ => {}
        }
    }
    // The slot row lock orders this against POST /experiments, which takes it first.
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT 1 FROM slots WHERE id = (SELECT slot_id FROM variants WHERE id = $1) FOR UPDATE").bind(id).execute(&mut *tx).await?;
    if b.audience.is_some() {
        if let Some(e) = crate::experiments::unended_with_arm(&mut tx, id).await? {
            return Err(ApiError::Conflict(format!(
                "variant {id} is an arm of experiment {e}; an audience would make its arms compare different visitors. Stop the experiment first"
            )));
        }
    }
    sqlx::query("UPDATE variants SET audience_id = $2 WHERE id = $1").bind(id).bind(b.audience).execute(&mut *tx).await?;
    tx.commit().await?;
    tracing::info!(variant = id, audience = ?b.audience, actor = %who, "variant audience set");
    crate::edge::changed(&state.pool, &state.edge, &site).await;
    Ok(Json(json!({ "variant": id, "audience": b.audience })))
}
