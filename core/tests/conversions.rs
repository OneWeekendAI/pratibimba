//! Inbound conversion sources: the Stripe webhook and the generic signed webhook. Platform
//! conversions are authoritative: a browser event for the same goal is only a hint.

mod common;

use axum::{body::Body, http::Request};
use chrono::{Duration, Utc};
use common::*;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::PgPool;

const STRIPE_SECRET: &str = "whsec_test_5f2b8e1c";
const CONV_SECRET: &str = "prb_conv_test_secret";

/// Stripe's documented scheme, written out independently of the code under test:
/// `Stripe-Signature: t=<unix>,v1=hex(HMAC-SHA256(secret, "<t>.<raw body>"))`.
fn sign(secret: &str, t: i64, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{t}.{body}").as_bytes());
    mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

async fn webhook(app: &axum::Router, path: &str, header: &str, value: String, body: &str) -> Resp {
    send(app, Request::post(path).header(header, value).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap()).await
}

async fn stripe(app: &axum::Router, body: &str) -> Resp {
    let t = Utc::now().timestamp();
    webhook(app, "/webhook/stripe/demo", "stripe-signature", format!("t={t},v1={}", sign(STRIPE_SECRET, t, body)), body).await
}

async fn generic(app: &axum::Router, body: &Value) -> Resp {
    let body = body.to_string();
    let t = Utc::now().timestamp();
    webhook(app, "/webhook/conversion/demo", "pratibimba-signature", format!("t={t},v1={}", sign(CONV_SECRET, t, &body)), &body).await
}

fn checkout(event_id: &str, session: &str, subject: &str, decision: Option<&str>) -> String {
    let mut metadata = json!({});
    if let Some(d) = decision {
        metadata["prb_decision_id"] = json!(d);
    }
    json!({
        "id": event_id, "object": "event", "type": "checkout.session.completed", "created": Utc::now().timestamp(),
        "data": { "object": { "id": session, "object": "checkout.session", "client_reference_id": subject,
                              "payment_status": "paid", "amount_total": 4900, "currency": "usd", "metadata": metadata } }
    })
    .to_string()
}

/// Demo site whose hero slot measures `purchase`, with both sources configured and `purchase`
/// declared platform-authoritative. Decisions are not sticky, so one subject can have several.
async fn setup(pool: &PgPool) -> axum::Router {
    setup_learning(pool).await;
    sqlx::query("UPDATE slots SET metric = 'purchase', decision_ttl_secs = 0").execute(pool).await.unwrap();
    let app = admin_app(pool);
    let r = api_post(&app, "/api/sites/demo/conversion-sources", json!({
        "stripe_signing_secret": STRIPE_SECRET, "conversion_signing_secret": CONV_SECRET, "goals": ["purchase"]
    }))
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    app
}

/// A decision for `subject`, exposed.
async fn exposed(app: &axum::Router, subject: &str) -> String {
    let d = decide(app, subject, false).await.json();
    let id = d["decision_id"].as_str().unwrap().to_string();
    assert_eq!(event(app, subject, "exposure", Some(&id)).await.status, StatusCode::ACCEPTED);
    id
}

async fn outcome(pool: &PgPool, id: &str) -> Option<i16> {
    sqlx::query_scalar("SELECT outcome FROM decisions WHERE id = $1::uuid").bind(id).fetch_one(pool).await.unwrap()
}

async fn model_version(pool: &PgPool) -> i64 {
    count(pool, "SELECT version FROM models").await
}

#[sqlx::test(migrations = false)]
async fn a_signed_stripe_checkout_credits_the_decision(pool: PgPool) {
    let app = setup(&pool).await;
    let d = exposed(&app, "s1").await;
    let r = stripe(&app, &checkout("evt_1", "cs_1", "s1", Some(&d))).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    pratibimba::worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(outcome(&pool, &d).await, Some(1));
    let (source, ext): (String, String) = sqlx::query_as("SELECT source, external_id FROM events WHERE name = 'purchase'").fetch_one(&pool).await.unwrap();
    assert_eq!((source.as_str(), ext.as_str()), ("stripe", "cs_1"));
}

#[sqlx::test(migrations = false)]
async fn a_bad_stripe_signature_is_rejected(pool: PgPool) {
    let app = setup(&pool).await;
    let d = exposed(&app, "s1").await;
    let body = checkout("evt_1", "cs_1", "s1", Some(&d));
    let t = Utc::now().timestamp();
    let r = webhook(&app, "/webhook/stripe/demo", "stripe-signature", format!("t={t},v1={}", sign("whsec_wrong", t, &body)), &body).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE name = 'purchase'").await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_tampered_stripe_body_is_rejected(pool: PgPool) {
    let app = setup(&pool).await;
    let body = checkout("evt_1", "cs_1", "s1", None);
    let t = Utc::now().timestamp();
    let header = format!("t={t},v1={}", sign(STRIPE_SECRET, t, &body));
    let r = webhook(&app, "/webhook/stripe/demo", "stripe-signature", header, &body.replace("s1", "s2")).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = false)]
