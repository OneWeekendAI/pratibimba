//! Declared features and the PII tag (backend.md, "PII features are opt-in and tagged"). A site
//! file declares each feature the client may send in `ctx.features`, and tags it `pii: true` or
//! `pii: false`. A PII feature is dropped from the context in `cookieless` mode, and in
//! `consent_aware` mode without consent; it is never stored on such a decision.
//!
//! The fixture: `corp_email` (the visitor signed in with a company email domain) is PII;
//! `loyalty_member` is not.

mod common;
use chrono::{Duration, Utc};
use common::*;
use pratibimba::worker;
use serde_json::{Value, json};
use sqlx::PgPool;

fn site(mode: &str, features: Value) -> Value {
    json!({
        "site": "demo", "origin": "http://localhost:5173", "privacy_mode": mode, "features": features,
        "slots": [{ "key": "hero", "selector": "#hero",
                    "variants": [ { "key": "baseline", "baseline": true }, { "key": "gift", "html": "<h1>Gift sets</h1>" } ] }]
    })
}

fn fixture() -> Value {
    json!([{ "key": "corp_email", "pii": true }, { "key": "loyalty_member", "pii": false }])
}

async fn setup(pool: &PgPool, mode: &str) {
    pratibimba::db::migrate(pool).await.unwrap();
    pratibimba::site_file::apply(pool, &file_from(site(mode, fixture()))).await.unwrap();
    set_holdout(pool, 0.0).await;
}

async fn decide_with(pool: &PgPool, consent: bool, features: Value) -> Resp {
    post(&app(pool), "/decide", json!({ "site": "demo", "slot": "hero", "subject": "v1",
        "ctx": { "consent": consent, "features": features } })).await
}

fn feature(d: &Value, key: &str) -> Value {
    d["explanation"]["features"][format!("feature:{key}")].clone()
}

async fn stored_features(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT features FROM decisions").fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn pii_feature_is_used_with_consent_in_consent_aware_mode(pool: PgPool) {
    setup(&pool, "consent_aware").await;
    let d = decide_with(&pool, true, json!({ "corp_email": true })).await.json();
    assert_eq!(feature(&d, "corp_email"), json!(1.0), "{d}");
}

#[sqlx::test(migrations = false)]
async fn pii_feature_is_dropped_without_consent_in_consent_aware_mode(pool: PgPool) {
    setup(&pool, "consent_aware").await;
    let d = decide_with(&pool, false, json!({ "corp_email": true })).await.json();
    assert_eq!(feature(&d, "corp_email"), Value::Null, "{d}");
    assert!(stored_features(&pool).await.get("feature:corp_email").is_none());
}

#[sqlx::test(migrations = false)]
async fn pii_feature_is_dropped_in_cookieless_mode_even_with_consent(pool: PgPool) {
    setup(&pool, "cookieless").await;
    let d = decide_with(&pool, true, json!({ "corp_email": true })).await.json();
    assert_eq!(feature(&d, "corp_email"), Value::Null, "{d}");
    assert!(stored_features(&pool).await.get("feature:corp_email").is_none());
}

#[sqlx::test(migrations = false)]
async fn pii_feature_is_used_in_full_mode(pool: PgPool) {
    setup(&pool, "full").await;
    let d = decide_with(&pool, false, json!({ "corp_email": true })).await.json();
    assert_eq!(feature(&d, "corp_email"), json!(1.0), "{d}");
}

#[sqlx::test(migrations = false)]
async fn non_pii_feature_is_kept_in_cookieless_mode(pool: PgPool) {
    setup(&pool, "cookieless").await;
    let d = decide_with(&pool, false, json!({ "loyalty_member": true })).await.json();
    assert_eq!(feature(&d, "loyalty_member"), json!(1.0), "{d}");
}

#[sqlx::test(migrations = false)]
async fn declared_feature_the_client_did_not_send_is_zero(pool: PgPool) {
    setup(&pool, "full").await;
    let d = decide_with(&pool, false, json!({})).await.json();
    assert_eq!(feature(&d, "loyalty_member"), json!(0.0), "{d}");
}

