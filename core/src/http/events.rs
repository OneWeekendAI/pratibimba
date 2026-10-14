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
    /// Whether the visitor consented to being remembered; see [`crate::privacy`].
    #[serde(default)]
    pub consent: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventIn {
    pub id: Uuid,
    pub name: String,
    pub slot: Option<String>,
    pub decision_id: Option<Uuid>,
    /// `pick` only: the picker option keys chosen (see [`crate::stated`]).
    #[serde(default)]
    pub picks: Option<Vec<String>>,
    /// `rescore` only: an on-device decision (see [`crate::adapt`]).
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub features: Option<Value>,
    #[serde(default)]
    pub score: Option<f64>,
    /// Commerce (see [`crate::commerce`]): the order this event reports. Its id is derived from
    /// `(site, order_id)`, so one order is one row however often it is sent.
    #[serde(default)]
    pub order_id: Option<String>,
    /// The order's value in `currency`'s major unit (49.99, not 4999).
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub currency: Option<String>,
    /// `[{id, price?, quantity?, ...}]`, kept as sent.
    #[serde(default)]
    pub items: Option<Value>,
}

impl EventIn {
    pub fn is_commerce(&self) -> bool {
        self.order_id.is_some() || self.value.is_some() || self.currency.is_some() || self.items.is_some()
    }
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
    let mut req = req;
    for e in &mut req.events {
        let cur = crate::commerce::check(e.order_id.as_deref(), e.value, e.currency.as_deref(), e.items.as_ref()).map_err(ApiError::BadRequest)?;
        e.currency = cur;
        if let Some(o) = &e.order_id {
            e.id = crate::commerce::order_event_id(&req.site, o);
        }
    }
    let Some(policy) = crate::privacy::policy(&state.pool, &req.site).await? else {
        return Err(ApiError::NotFound(format!("unknown site {:?}", req.site)));
    };
    // Privacy: from here on the subject is the one stored (a pseudonym unless the request is
    // identified), and the batch holds only the fields this site collects.
    let EventsRequest { site, subject: raw, events, tz_offset, consent } = req;
    let identified = policy.identified(consent);
    let req = EventsRequest {
        subject: crate::privacy::stored_subject(&state.pool, &policy, &site, &raw, identified).await?,
        site,
        events: policy.events(events),
        tz_offset: policy.tz(tz_offset),
        consent,
    };
    let slots: Vec<(String, i64)> = sqlx::query_as("SELECT key, id FROM slots WHERE site_id = $1").bind(&req.site).fetch_all(&state.pool).await?;
    let slots: HashMap<String, i64> = slots.into_iter().collect();
    let client = crate::adapt::check(&state.pool, &req.site, &req.subject, &req.events, identified).await?;

    let mut tx = state.pool.begin().await?;
    crate::privacy::admit(&mut tx, &req.site, &raw).await?;
    crate::adapt::record(&mut tx, &req.site, &req.subject, &client).await?;
    let mut accepted = 0u64;
    let mut names: Vec<String> = Vec::new();
    for e in &req.events {
        // An unknown slot key (a stale page, a renamed slot) is kept with no slot rather than
        // failing the whole batch.
        let slot_id = e.slot.as_ref().and_then(|k| slots.get(k)).copied();
        let r = sqlx::query(
            "WITH ins AS (
                 INSERT INTO events (id, site_id, subject, name, slot_id, decision_id, received_at, tz_offset_min, picks,
                                     order_id, value, currency, items)
                 VALUES ($1, $2, $3, $4, $5, $6, (SELECT COALESCE(sim_now, now()) FROM sites WHERE id = $2), $7, $8,
                         $9, $10::int8 / 10000.0, $11, $12)
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
        .bind(&e.picks)
        .bind(&e.order_id)
        .bind(e.value.map(crate::commerce::units))
        .bind(&e.currency)
        .bind(&e.items)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() > 0 {
            accepted += 1;
            names.push(e.name.clone());
        }
    }
    crate::privacy::record_visit(&mut tx, &req.site, &req.subject, &names, policy.at).await?;
    sqlx::query("SELECT pg_notify($1, '')").bind(CHANNEL).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "accepted": accepted }))))
}
