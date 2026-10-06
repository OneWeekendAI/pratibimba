//! Inbound conversion webhooks (public) and their configuration (admin).
//!
//! - `POST /webhook/stripe/:site`: Stripe `checkout.session.completed` and
//!   `checkout.session.async_payment_succeeded`, verified with the site's Stripe signing
//!   secret (`Stripe-Signature`). The checkout session joins to a visitor by
//!   `client_reference_id` (or `metadata.prb_subject`): pass the Pratibimba subject there when
//!   creating the session. `metadata.prb_decision_id` names the decision, `metadata.prb_event`
//!   the goal (default `purchase`). The session id is the dedup key.
//! - `POST /webhook/conversion/:site`: anything else, signed the same way with the site's
//!   conversion secret (`Pratibimba-Signature`). Body:
//!   `{id, subject, event?, decision_id?, occurred_at?}`; `id` is the dedup key.
//! - `GET|POST /api/sites/:site/conversion-sources`: set the secrets, the goals the platform
//!   is authoritative for, and the grace period. Secrets are write-only.
//!
//! A request that fails verification gets 400 and writes nothing. A verified Stripe event that
//! cannot be a conversion (another type, unpaid, no subject) gets 200 so Stripe stops retrying.

use super::{ApiError, AppState, valid_subject};
use crate::conversions::{self, Conversion, DEFAULT_GOAL, Source};
use crate::site_file::is_event_name;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

pub const STRIPE_HEADER: &str = "stripe-signature";
pub const CONVERSION_HEADER: &str = "pratibimba-signature";

/// The site's secret for one source. 404 for an unknown site, 400 when none is set.
async fn secret(state: &AppState, site: &str, source: Source) -> Result<String, ApiError> {
    let col = match source {
        Source::Stripe => "stripe_signing_secret",
        Source::Conversion => "conversion_signing_secret",
    };
    let row: Option<Option<String>> =
        sqlx::query_scalar(&format!("SELECT {col} FROM sites WHERE id = $1")).bind(site).fetch_optional(&state.pool).await?;
    match row {
        None => Err(ApiError::NotFound(format!("unknown site {site:?}"))),
        Some(None) => Err(ApiError::BadRequest(format!("site {site:?} has no {} secret configured", source.as_str()))),
        Some(Some(s)) => Ok(s),
    }
}

fn verified(headers: &HeaderMap, name: &str, secret: &str, body: &[u8]) -> Result<(), ApiError> {
    let header = headers.get(name).and_then(|v| v.to_str().ok()).ok_or_else(|| ApiError::BadRequest(format!("missing {name} header")))?;
    conversions::verify(secret, header, body, Utc::now()).map_err(|e| ApiError::BadRequest(format!("invalid signature: {e}")))
}

fn ignored(reason: &str) -> Json<Value> {
    Json(json!({ "received": true, "ignored": reason }))
}

pub async fn stripe(State(state): State<Arc<AppState>>, Path(site): Path<String>, headers: HeaderMap, body: Bytes) -> Result<Json<Value>, ApiError> {
    let secret = secret(&state, &site, Source::Stripe).await?;
    verified(&headers, STRIPE_HEADER, &secret, &body)?;
    let event: Value = serde_json::from_slice(&body).map_err(|e| ApiError::BadRequest(format!("body is not JSON: {e}")))?;
    let kind = event["type"].as_str().unwrap_or_default();
    if !matches!(kind, "checkout.session.completed" | "checkout.session.async_payment_succeeded") {
        return Ok(ignored("event type"));
    }
    let session = &event["data"]["object"];
    if !matches!(session["payment_status"].as_str(), Some("paid" | "no_payment_required")) {
        // An async payment method completes later as `async_payment_succeeded`.
        return Ok(ignored("not paid"));
    }
    let Some(session_id) = session["id"].as_str().filter(|s| !s.is_empty()) else {
        return Err(ApiError::BadRequest("checkout session has no id".into()));
    };
    let meta = &session["metadata"];
    let Some(subject) = session["client_reference_id"].as_str().or_else(|| meta["prb_subject"].as_str()).filter(|s| valid_subject(s)) else {
        return Ok(ignored("no subject: set client_reference_id to the Pratibimba subject"));
    };
    let goal = meta["prb_event"].as_str().filter(|g| is_event_name(g)).unwrap_or(DEFAULT_GOAL);
    let decision_id = meta["prb_decision_id"].as_str().and_then(|d| Uuid::parse_str(d).ok());
    // Stripe times are whole seconds; read one as the end of its second, so a checkout in the
    // same second as its decision does not appear to precede it. `record` caps it at the clock.
    let occurred_at = event["created"].as_i64().and_then(|t| DateTime::<Utc>::from_timestamp(t, 999_999_999));
    let c = Conversion { site: &site, source: Source::Stripe, external_id: session_id, subject, goal, decision_id, occurred_at };
    let new = conversions::record(&state.pool, &c).await?;
    Ok(Json(json!({ "received": true, "duplicate": !new })))
}

