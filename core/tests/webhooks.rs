//! Webhooks out: subscriptions per site, an outbox row written in the same transaction as the
//! change it reports, HMAC-signed delivery with retries and backoff, and a delivery record.
//!
//! The receiver here checks signatures with its own HMAC code, not Core's, so a broken signer
//! in Core turns these tests red.

mod common;
use axum::{Router, body::Body, extract::State, http::{HeaderMap, Request}, routing::post as route_post};
use chrono::{DateTime, Duration, SubsecRound, Utc};
use common::*;
use hmac::{Hmac, Mac};
use pratibimba::webhooks;
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::PgPool;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

// ---- a local receiver ----

#[derive(Clone, Default)]
struct Receiver {
    seen: Arc<Mutex<Vec<(HeaderMap, String)>>>,
    /// Status codes to answer with, in order; 200 once the script runs out.
    script: Arc<Mutex<VecDeque<u16>>>,
}

impl Receiver {
    fn seen(&self) -> Vec<(HeaderMap, String)> {
        self.seen.lock().unwrap().clone()
    }
    fn bodies(&self) -> Vec<Value> {
        self.seen().iter().map(|(_, b)| serde_json::from_str(b).unwrap()).collect()
    }
}

async fn receive(State(r): State<Receiver>, headers: HeaderMap, body: String) -> axum::http::StatusCode {
    r.seen.lock().unwrap().push((headers, body));
    let code = r.script.lock().unwrap().pop_front().unwrap_or(200);
    axum::http::StatusCode::from_u16(code).unwrap()
}

