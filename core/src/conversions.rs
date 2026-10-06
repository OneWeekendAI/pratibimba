//! Inbound conversions from a platform: signature checks and the one write path.
//!
//! Both webhooks use Stripe's documented signing scheme, so one verifier serves both:
//! the header is `t=<unix seconds>,v1=<hex>[,v1=<hex>...]`, and each `v1` is
//! `HMAC-SHA256(secret, "<t>.<raw body>")` in lowercase hex. A signature older or newer than
//! `TOLERANCE_SECS` is refused, which stops a captured request being replayed later. Several
//! `v1` entries are allowed so a secret can be rolled without downtime (Stripe does the same).
//!
//! **Dedup.** A platform event's row id is derived from `(site, source, external id)` — the
//! Stripe checkout session, the caller's order id — so a retried or re-sent webhook, or the
//! same checkout reported by two Stripe event types, hits the `events` primary key and is
//! absorbed: one row, one outbox entry, one credit.
//!
//! **Precedence** lives in the worker: see `worker` ("Platform goals are authoritative").

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

pub const TOLERANCE_SECS: i64 = 300;
pub const DEFAULT_GOAL: &str = "purchase";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Stripe,
    Conversion,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Stripe => "stripe",
            Source::Conversion => "conversion",
        }
    }
}

/// Check a `t=...,v1=...` signature header against the raw body.
pub fn verify(secret: &str, header: &str, body: &[u8], now: DateTime<Utc>) -> Result<(), &'static str> {
    let mut t: Option<i64> = None;
    let mut sigs = vec![];
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", v)) => t = v.parse().ok(),
            Some(("v1", v)) => sigs.push(v),
            _ => {}
        }
    }
    let t = t.ok_or("signature header has no timestamp")?;
    if sigs.is_empty() {
        return Err("signature header has no v1 signature");
    }
    if (now.timestamp() - t).abs() > TOLERANCE_SECS {
        return Err("signature timestamp is outside the tolerance");
    }
    for sig in sigs {
        let Some(bytes) = unhex(sig) else { continue };
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes any key length");
        mac.update(t.to_string().as_bytes());
        mac.update(b".");
        mac.update(body);
        // Constant-time comparison.
        if mac.verify_slice(&bytes).is_ok() {
            return Ok(());
        }
    }
    Err("no signature matches")
}

/// Sign a body the way `verify` checks it. For tests and for callers of the generic webhook.
pub fn sign(secret: &str, t: i64, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes any key length");
    mac.update(t.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    format!("t={t},v1={}", hex(&mac.finalize().into_bytes()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// The event id for a platform conversion: the same `(site, source, external id)` always maps
/// to the same id.
pub fn event_id(site: &str, source: Source, external_id: &str) -> Uuid {
    let h = Sha256::new().chain_update(site).chain_update([0]).chain_update(source.as_str()).chain_update([0]).chain_update(external_id).finalize();
    let mut b = [0u8; 16];
    b.copy_from_slice(&h[..16]);
    uuid::Builder::from_custom_bytes(b).into_uuid()
}

pub struct Conversion<'a> {
    pub site: &'a str,
    pub source: Source,
    pub external_id: &'a str,
    pub subject: &'a str,
    pub goal: &'a str,
    pub decision_id: Option<Uuid>,
    /// When the platform says it happened; capped at the site's clock.
    pub occurred_at: Option<DateTime<Utc>>,
}

/// Write one platform conversion and its outbox entry. Returns `false` for a duplicate.
pub async fn record(pool: &PgPool, c: &Conversion<'_>) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await?;
    let r = sqlx::query(
        "WITH clock AS (SELECT COALESCE(sim_now, now()) AS at FROM sites WHERE id = $2),
              ins AS (
                 INSERT INTO events (id, site_id, subject, name, decision_id, received_at, source, external_id, occurred_at)
                 SELECT $1, $2, $3, $4, $5, clock.at, $6, $7, LEAST(COALESCE($8, clock.at), clock.at) FROM clock
                 ON CONFLICT (id) DO NOTHING
                 RETURNING id)
         INSERT INTO pending_events (event_id) SELECT id FROM ins",
    )
    .bind(event_id(c.site, c.source, c.external_id))
    .bind(c.site)
    .bind(c.subject)
    .bind(c.goal)
    .bind(c.decision_id)
    .bind(c.source.as_str())
    .bind(c.external_id)
    .bind(c.occurred_at)
    .execute(&mut *tx)
    .await?;
    sqlx::query("SELECT pg_notify($1, '')").bind(crate::http::events::CHANNEL).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(r.rows_affected() == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_round_trip() {
        let now = Utc::now();
        let h = sign("whsec_x", now.timestamp(), b"{}");
        assert_eq!(verify("whsec_x", &h, b"{}", now), Ok(()));
        assert!(verify("whsec_y", &h, b"{}", now).is_err());
        assert!(verify("whsec_x", &h, b"{ }", now).is_err());
        assert!(verify("whsec_x", &h, b"{}", now + chrono::Duration::seconds(TOLERANCE_SECS + 1)).is_err());
    }

    #[test]
    fn a_rolled_secret_matches_any_v1() {
        let now = Utc::now();
        let good = sign("new", now.timestamp(), b"x");
        let v1 = good.split_once(",v1=").unwrap().1;
        let h = format!("t={},v1={},v1={v1}", now.timestamp(), "00".repeat(32));
        assert_eq!(verify("new", &h, b"x", now), Ok(()));
    }

    #[test]
    fn event_ids_are_stable_and_distinct() {
        assert_eq!(event_id("a", Source::Stripe, "cs_1"), event_id("a", Source::Stripe, "cs_1"));
        assert_ne!(event_id("a", Source::Stripe, "cs_1"), event_id("a", Source::Conversion, "cs_1"));
        assert_ne!(event_id("a", Source::Stripe, "cs_1"), event_id("b", Source::Stripe, "cs_1"));
    }
}
