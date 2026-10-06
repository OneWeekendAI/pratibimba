//! `POST /e`: ingest events. Each event row and its outbox row are written in one transaction
//! together with a `NOTIFY`, so the worker wakes promptly and an event is never visible without
//! its outbox entry (or the reverse). Event ids are client-generated idempotency keys; a
//! retried batch is absorbed, not double-counted.

use super::{ApiError, AppState, valid_subject};
use crate::site_file::is_event_name;
use axum::{Json, extract::State, http::StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

pub const MAX_EVENTS: usize = 100;
pub const CHANNEL: &str = "prb_events";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventsRequest {
    pub site: String,
    pub subject: String,
    pub events: Vec<EventIn>,
    /// The visitor's UTC offset in minutes east (`-new Date().getTimezoneOffset()`), for the
    /// local-time insights. Optional; see `insights::localtime`.
    #[serde(default)]
    pub tz_offset: Option<i16>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventIn {
    pub id: Uuid,
    pub name: String,
    pub slot: Option<String>,
    pub decision_id: Option<Uuid>,
}

pub async fn handler(State(state): State<Arc<AppState>>, Json(req): Json<EventsRequest>) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !valid_subject(&req.subject) {
        return Err(ApiError::BadRequest("subject must be 1-256 printable ASCII characters".into()));
    }
    if req.events.is_empty() || req.events.len() > MAX_EVENTS {
        return Err(ApiError::BadRequest(format!("send 1-{MAX_EVENTS} events per request")));
    }
    if let Some(bad) = req.events.iter().find(|e| !is_event_name(&e.name)) {
        return Err(ApiError::BadRequest(format!("event name {:?} must match [a-z0-9_:]{{1,64}}", bad.name)));
    }
    if req.tz_offset.is_some_and(|m| !crate::insights::localtime::valid_offset(m)) {
        return Err(ApiError::BadRequest("tz_offset is minutes east of UTC, -720 to 840".into()));
    }
    let slots: Vec<(String, i64)> = sqlx::query_as("SELECT key, id FROM slots WHERE site_id = $1").bind(&req.site).fetch_all(&state.pool).await?;
    if slots.is_empty() {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sites WHERE id = $1)").bind(&req.site).fetch_one(&state.pool).await?;
        if !exists {
            return Err(ApiError::NotFound(format!("unknown site {:?}", req.site)));
        }
    }
    let slots: HashMap<String, i64> = slots.into_iter().collect();

    let mut tx = state.pool.begin().await?;
    let mut accepted = 0u64;
    for e in &req.events {
        // An unknown slot key (a stale page, a renamed slot) is kept with no slot rather than
        // failing the whole batch.
        let slot_id = e.slot.as_ref().and_then(|k| slots.get(k)).copied();
        let r = sqlx::query(
            "WITH ins AS (
                 INSERT INTO events (id, site_id, subject, name, slot_id, decision_id, received_at, tz_offset_min)
                 VALUES ($1, $2, $3, $4, $5, $6, (SELECT COALESCE(sim_now, now()) FROM sites WHERE id = $2), $7)
                 ON CONFLICT (id) DO NOTHING
                 RETURNING id)
             INSERT INTO pending_events (event_id) SELECT id FROM ins",
        )
        .bind(e.id)
        .bind(&req.site)
        .bind(&req.subject)
        .bind(&e.name)
        .bind(slot_id)
        .bind(e.decision_id)
        .bind(req.tz_offset)
        .execute(&mut *tx)
        .await?;
        accepted += r.rows_affected();
    }
    sqlx::query("SELECT pg_notify($1, '')").bind(CHANNEL).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "accepted": accepted }))))
}