#[derive(Debug, Deserialize)]
struct Generic {
    id: Option<String>,
    subject: Option<String>,
    event: Option<String>,
    decision_id: Option<Uuid>,
    occurred_at: Option<DateTime<Utc>>,
}

pub async fn conversion(State(state): State<Arc<AppState>>, Path(site): Path<String>, headers: HeaderMap, body: Bytes) -> Result<Json<Value>, ApiError> {
    let secret = secret(&state, &site, Source::Conversion).await?;
    verified(&headers, CONVERSION_HEADER, &secret, &body)?;
    let g: Generic = serde_json::from_slice(&body).map_err(|e| ApiError::BadRequest(format!("bad body: {e}")))?;
    let Some(id) = g.id.as_deref().filter(|i| !i.is_empty() && i.len() <= 256) else {
        return Err(ApiError::BadRequest("id (the order or conversion id, 1-256 chars) is required".into()));
    };
    let Some(subject) = g.subject.as_deref().filter(|s| valid_subject(s)) else {
        return Err(ApiError::BadRequest("subject must be 1-256 printable ASCII characters".into()));
    };
    let goal = g.event.as_deref().unwrap_or(DEFAULT_GOAL);
    if !is_event_name(goal) {
        return Err(ApiError::BadRequest(format!("event {goal:?} must match [a-z0-9_:]{{1,64}}")));
    }
    let c = Conversion { site: &site, source: Source::Conversion, external_id: id, subject, goal, decision_id: g.decision_id, occurred_at: g.occurred_at };
    let new = conversions::record(&state.pool, &c).await?;
    Ok(Json(json!({ "received": true, "duplicate": !new })))
}

pub fn admin_routes() -> Router<Arc<AppState>> {
    Router::new().route("/sites/{site}/conversion-sources", get(sources).post(set_sources))
}

async fn sources(State(state): State<Arc<AppState>>, Path(site): Path<String>) -> Result<Json<Value>, ApiError> {
    let row: Option<(bool, bool, Vec<String>, i32)> = sqlx::query_as(
        "SELECT stripe_signing_secret IS NOT NULL, conversion_signing_secret IS NOT NULL, authoritative_goals, platform_grace_secs
           FROM sites WHERE id = $1",
    )
    .bind(&site)
    .fetch_optional(&state.pool)
    .await?;
    let Some((stripe, conversion, goals, grace)) = row else {
        return Err(ApiError::NotFound(format!("unknown site {site:?}")));
    };
    Ok(Json(json!({
        "stripe": stripe, "conversion": conversion, "goals": goals, "grace_secs": grace,
        "endpoints": { "stripe": format!("/webhook/stripe/{site}"), "conversion": format!("/webhook/conversion/{site}") },
    })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetSources {
    /// `""` clears a secret; absent leaves it as it is.
    stripe_signing_secret: Option<String>,
    conversion_signing_secret: Option<String>,
    goals: Option<Vec<String>>,
    grace_secs: Option<i32>,
}

async fn set_sources(State(state): State<Arc<AppState>>, Path(site): Path<String>, Json(b): Json<SetSources>) -> Result<Json<Value>, ApiError> {
    if let Some(bad) = b.goals.iter().flatten().find(|g| !is_event_name(g)) {
        return Err(ApiError::BadRequest(format!("goal {bad:?} must match [a-z0-9_:]{{1,64}}")));
    }
    if b.goals.as_ref().is_some_and(|g| g.iter().any(|g| g == "exposure" || g == "missed")) {
        return Err(ApiError::BadRequest("exposure and missed are not goals".into()));
    }
    if b.grace_secs.is_some_and(|g| !(0..=7 * 86_400).contains(&g)) {
        return Err(ApiError::BadRequest("grace_secs must be 0 to 604800".into()));
    }
    let r = sqlx::query(
        "UPDATE sites SET
             stripe_signing_secret     = CASE WHEN $2::text IS NULL THEN stripe_signing_secret ELSE NULLIF($2, '') END,
             conversion_signing_secret = CASE WHEN $3::text IS NULL THEN conversion_signing_secret ELSE NULLIF($3, '') END,
             authoritative_goals       = COALESCE($4, authoritative_goals),
             platform_grace_secs       = COALESCE($5, platform_grace_secs)
          WHERE id = $1 AND sandbox_of IS NULL",
    )
    .bind(&site)
    .bind(&b.stripe_signing_secret)
    .bind(&b.conversion_signing_secret)
    .bind(&b.goals)
    .bind(b.grace_secs)
    .execute(&state.pool)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("unknown site {site:?}")));
    }
    sources(State(state), Path(site)).await
}
