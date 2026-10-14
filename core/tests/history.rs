//! Visitor history (F3, backend.md "Visitor history"): facts from the visitor's own stored orders
//! and item events, as learner features (`feature:history.*`) and in audience rules, only for an
//! identified request on a site whose `fields_allowed` admits `history`.
//!
//! The fixture is the spec's exemplar: Asha ordered twice (600 in all, last 41 days ago), viewed a
//! filter yesterday and left it in her cart. Bea ordered once, 3 days ago.

mod common;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

fn site(mode: &str, extra: Value) -> Value {
    let mut v = json!({
        "site": "demo", "origin": "http://localhost:5173", "privacy_mode": mode,
        "slots": [
            { "key": "hero", "selector": "#hero",
              "variants": [ { "key": "baseline", "baseline": true }, { "key": "gift", "html": "<h1>Come back</h1>" } ] },
            { "key": "promo", "selector": "#promo",
              "variants": [ { "key": "baseline", "baseline": true }, { "key": "winback", "html": "<p>10% off</p>" } ] }
        ]
    });
    v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    v
}

fn block() -> Value {
    json!({ "history": { "views": [ { "key": "filters", "category": "filters", "days": 30 } ] } })
}

async fn setup(pool: &PgPool, mode: &str, extra: Value) {
    pratibimba::db::migrate(pool).await.unwrap();
    pratibimba::site_file::apply(pool, &file_from(site(mode, extra))).await.unwrap();
    set_holdout(pool, 0.0).await;
}