/// Start a receiver; returns it and its URL.
async fn receiver(script: &[u16]) -> (Receiver, String) {
    let r = Receiver { script: Arc::new(Mutex::new(script.iter().copied().collect())), ..Default::default() };
    let app = Router::new().route("/hook", route_post(receive)).with_state(r.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (r, url)
}

/// The receiver's own check: `t=<unix>,v1=<hex HMAC-SHA256(secret, "<t>.<body>")>`.
fn verifies(secret: &str, headers: &HeaderMap, body: &str) -> bool {
    let Some(sig) = headers.get("pratibimba-signature").and_then(|v| v.to_str().ok()) else { return false };
    let mut t = None;
    let mut v1 = None;
    for part in sig.split(',') {
        match part.trim().split_once('=') {
            Some(("t", x)) => t = Some(x.to_string()),
            Some(("v1", x)) => v1 = Some(x.to_string()),
            _ => {}
        }
    }
    let (Some(t), Some(v1)) = (t, v1) else { return false };
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{t}.{body}").as_bytes());
    let want: String = mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect();
    want == v1
}

// ---- helpers ----

async fn api_delete(app: &Router, path: &str) -> Resp {
    send(app, Request::delete(path).header("authorization", format!("Bearer {TOKEN}")).body(Body::empty()).unwrap()).await
}

async fn subscribe(app: &Router, url: &str, events: &[&str]) -> (i64, String) {
    let r = api_post(app, "/api/sites/demo/webhooks", json!({ "url": url, "events": events })).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    let j = r.json();
    (j["id"].as_i64().unwrap(), j["secret"].as_str().unwrap().to_string())
}

async fn deliver(pool: &PgPool, now: DateTime<Utc>) -> webhooks::Pass {
    webhooks::deliver_once(pool, &webhooks::client(), now).await.unwrap()
}

async fn rows(pool: &PgPool) -> i64 {
    count(pool, "SELECT count(*) FROM webhook_deliveries").await
}

async fn record(pool: &PgPool) -> (String, i32, DateTime<Utc>) {
    sqlx::query_as("SELECT status, attempts, next_attempt_at FROM webhook_deliveries").fetch_one(pool).await.unwrap()
}

async fn setup(pool: &PgPool) -> Router {
    setup_demo(pool).await;
    admin_app(pool)
}

// ---- the subscription API ----

#[sqlx::test(migrations = false)]
async fn create_returns_the_secret_once_and_list_hides_it(pool: PgPool) {
    let app = setup(&pool).await;
    let (id, secret) = subscribe(&app, "https://example.com/hook", &["decision", "conversion"]).await;
    assert!(secret.starts_with("whsec_") && secret.len() > 30, "{secret}");
    let l = api_get(&app, "/api/sites/demo/webhooks").await;
    assert_eq!(l.status, StatusCode::OK);
    assert!(!l.text.contains(&secret), "the list must not carry the secret: {}", l.text);
    let subs = l.json()["subscriptions"].clone();
    assert_eq!(subs[0]["id"], id);
    assert_eq!(subs[0]["events"], json!(["decision", "conversion"]));
}

#[sqlx::test(migrations = false)]
async fn list_names_every_event_type(pool: PgPool) {
    let app = setup(&pool).await;
    let l = api_get(&app, "/api/sites/demo/webhooks").await.json();
    assert_eq!(l["event_types"], json!(["decision", "exposure", "conversion", "variant.approved", "variant.retired", "variant.share_changed", "auto_rollback"]));
}

#[sqlx::test(migrations = false)]
async fn unknown_event_type_is_refused(pool: PgPool) {
    let app = setup(&pool).await;
    let r = api_post(&app, "/api/sites/demo/webhooks", json!({ "url": "https://example.com/h", "events": ["decision", "purchase"] })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn empty_event_list_is_refused(pool: PgPool) {
    let app = setup(&pool).await;
    let r = api_post(&app, "/api/sites/demo/webhooks", json!({ "url": "https://example.com/h", "events": [] })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn non_http_url_is_refused(pool: PgPool) {
    let app = setup(&pool).await;
    let r = api_post(&app, "/api/sites/demo/webhooks", json!({ "url": "ftp://example.com/h", "events": ["decision"] })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn unknown_site_is_404(pool: PgPool) {
    let app = setup(&pool).await;
    let r = api_post(&app, "/api/sites/nope/webhooks", json!({ "url": "https://example.com/h", "events": ["decision"] })).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn delete_removes_the_subscription(pool: PgPool) {
    let app = setup(&pool).await;
    let (id, _) = subscribe(&app, "https://example.com/hook", &["decision"]).await;
    assert_eq!(api_delete(&app, &format!("/api/webhooks/{id}")).await.status, StatusCode::NO_CONTENT);
    assert_eq!(api_get(&app, "/api/sites/demo/webhooks").await.json()["subscriptions"], json!([]));
    assert_eq!(api_delete(&app, &format!("/api/webhooks/{id}")).await.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = false)]
async fn the_api_needs_the_admin_token(pool: PgPool) {
    let app = setup(&pool).await;
    assert_eq!(get(&app, "/api/sites/demo/webhooks").await.status, StatusCode::UNAUTHORIZED);
}

// ---- signing ----

#[sqlx::test(migrations = false)]
async fn a_decision_is_delivered_signed_with_the_subscriber_secret(pool: PgPool) {
    let app = setup(&pool).await;
    let (rx, url) = receiver(&[]).await;
    let (_, secret) = subscribe(&app, &url, &["decision"]).await;
    let d = decide(&app, "v1", false).await.json();
    let pass = deliver(&pool, Utc::now()).await;
    assert_eq!(pass.delivered, 1, "{pass:?}");
    let seen = rx.seen();
    assert_eq!(seen.len(), 1);
    let (h, body) = &seen[0];
    assert!(verifies(&secret, h, body), "signature does not verify: {:?}", h.get("pratibimba-signature"));
    let b: Value = serde_json::from_str(body).unwrap();
    assert_eq!(b["type"], "decision");
    assert_eq!(b["site"], "demo");
    assert_eq!(b["data"]["decision_id"], d["decision_id"]);
    assert_eq!(h.get("pratibimba-event").unwrap(), "decision");
    assert_eq!(h.get("pratibimba-idempotency-key").unwrap().to_str().unwrap(), b["idempotency_key"].as_str().unwrap());
}

#[sqlx::test(migrations = false)]
async fn another_subscribers_secret_does_not_verify(pool: PgPool) {
    let app = setup(&pool).await;
    let (rx, url) = receiver(&[]).await;
    let (_, mine) = subscribe(&app, &url, &["decision"]).await;
    let (other_rx, other_url) = receiver(&[]).await;
    let (_, theirs) = subscribe(&app, &other_url, &["decision"]).await;
    decide(&app, "v1", false).await;
    deliver(&pool, Utc::now()).await;
    let (h, body) = &rx.seen()[0];
    assert!(verifies(&mine, h, body));
    assert!(!verifies(&theirs, h, body), "each subscriber is signed with its own secret");
    let (h2, body2) = &other_rx.seen()[0];
    assert!(verifies(&theirs, h2, body2));
}

#[sqlx::test(migrations = false)]
async fn a_tampered_body_does_not_verify(pool: PgPool) {
    let app = setup(&pool).await;
    let (rx, url) = receiver(&[]).await;
    let (_, secret) = subscribe(&app, &url, &["decision"]).await;
    decide(&app, "v1", false).await;
    deliver(&pool, Utc::now()).await;
    let (h, body) = &rx.seen()[0];
    assert!(!verifies(&secret, h, &body.replace("demo", "dem0")));
}

#[sqlx::test(migrations = false)]
async fn the_signature_timestamp_is_the_send_time(pool: PgPool) {
    let app = setup(&pool).await;
    let (rx, url) = receiver(&[]).await;
    subscribe(&app, &url, &["decision"]).await;
    decide(&app, "v1", false).await;
    let now = Utc::now();
    deliver(&pool, now).await;
    let sig = rx.seen()[0].0.get("pratibimba-signature").unwrap().to_str().unwrap().to_string();
    assert!(sig.starts_with(&format!("t={},", now.timestamp())), "{sig}");
}

// ---- which events reach whom ----

#[sqlx::test(migrations = false)]
async fn an_unsubscribed_event_type_is_not_delivered(pool: PgPool) {
    let app = setup(&pool).await;
    let (rx, url) = receiver(&[]).await;
    subscribe(&app, &url, &["conversion"]).await;
    decide(&app, "v1", false).await;
    assert_eq!(rows(&pool).await, 0, "no outbox row for an event type nobody subscribed to");
    deliver(&pool, Utc::now()).await;
    assert!(rx.seen().is_empty());
}

#[sqlx::test(migrations = false)]
async fn another_sites_subscriber_gets_nothing(pool: PgPool) {
    let app = setup(&pool).await;
    sqlx::query("INSERT INTO sites (id, origin) VALUES ('other', 'http://o')").execute(&pool).await.unwrap();
    let (rx, url) = receiver(&[]).await;
    let r = api_post(&app, "/api/sites/other/webhooks", json!({ "url": url, "events": ["decision"] })).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    decide(&app, "v1", false).await;
    deliver(&pool, Utc::now()).await;
    assert!(rx.seen().is_empty());
}

#[sqlx::test(migrations = false)]
async fn a_deleted_subscription_gets_nothing_more(pool: PgPool) {
    let app = setup(&pool).await;
    let (rx, url) = receiver(&[]).await;
    let (id, _) = subscribe(&app, &url, &["decision"]).await;
    decide(&app, "v1", false).await;
    api_delete(&app, &format!("/api/webhooks/{id}")).await;
    deliver(&pool, Utc::now()).await;
    assert!(rx.seen().is_empty());
}

#[sqlx::test(migrations = false)]
async fn exposure_is_delivered(pool: PgPool) {
    let app = setup(&pool).await;
    let (rx, url) = receiver(&[]).await;
    subscribe(&app, &url, &["exposure"]).await;
    let d = decide(&app, "v1", false).await.json();
    let id = d["decision_id"].as_str().unwrap().to_string();
    event(&app, "v1", "exposure", Some(&id)).await;
    pratibimba::worker::run_once(&pool, Utc::now()).await.unwrap();
    deliver(&pool, Utc::now()).await;
    let b = rx.bodies();
    assert_eq!(b.len(), 1, "{b:?}");
    assert_eq!(b[0]["type"], "exposure");
    assert_eq!(b[0]["data"]["decision_id"], id.as_str());
}

#[sqlx::test(migrations = false)]
async fn a_repeated_exposure_is_delivered_once(pool: PgPool) {
    let app = setup(&pool).await;
    let (rx, url) = receiver(&[]).await;
    subscribe(&app, &url, &["exposure"]).await;
    let id = decide(&app, "v1", false).await.json()["decision_id"].as_str().unwrap().to_string();
    event(&app, "v1", "exposure", Some(&id)).await;
    event(&app, "v1", "exposure", Some(&id)).await;
    pratibimba::worker::run_once(&pool, Utc::now()).await.unwrap();
    deliver(&pool, Utc::now()).await;
    assert_eq!(rx.seen().len(), 1);
}

#[sqlx::test(migrations = false)]
async fn conversion_is_delivered(pool: PgPool) {
    let app = setup(&pool).await;
    let (rx, url) = receiver(&[]).await;
    subscribe(&app, &url, &["conversion"]).await;
    let id = decide(&app, "v1", false).await.json()["decision_id"].as_str().unwrap().to_string();
    event(&app, "v1", "exposure", Some(&id)).await;
    event(&app, "v1", "click", Some(&id)).await;
    pratibimba::worker::run_once(&pool, Utc::now()).await.unwrap();
    deliver(&pool, Utc::now()).await;
    let b = rx.bodies();
    assert_eq!(b.len(), 1, "{b:?}");
    assert_eq!(b[0]["type"], "conversion");
    assert_eq!(b[0]["data"]["decision_id"], id.as_str());
    assert_eq!(b[0]["data"]["metric"], "click");
}

#[sqlx::test(migrations = false)]
async fn variant_approved_is_delivered(pool: PgPool) {
    let app = setup(&pool).await;
    let (rx, url) = receiver(&[]).await;
    subscribe(&app, &url, &["variant.approved"]).await;
    let r = api_post(&app, "/api/sites/demo/slots/hero/variants", json!({ "key": "ship", "html": "<h2>Ship</h2>" })).await;
    let id = r.json()["id"].as_i64().unwrap();
    let a = api_post(&app, &format!("/api/variants/{id}/approve"), json!({ "approver": "Asha", "traffic_share": 0.1 })).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    deliver(&pool, Utc::now()).await;
    let b = rx.bodies();
    assert_eq!(b.len(), 1, "{b:?}");
    assert_eq!(b[0]["type"], "variant.approved");
    assert_eq!(b[0]["data"]["variant"], "ship");
    assert_eq!(b[0]["data"]["approved_by"], "Asha");
}

#[sqlx::test(migrations = false)]
async fn reapplying_the_site_file_does_not_reapprove(pool: PgPool) {
    let app = setup(&pool).await;
    let (rx, url) = receiver(&[]).await;
    subscribe(&app, &url, &["variant.approved"]).await;
    pratibimba::site_file::apply(&pool, &demo_file()).await.unwrap();
    deliver(&pool, Utc::now()).await;
    assert!(rx.seen().is_empty(), "nothing changed status: {:?}", rx.bodies());
}

#[sqlx::test(migrations = false)]
async fn variant_retired_is_delivered(pool: PgPool) {
    let app = setup(&pool).await;
    let (rx, url) = receiver(&[]).await;
    subscribe(&app, &url, &["variant.retired"]).await;
    pratibimba::site_file::apply(
        &pool,
        &file_from(json!({ "site": "demo", "origin": "http://localhost:5173",
            "slots": [{ "key": "hero", "selector": "#hero", "variants": [{ "key": "baseline", "baseline": true }] }] })),
    )
    .await
    .unwrap();
    deliver(&pool, Utc::now()).await;
    let b = rx.bodies();
    assert_eq!(b.len(), 1, "{b:?}");
    assert_eq!(b[0]["type"], "variant.retired");
    assert_eq!(b[0]["data"]["variant"], "gift");
}

#[sqlx::test(migrations = false)]
async fn auto_rollback_is_delivered(pool: PgPool) {
    let app = setup(&pool).await;
    let (rx, url) = receiver(&[]).await;
    subscribe(&app, &url, &["auto_rollback"]).await;
    seed_decisions(&pool, "gift", false, 1000, 20, 60).await;
    seed_decisions(&pool, "baseline", true, 1000, 100, 60).await;
    assert_eq!(pratibimba::guard::run(&pool, Utc::now()).await.unwrap().len(), 1);
    deliver(&pool, Utc::now()).await;
    let b = rx.bodies();
    assert_eq!(b.len(), 1, "{b:?}");
    assert_eq!(b[0]["type"], "auto_rollback");
    assert_eq!(b[0]["data"]["variant"], "gift");
    assert!(b[0]["data"]["sentence"].as_str().unwrap().contains("auto-rollback"));
}

// ---- retries ----

#[sqlx::test(migrations = false)]
async fn a_failed_delivery_is_retried_after_its_backoff(pool: PgPool) {
    let app = setup(&pool).await;
    let (rx, url) = receiver(&[500, 503]).await;
    subscribe(&app, &url, &["decision"]).await;
    decide(&app, "v1", false).await;
    let t0 = Utc::now().trunc_subsecs(6); // Postgres keeps microseconds
    assert_eq!(deliver(&pool, t0).await.retrying, 1);
    let (status, attempts, next) = record(&pool).await;
    assert_eq!((status.as_str(), attempts), ("pending", 1));
    assert_eq!(next, t0 + webhooks::backoff(1), "first retry waits one backoff step");
    // Not due yet: nothing is sent.
    deliver(&pool, t0 + webhooks::backoff(1) - Duration::seconds(1)).await;
    assert_eq!(rx.seen().len(), 1);
    let t1 = t0 + webhooks::backoff(1);
    assert_eq!(deliver(&pool, t1).await.retrying, 1);
    assert_eq!(record(&pool).await.2, t1 + webhooks::backoff(2));
    let t2 = t1 + webhooks::backoff(2);
    assert_eq!(deliver(&pool, t2).await.delivered, 1);
    assert_eq!(record(&pool).await.0, "delivered");
    let seen = rx.bodies();
    assert_eq!(seen.len(), 3);
    assert!(seen.iter().all(|b| b["idempotency_key"] == seen[0]["idempotency_key"]), "a retry keeps its idempotency key");
}

#[test]
fn backoff_doubles_and_is_capped() {
    assert_eq!(webhooks::backoff(1), Duration::seconds(10));
    assert_eq!(webhooks::backoff(2), Duration::seconds(20));
    assert_eq!(webhooks::backoff(3), Duration::seconds(40));
    assert_eq!(webhooks::backoff(20), Duration::hours(1));
}

#[sqlx::test(migrations = false)]
async fn delivery_gives_up_after_the_last_attempt(pool: PgPool) {
    let app = setup(&pool).await;
    let (rx, url) = receiver(&[500; 32]).await;
    subscribe(&app, &url, &["decision"]).await;
    decide(&app, "v1", false).await;
    let mut t = Utc::now();
    for _ in 0..webhooks::MAX_ATTEMPTS + 3 {
        deliver(&pool, t).await;
        t += Duration::hours(2);
    }
    assert_eq!(rx.seen().len(), webhooks::MAX_ATTEMPTS as usize);
    let (status, attempts, _) = record(&pool).await;
    assert_eq!((status.as_str(), attempts), ("failed", webhooks::MAX_ATTEMPTS));
}

#[sqlx::test(migrations = false)]
async fn an_unreachable_receiver_is_a_failed_attempt(pool: PgPool) {
    let app = setup(&pool).await;
    // Bind and drop: nothing listens on this port any more.
    let port = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
    subscribe(&app, &format!("http://127.0.0.1:{port}/hook"), &["decision"]).await;
    decide(&app, "v1", false).await;
    assert_eq!(deliver(&pool, Utc::now()).await.retrying, 1);
    let (status, attempts, _) = record(&pool).await;
    assert_eq!((status.as_str(), attempts), ("pending", 1));
}

#[sqlx::test(migrations = false)]
async fn a_delivered_event_is_not_sent_again(pool: PgPool) {
    let app = setup(&pool).await;
    let (rx, url) = receiver(&[]).await;
    subscribe(&app, &url, &["decision"]).await;
    decide(&app, "v1", false).await;
    deliver(&pool, Utc::now()).await;
    deliver(&pool, Utc::now() + Duration::hours(3)).await;
    assert_eq!(rx.seen().len(), 1);
}

// ---- delivery records ----

#[sqlx::test(migrations = false)]
async fn delivery_records_show_attempts_and_the_last_response(pool: PgPool) {
    let app = setup(&pool).await;
    let (_rx, url) = receiver(&[502]).await;
    let (id, _) = subscribe(&app, &url, &["decision"]).await;
    decide(&app, "v1", false).await;
    deliver(&pool, Utc::now()).await;
    let r = api_get(&app, &format!("/api/webhooks/{id}/deliveries")).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let d = r.json()["deliveries"][0].clone();
    assert_eq!(d["event"], "decision");
    assert_eq!(d["status"], "pending");
    assert_eq!(d["attempts"], 1);
    assert_eq!(d["last_status"], 502);
    let l = api_get(&app, "/api/sites/demo/webhooks").await.json();
    assert_eq!(l["subscriptions"][0]["counts"]["pending"], 1, "{l}");
}