async fn a_replayed_stripe_signature_outside_tolerance_is_rejected(pool: PgPool) {
    let app = setup(&pool).await;
    let body = checkout("evt_1", "cs_1", "s1", None);
    let t = Utc::now().timestamp() - 600;
    let r = webhook(&app, "/webhook/stripe/demo", "stripe-signature", format!("t={t},v1={}", sign(STRIPE_SECRET, t, &body)), &body).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = false)]
async fn a_missing_stripe_signature_or_secret_is_rejected(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    // No secret configured for the site: nothing can verify, so nothing is accepted.
    let r = stripe(&app, &checkout("evt_1", "cs_1", "s1", None)).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let app = setup(&pool).await;
    let body = checkout("evt_1", "cs_1", "s1", None);
    let r = send(&app, Request::post("/webhook/stripe/demo").body(Body::from(body)).unwrap()).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = false)]
async fn unpaid_checkouts_and_other_stripe_events_are_acknowledged_not_counted(pool: PgPool) {
    let app = setup(&pool).await;
    let unpaid = checkout("evt_1", "cs_1", "s1", None).replace("\"paid\"", "\"unpaid\"");
    assert_eq!(stripe(&app, &unpaid).await.status, StatusCode::OK);
    let other = checkout("evt_2", "cs_2", "s1", None).replace("checkout.session.completed", "customer.created");
    assert_eq!(stripe(&app, &other).await.status, StatusCode::OK);
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE name = 'purchase'").await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_duplicate_stripe_conversion_is_counted_once(pool: PgPool) {
    let app = setup(&pool).await;
    let d = exposed(&app, "s1").await;
    let body = checkout("evt_1", "cs_1", "s1", Some(&d));
    assert_eq!(stripe(&app, &body).await.json()["duplicate"], false);
    // Stripe retries the same event, and the same checkout also arrives as a second event.
    assert_eq!(stripe(&app, &body).await.json()["duplicate"], true);
    let again = checkout("evt_2", "cs_1", "s1", Some(&d)).replace("checkout.session.completed", "checkout.session.async_payment_succeeded");
    assert_eq!(stripe(&app, &again).await.json()["duplicate"], true);
    let v0 = model_version(&pool).await;
    pratibimba::worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE name = 'purchase'").await, 1);
    assert_eq!(model_version(&pool).await, v0 + 1);
}

#[sqlx::test(migrations = false)]
async fn a_duplicate_generic_conversion_is_counted_once(pool: PgPool) {
    let app = setup(&pool).await;
    exposed(&app, "s1").await;
    let body = json!({ "id": "order-77", "subject": "s1", "event": "purchase" });
    let r = generic(&app, &body).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.json()["duplicate"], false);
    assert_eq!(generic(&app, &body).await.json()["duplicate"], true);
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE name = 'purchase'").await, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM pending_events p JOIN events e ON e.id = p.event_id WHERE e.name = 'purchase'").await, 1);
}