/// One stored event for `subject`, `days` ago.
async fn event(pool: &PgPool, subject: &str, name: &str, days: f64, order: Option<(&str, f64)>, items: Option<Value>) {
    sqlx::query(
        "INSERT INTO events (id, site_id, subject, name, received_at, order_id, value, currency, items)
         VALUES ($1, 'demo', $2, $3, now() - make_interval(secs => $4), $5, $6::numeric, CASE WHEN $6 IS NULL THEN NULL ELSE 'USD' END, $7)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(subject)
    .bind(name)
    .bind(days * 86400.0)
    .bind(order.map(|o| o.0))
    .bind(order.map(|o| o.1))
    .bind(items)
    .execute(pool)
    .await
    .unwrap();
}

async fn asha(pool: &PgPool, subject: &str) {
    event(pool, subject, "purchase", 80.0, Some(("o1", 200.0)), Some(json!([{ "id": "kaapi" }]))).await;
    event(pool, subject, "purchase", 41.5, Some(("o2", 400.0)), Some(json!([{ "id": "kaapi" }]))).await;
    event(pool, subject, "click", 1.2, None, Some(json!([{ "id": "steel-filter", "category": "filters" }]))).await;
    event(pool, subject, "add_to_cart", 1.1, None, Some(json!([{ "id": "steel-filter", "category": "filters" }]))).await;
}

async fn decide(pool: &PgPool, slot: &str, subject: &str, consent: bool) -> Value {
    let r = post(&app(pool), "/decide", json!({ "site": "demo", "slot": slot, "subject": subject, "ctx": { "consent": consent } })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json()
}

fn history_features(d: &Value) -> Value {
    let f = d["explanation"]["features"].as_object().unwrap();
    Value::Object(f.iter().filter(|(k, _)| k.starts_with("feature:history.")).map(|(k, v)| (k.clone(), v.clone())).collect())
}

#[sqlx::test(migrations = false)]
async fn a_full_mode_visitor_carries_her_history_into_the_learner_and_the_explanation(pool: PgPool) {
    setup(&pool, "full", block()).await;
    asha(&pool, "asha").await;
    let d = decide(&pool, "hero", "asha", false).await;
    assert_eq!(
        history_features(&d),
        json!({ "feature:history.days_since_last_purchase": 41.0 / 365.0, "feature:history.purchase_count": 0.2,
                "feature:history.lifetime_value": 2.0 / 3.0, "feature:history.cart_abandoned": 1.0,
                "feature:history.viewed.filters": 1.0 }),
        "{d}"
    );
    assert_eq!(d["explanation"]["policy"], json!("lints_features"), "{d}");
    // The decision as stored (what the detail page shows and the worker learns from) has them too.
    let stored: Value = sqlx::query_scalar("SELECT features FROM decisions").fetch_one(&pool).await.unwrap();
    assert_eq!(stored["feature:history.cart_abandoned"], json!(1.0), "{stored}");
}

#[sqlx::test(migrations = false)]
async fn a_cookieless_visitor_has_no_history_even_from_her_own_days_events(pool: PgPool) {
    setup(&pool, "cookieless", block()).await;
    let first = decide(&pool, "hero", "asha", true).await;
    let pseudonym: String = sqlx::query_scalar("SELECT subject FROM decisions").fetch_one(&pool).await.unwrap();
    asha(&pool, &pseudonym).await;
    let d = decide(&pool, "promo", "asha", true).await;
    assert_eq!(history_features(&first), json!({}));
    assert_eq!(history_features(&d), json!({}), "a cookieless decide read the pseudonym's events: {d}");
}

#[sqlx::test(migrations = false)]
async fn consent_aware_reads_history_only_with_consent(pool: PgPool) {
    setup(&pool, "consent_aware", block()).await;
    asha(&pool, "asha").await;
    let without = decide(&pool, "hero", "asha", false).await;
    let with = decide(&pool, "promo", "asha", true).await;
    assert_eq!(history_features(&without), json!({}), "{without}");
    assert_eq!(history_features(&with)["feature:history.purchase_count"], json!(0.2), "{with}");
}

#[sqlx::test(migrations = false)]
async fn fields_allowed_without_history_reads_none(pool: PgPool) {
    let mut extra = block();
    extra["fields_allowed"] = json!(["mobile"]);
    setup(&pool, "full", extra).await;
    asha(&pool, "asha").await;
    let d = decide(&pool, "hero", "asha", false).await;
    assert_eq!(history_features(&d), json!({}), "{d}");
}

#[sqlx::test(migrations = false)]
async fn a_disabled_block_adds_no_learner_features(pool: PgPool) {
    setup(&pool, "full", json!({ "history": { "enabled": false } })).await;
    asha(&pool, "asha").await;
    let d = decide(&pool, "hero", "asha", false).await;
    assert_eq!(history_features(&d), json!({}), "{d}");
}

/// The hero's gift is gated on "last order 30+ days ago", with no `history:` block on the site.
async fn lapsed_gate(pool: &PgPool) -> i64 {
    let admin = admin_app(pool);
    let r = api_post(&admin, "/api/sites/demo/segments", json!({ "predicate": { "days_since_last_purchase": { "gte": 30 } }, "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    let aud = r.json()["id"].as_i64().unwrap();
    let r = api_post(&admin, &format!("/api/segments/{aud}/promote"), json!({ "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let r = api_post(&admin, &format!("/api/variants/{}/audience", variant_id(pool, "gift").await), json!({ "audience": aud, "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    aud
}

fn matched(d: &Value) -> Value {
    d["explanation"]["audiences"][0]["matched"].clone()
}

#[sqlx::test(migrations = false)]
async fn an_audience_rule_reads_last_order_days_without_a_history_block(pool: PgPool) {
    setup(&pool, "full", json!({})).await;
    lapsed_gate(&pool).await;
    asha(&pool, "asha").await;
    event(&pool, "bea", "purchase", 3.0, Some(("o3", 50.0)), None).await;
    let a = decide(&pool, "hero", "asha", false).await;
    let b = decide(&pool, "hero", "bea", false).await;
    let c = decide(&pool, "hero", "cleo", false).await;
    assert_eq!(matched(&a), json!(true), "41 days since Asha's last order: {a}");
    assert_eq!(matched(&b), json!(false), "Bea ordered 3 days ago: {b}");
    assert_eq!(matched(&c), json!(false), "Cleo never ordered: {c}");
    // No block: the rule reads history, the learner does not.
    assert_eq!(history_features(&a), json!({}), "{a}");
}

#[sqlx::test(migrations = false)]
async fn a_history_audience_matches_nobody_in_cookieless_mode(pool: PgPool) {
    setup(&pool, "cookieless", json!({})).await;
    lapsed_gate(&pool).await;
    decide(&pool, "promo", "asha", true).await;
    let pseudonym: String = sqlx::query_scalar("SELECT subject FROM decisions").fetch_one(&pool).await.unwrap();
    asha(&pool, &pseudonym).await;
    let d = decide(&pool, "hero", "asha", true).await;
    assert_eq!(matched(&d), json!(false), "{d}");
}
