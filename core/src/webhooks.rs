//! Webhooks out.
//!
//! A site's subscribers name a URL and the event types they want. When something they asked
//! for happens, one `webhook_deliveries` row per subscriber is written **in the same
//! transaction** as the change itself (the outbox), so an event survives a crash and a change
//! that rolls back sends nothing. The worker drains due rows, POSTs them, and keeps a record of
//! every attempt's outcome.
//!
//! - **Body**: `{ "id", "idempotency_key", "type", "site", "created_at", "data" }`. `id` and
//!   `idempotency_key` are the same UUID: one per event, shared by its subscribers and kept
//!   across retries, so a receiver can drop repeats.
//! - **Signature**: header `Pratibimba-Signature: t=<unix seconds>,v1=<hex>` where `<hex>` is
//!   HMAC-SHA256 over `"<t>.<raw body>"`, keyed with the subscriber's secret. Also sent:
//!   `Pratibimba-Event` and `Pratibimba-Idempotency-Key`.
//! - **Delivery**: any 2xx is delivered. Anything else (a non-2xx, a timeout, a refused
//!   connection) is a failed attempt, retried after [`backoff`]; after [`MAX_ATTEMPTS`] the row
//!   is `failed` and is not tried again.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

pub const EVENT_TYPES: [&str; 7] =
    ["decision", "exposure", "conversion", "variant.approved", "variant.retired", "variant.share_changed", "auto_rollback"];

/// Attempts before a delivery is given up as `failed`.
pub const MAX_ATTEMPTS: i32 = 8;
/// Rows claimed per pass.
pub const BATCH: i64 = 50;
/// A claimed row is hidden from other workers this long, in case this one dies mid-send.
const LEASE: Duration = Duration::minutes(5);
/// Delivered rows older than this are pruned.
pub const KEEP_DELIVERED: Duration = Duration::days(7);

/// The wait after the `attempts`-th failed attempt: 10 s, doubling, at most an hour.
pub fn backoff(attempts: i32) -> Duration {
    let exp = (attempts.max(1) - 1).min(20) as u32;
    Duration::seconds((10i64 << exp).min(3600))
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("a reqwest client")
}

/// The body for one event, and its idempotency key.
pub fn envelope(site: &str, event: &str, data: Value, at: DateTime<Utc>) -> (Uuid, Value) {
    debug_assert!(EVENT_TYPES.contains(&event), "unknown webhook event {event}");
    let key = Uuid::new_v4();
    (key, json!({ "id": key, "idempotency_key": key, "type": event, "site": site, "created_at": at, "data": data }))
}

/// The SQL that fans one event out to its subscribers. `$1` site, `$2` event, `$3` key, `$4` body.
pub const FAN_OUT: &str = "INSERT INTO webhook_deliveries (subscription_id, event, idempotency_key, payload)
     SELECT id, $2, $3, $4 FROM webhook_subscriptions WHERE site_id = $1 AND $2 = ANY(events)";

/// Write one event to the outbox for every subscriber of `site` that asked for `event`. Call
/// it inside the transaction that makes the change. Returns the rows written.
pub async fn enqueue<'e, E: PgExecutor<'e>>(ex: E, site: &str, event: &str, data: Value) -> Result<u64, sqlx::Error> {
    let (key, body) = envelope(site, event, data, Utc::now());
    Ok(sqlx::query(FAN_OUT).bind(site).bind(event).bind(key).bind(body).execute(ex).await?.rows_affected())
}

/// `Pratibimba-Signature` for a body sent at unix time `t`.
pub fn sign(secret: &str, t: i64, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes any key length");
    mac.update(format!("{t}.{body}").as_bytes());
    let hex: String = mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect();
    format!("t={t},v1={hex}")
}

/// A fresh subscriber secret: `whsec_` and 32 random bytes as hex.
pub fn new_secret() -> String {
    let bytes: [u8; 32] = rand::random();
    format!("whsec_{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Pass {
    pub delivered: u64,
    pub retrying: u64,
    pub failed: u64,
}

type Claimed = (i64, String, Uuid, Value, i32, String, String);

/// Send every delivery due at `now`, once each, and record what happened.
pub async fn deliver_once(pool: &PgPool, http: &reqwest::Client, now: DateTime<Utc>) -> Result<Pass> {
    let mut pass = Pass::default();
    loop {
        let rows: Vec<Claimed> = sqlx::query_as(
            "UPDATE webhook_deliveries d SET next_attempt_at = $3
               FROM webhook_subscriptions s
              WHERE s.id = d.subscription_id AND d.id IN (
                    SELECT id FROM webhook_deliveries WHERE status = 'pending' AND next_attempt_at <= $1
                     ORDER BY next_attempt_at, id LIMIT $2 FOR UPDATE SKIP LOCKED)
          RETURNING d.id, d.event, d.idempotency_key, d.payload, d.attempts, s.url, s.secret",
        )
        .bind(now)
        .bind(BATCH)
        .bind(now + LEASE)
        .fetch_all(pool)
        .await?;
        let n = rows.len();
        let sends = rows.into_iter().map(|r| send(http, r, now));
        for (id, attempts, outcome) in futures_util::future::join_all(sends).await {
            let tried = attempts + 1;
            let (status, code, error) = match outcome {
                Ok(code) if (200..300).contains(&code) => ("delivered", Some(code), None),
                Ok(code) => (if tried >= MAX_ATTEMPTS { "failed" } else { "pending" }, Some(code), Some(format!("HTTP {code}"))),
                Err(e) => (if tried >= MAX_ATTEMPTS { "failed" } else { "pending" }, None, Some(e)),
            };
            match status {
                "delivered" => pass.delivered += 1,
                "failed" => pass.failed += 1,
                _ => pass.retrying += 1,
            }
            sqlx::query(
                "UPDATE webhook_deliveries SET attempts = $2, status = $3, last_status = $4, last_error = $5,
                        delivered_at = CASE WHEN $3 = 'delivered' THEN $6 END, next_attempt_at = $7
                  WHERE id = $1",
            )
            .bind(id)
            .bind(tried)
            .bind(status)
            .bind(code.map(i32::from))
            .bind(error)
            .bind(now)
            .bind(now + backoff(tried))
            .execute(pool)
            .await?;
        }
        if n < BATCH as usize {
            return Ok(pass);
        }
    }
}

async fn send(http: &reqwest::Client, (id, event, key, payload, attempts, url, secret): Claimed, now: DateTime<Utc>) -> (i64, i32, Result<u16, String>) {
    let body = payload.to_string();
    let r = http
        .post(&url)
        .header("content-type", "application/json")
        .header("user-agent", "pratibimba-webhooks")
        .header("pratibimba-event", &event)
        .header("pratibimba-idempotency-key", key.to_string())
        .header("pratibimba-signature", sign(&secret, now.timestamp(), &body))
        .body(body)
        .send()
        .await;
    let outcome = match r {
        Ok(res) => Ok(res.status().as_u16()),
        Err(e) => Err(if e.is_timeout() { "timed out".to_string() } else { format!("could not connect: {e}") }),
    };
    (id, attempts, outcome)
}

/// Drop delivered rows older than [`KEEP_DELIVERED`]. Failed rows stay for a person to read.
pub async fn prune(pool: &PgPool, now: DateTime<Utc>) -> Result<u64> {
    Ok(sqlx::query("DELETE FROM webhook_deliveries WHERE status = 'delivered' AND delivered_at < $1")
        .bind(now - KEEP_DELIVERED)
        .execute(pool)
        .await?
        .rows_affected())
}