#[sqlx::test(migrations = false)]
async fn undeclared_feature_is_refused(pool: PgPool) {
    setup(&pool, "full").await;
    let r = decide_with(&pool, false, json!({ "age_band": true })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    assert!(r.text.contains("age_band"), "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn declared_features_are_learned(pool: PgPool) {
    setup(&pool, "full").await;
    let app = app(&pool);
    for i in 0..5 {
        let subject = format!("v{i}");
        let d = post(&app, "/decide", json!({ "site": "demo", "slot": "hero", "subject": subject, "ctx": { "features": { "loyalty_member": true } } }))
            .await
            .json();
        let id = d["decision_id"].as_str().unwrap().to_string();
        event(&app, &subject, "exposure", Some(&id)).await;
        event(&app, &subject, "click", Some(&id)).await;
    }
    worker::run_once(&pool, Utc::now() + Duration::hours(2)).await.unwrap();
    let keys: Value = sqlx::query_scalar("SELECT state->'stated'->'keys' FROM models").fetch_one(&pool).await.unwrap();
    assert!(keys.as_array().is_some_and(|k| k.contains(&json!("feature:loyalty_member"))), "{keys}");
}

#[sqlx::test(migrations = false)]
async fn decision_with_a_declared_feature_replays(pool: PgPool) {
    setup(&pool, "full").await;
    let d = decide_with(&pool, false, json!({ "loyalty_member": true })).await.json();
    let id = d["decision_id"].as_str().unwrap().parse().unwrap();
    let detail = pratibimba::explain::detail(&pool, id, Utc::now()).await.unwrap().unwrap();
    assert_eq!(detail.replay_matches, Some(true), "policy {}", detail.policy);
}

// --- the site file ------------------------------------------------------------------------------

fn validate(features: Value) -> Result<(), String> {
    let f: pratibimba::site_file::SiteFile = serde_json::from_value(site("full", features)).map_err(|e| e.to_string())?;
    f.validate().map_err(|e| e.to_string())
}

#[test]
fn feature_fixture_is_accepted() {
    validate(fixture()).unwrap();
}

#[test]
fn feature_without_a_pii_tag_is_refused() {
    // The owner must say which it is; there is no default to forget.
    let e = validate(json!([{ "key": "corp_email" }])).unwrap_err();
    assert!(e.contains("pii"), "{e}");
}

#[test]
fn feature_that_names_a_demographic_is_refused() {
    let e = validate(json!([{ "key": "gender", "pii": true }])).unwrap_err();
    assert!(e.contains("gender"), "{e}");
}

#[test]
fn feature_key_that_is_not_a_slug_is_refused() {
    let e = validate(json!([{ "key": "Corp Email", "pii": true }])).unwrap_err();
    assert!(e.contains("Corp Email"), "{e}");
}

#[test]
fn feature_declared_twice_is_refused() {
    let e = validate(json!([{ "key": "corp_email", "pii": true }, { "key": "corp_email", "pii": false }])).unwrap_err();
    assert!(e.contains("corp_email"), "{e}");
}

#[sqlx::test(migrations = false)]
async fn features_are_replaced_when_the_site_file_lists_them_again(pool: PgPool) {
    setup(&pool, "full").await;
    pratibimba::site_file::apply(&pool, &file_from(site("full", json!([{ "key": "corp_email", "pii": false }])))).await.unwrap();
    let rows: Vec<(String, bool)> = sqlx::query_as("SELECT key, pii FROM site_features ORDER BY key").fetch_all(&pool).await.unwrap();
    assert_eq!(rows, vec![("corp_email".to_string(), false)]);
}

#[sqlx::test(migrations = false)]
async fn features_are_kept_when_the_site_file_omits_them(pool: PgPool) {
    setup(&pool, "full").await;
    let mut f = site("full", Value::Null);
    f.as_object_mut().unwrap().remove("features");
    pratibimba::site_file::apply(&pool, &file_from(f)).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM site_features").await, 2);
}

// The manifest lists the declared features with their tags, so the SDK sends only declared
// names and keeps a PII one on the device unless the visitor is identified.
#[sqlx::test(migrations = false)]
async fn manifest_lists_declared_features_with_their_pii_tag(pool: PgPool) {
    setup(&pool, "consent_aware").await;
    let m = get(&app(&pool), "/manifest/demo").await.json();
    assert_eq!(m["features"], fixture());
}

#[sqlx::test(migrations = false)]
async fn manifest_without_declared_features_has_no_features_field(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    pratibimba::site_file::apply(&pool, &file_from(site("full", json!([])))).await.unwrap();
    let m = get(&app(&pool), "/manifest/demo").await.json();
    assert!(m.get("features").is_none());
}
