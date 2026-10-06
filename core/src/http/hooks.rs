//! `/api` routes for webhooks out: create, list and delete a site's subscriptions, and read a
//! subscription's delivery records. Mounted inside `admin`, so every route needs the admin token.
//! The secret is returned once, by create; nothing else ever shows it.

use super::{ApiError, AppState};
use crate::webhooks::{self, EVENT_TYPES};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/sites/{site}/webhooks", get(list).post(create))
        .route("/webhooks/{id}", axum::routing::delete(remove))
        .route("/webhooks/{id}/deliveries", get(deliveries))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewSubscription {
    url: String,
    events: Vec<String>,
}

async fn site_exists(state: &AppState, site: &str) -> Result<(), ApiError> {
    let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sites WHERE id = $1)").bind(site).fetch_one(&state.pool).await?;
    if ok { Ok(()) } else { Err(ApiError::NotFound(format!("unknown site {site:?}"))) }
}

async fn create(State(state): State<Arc<AppState>>, Path(site): Path<String>, Json(b): Json<NewSubscription>) -> Result<(StatusCode, Json<Value>), ApiError> {
    site_exists(&state, &site).await?;
    let url = b.url.trim();
    let parsed = reqwest::Url::parse(url).map_err(|_| ApiError::BadRequest("url must be a full http:// or https:// URL".into()))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host().is_none() || url.len() > 2048 {
        return Err(ApiError::BadRequest("url must be a full http:// or https:// URL".into()));
    }
    if b.events.is_empty() {
        return Err(ApiError::BadRequest(format!("pick at least one event: {}", EVENT_TYPES.join(", "))));
    }
    if let Some(bad) = b.events.iter().find(|e| !EVENT_TYPES.contains(&e.as_str())) {
        return Err(ApiError::BadRequest(format!("unknown event {bad:?}; events are {}", EVENT_TYPES.join(", "))));
    }
    // Keep the caller's order, drop repeats.
    let mut events: Vec<&str> = vec![];
    for e in &b.events {
        if !events.contains(&e.as_str()) {
            events.push(e);
        }
    }
    let secret = webhooks::new_secret();
    let (id, created): (i64, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO webhook_subscriptions (site_id, url, events, secret) VALUES ($1, $2, $3, $4) RETURNING id, created_at",
    )
    .bind(&site)
    .bind(url)
    .bind(&events)
    .bind(&secret)
    .fetch_one(&state.pool)
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "url": url, "events": events, "secret": secret, "created_at": created }))))
}

type SubRow = (i64, String, Vec<String>, DateTime<Utc>, i64, i64, i64, Option<DateTime<Utc>>);

async fn list(State(state): State<Arc<AppState>>, Path(site): Path<String>) -> Result<Json<Value>, ApiError> {
    site_exists(&state, &site).await?;
    let rows: Vec<SubRow> = sqlx::query_as(
        "SELECT s.id, s.url, s.events, s.created_at,
                count(*) FILTER (WHERE d.status = 'pending'), count(*) FILTER (WHERE d.status = 'delivered'),
                count(*) FILTER (WHERE d.status = 'failed'), max(d.delivered_at)
           FROM webhook_subscriptions s LEFT JOIN webhook_deliveries d ON d.subscription_id = s.id
          WHERE s.site_id = $1 GROUP BY s.id ORDER BY s.id",
    )
    .bind(&site)
    .fetch_all(&state.pool)
    .await?;
    let subs: Vec<Value> = rows
        .into_iter()
        .map(|(id, url, events, created, pending, delivered, failed, last)| {
            json!({ "id": id, "url": url, "events": events, "created_at": created,
                    "counts": { "pending": pending, "delivered": delivered, "failed": failed }, "last_delivered_at": last })
        })
        .collect();
    Ok(Json(json!({ "subscriptions": subs, "event_types": EVENT_TYPES })))
}

async fn remove(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    let n = sqlx::query("DELETE FROM webhook_subscriptions WHERE id = $1").bind(id).execute(&state.pool).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound(format!("unknown webhook subscription {id}")));
    }
    Ok(StatusCode::NO_CONTENT)
}

type DeliveryRow = (i64, String, Uuid, String, i32, Option<i32>, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>, DateTime<Utc>);

async fn deliveries(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM webhook_subscriptions WHERE id = $1)").bind(id).fetch_one(&state.pool).await?;
    if !exists {
        return Err(ApiError::NotFound(format!("unknown webhook subscription {id}")));
    }
    let rows: Vec<DeliveryRow> = sqlx::query_as(
        "SELECT id, event, idempotency_key, status, attempts, last_status, last_error, next_attempt_at, delivered_at, created_at
           FROM webhook_deliveries WHERE subscription_id = $1 ORDER BY id DESC LIMIT 50",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let out: Vec<Value> = rows
        .into_iter()
        .map(|(id, event, key, status, attempts, code, error, next, delivered, created)| {
            json!({ "id": id, "event": event, "idempotency_key": key, "status": status, "attempts": attempts,
                    "last_status": code, "last_error": error, "next_attempt_at": if status == "pending" { Some(next) } else { None },
                    "delivered_at": delivered, "created_at": created })
        })
        .collect();
    Ok(Json(json!({ "deliveries": out })))
}