#[sqlx::test(migrations = false)]
async fn a_bad_generic_signature_is_rejected(pool: PgPool) {
    let app = setup(&pool).await;
    let body = json!({ "id": "order-1", "subject": "s1" }).to_string();
    let t = Utc::now().timestamp();
    let r = webhook(&app, "/webhook/conversion/demo", "pratibimba-signature", format!("t={t},v1={}", sign(STRIPE_SECRET, t, &body)), &body).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(count(&pool, "SELECT count(*) FROM events").await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_generic_conversion_needs_an_id_and_a_valid_subject(pool: PgPool) {
    let app = setup(&pool).await;
    assert_eq!(generic(&app, &json!({ "subject": "s1" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(generic(&app, &json!({ "id": "o1", "subject": "" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(generic(&app, &json!({ "id": "o1", "subject": "s1", "event": "Bad Name" })).await.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = false)]
async fn a_browser_purchase_is_only_a_hint(pool: PgPool) {
    let app = setup(&pool).await;
    let d = exposed(&app, "s1").await;
    event(&app, "s1", "purchase", Some(&d)).await;
    pratibimba::worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(outcome(&pool, &d).await, None, "a browser purchase credited an authoritative goal");
    // Window (30 min) plus grace (1 h) over and no platform conversion: a miss.
    pratibimba::worker::run_once(&pool, Utc::now() + Duration::hours(2)).await.unwrap();
    assert_eq!(outcome(&pool, &d).await, Some(0));
}

#[sqlx::test(migrations = false)]
async fn a_browser_goal_that_is_not_authoritative_still_counts(pool: PgPool) {
    let app = setup(&pool).await;
    sqlx::query("UPDATE sites SET authoritative_goals = '{}'").execute(&pool).await.unwrap();
    let d = exposed(&app, "s1").await;
    event(&app, "s1", "purchase", Some(&d)).await;
    pratibimba::worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(outcome(&pool, &d).await, Some(1));
}

#[sqlx::test(migrations = false)]
async fn a_platform_conversion_overrides_the_browser_event(pool: PgPool) {
    let app = setup(&pool).await;
    let a = exposed(&app, "s1").await;
    let b = exposed(&app, "s1").await;
    // The browser claims decision A converted; Stripe says the order came from decision B.
    event(&app, "s1", "purchase", Some(&a)).await;
    stripe(&app, &checkout("evt_1", "cs_1", "s1", Some(&b))).await;
    pratibimba::worker::run_once(&pool, Utc::now()).await.unwrap();
    pratibimba::worker::run_once(&pool, Utc::now() + Duration::hours(2)).await.unwrap();
    assert_eq!(outcome(&pool, &b).await, Some(1));
    assert_eq!(outcome(&pool, &a).await, Some(0));
}

#[sqlx::test(migrations = false)]
async fn a_platform_conversion_without_a_decision_follows_the_browser_hint(pool: PgPool) {
    let app = setup(&pool).await;
    let a = exposed(&app, "s1").await;
    let b = exposed(&app, "s1").await;
    // Last-touch would pick B; the browser saw the purchase happen on A's page.
    event(&app, "s1", "purchase", Some(&a)).await;
    generic(&app, &json!({ "id": "order-1", "subject": "s1", "event": "purchase" })).await;
    pratibimba::worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(outcome(&pool, &a).await, Some(1));
    assert_eq!(outcome(&pool, &b).await, None);
}

#[sqlx::test(migrations = false)]
async fn a_late_platform_conversion_inside_the_grace_period_still_counts(pool: PgPool) {
    let app = setup(&pool).await;
    let d = exposed(&app, "s1").await;
    // The window has closed, but the grace period has not: the decision must wait.
    pratibimba::worker::run_once(&pool, Utc::now() + Duration::minutes(40)).await.unwrap();
    assert_eq!(outcome(&pool, &d).await, None);
    let occurred = Utc::now().to_rfc3339();
    generic(&app, &json!({ "id": "order-1", "subject": "s1", "event": "purchase", "decision_id": d, "occurred_at": occurred })).await;
    pratibimba::worker::run_once(&pool, Utc::now() + Duration::minutes(41)).await.unwrap();
    assert_eq!(outcome(&pool, &d).await, Some(1));
}

#[sqlx::test(migrations = false)]
async fn conversion_sources_never_echo_their_secrets(pool: PgPool) {
    let app = setup(&pool).await;
    let r = api_get(&app, "/api/sites/demo/conversion-sources").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert!(!r.text.contains(STRIPE_SECRET) && !r.text.contains(CONV_SECRET), "{}", r.text);
    assert_eq!(r.json()["stripe"], true);
    assert_eq!(r.json()["goals"], json!(["purchase"]));
    assert_eq!(post(&app, "/api/sites/demo/conversion-sources", json!({})).await.status, StatusCode::UNAUTHORIZED);
}
