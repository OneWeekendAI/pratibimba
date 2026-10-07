//! Privacy: consent modes (what is stored and what a decision may use under each), the
//! per-site `fields_allowed` whitelist at /decide and /e, the visitors summary, right-to-erase
//! with its tombstone, and the right-of-access listing.

mod common;
use chrono::{Duration, Utc};
use common::*;
use pratibimba::{insights::rollup, privacy, worker};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::time::Duration as StdDuration;

/// The demo site in `mode`, with no holdout so every exposed decision teaches the model.
async fn setup(pool: &PgPool, mode: &str) {
    pratibimba::db::migrate(pool).await.unwrap();
    let mut f = demo_file();
    f.privacy_mode = Some(mode.into());
    pratibimba::site_file::apply(pool, &f).await.unwrap();
    set_holdout(pool, 0.0).await;
}

async fn set_fields(pool: &PgPool, fields: &[&str]) {
    let f: Vec<String> = fields.iter().map(|s| s.to_string()).collect();
    sqlx::query("UPDATE sites SET fields_allowed = $1 WHERE id = 'demo'").bind(f).execute(pool).await.unwrap();
}

async fn decide_ctx(app: &axum::Router, subject: &str, ctx: Value) -> Resp {
    post(app, "/decide", json!({ "site": "demo", "slot": "hero", "subject": subject, "ctx": ctx })).await
}

async fn decided(app: &axum::Router, subject: &str, ctx: Value) -> Value {
    let r = decide_ctx(app, subject, ctx).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json()
}

async fn events(app: &axum::Router, subject: &str, body: Value) -> Resp {
    let mut b = json!({ "site": "demo", "subject": subject });
    for (k, v) in body.as_object().unwrap() {
        b[k] = v.clone();
    }
    post(app, "/e", b).await
}

fn ev(name: &str, decision: Option<&str>) -> Value {
    json!({ "id": uuid::Uuid::new_v4(), "name": name, "slot": "hero", "decision_id": decision })
}

async fn stored_subject(pool: &PgPool, decision: &str) -> String {
    sqlx::query_scalar("SELECT subject FROM decisions WHERE id = $1::uuid").bind(decision).fetch_one(pool).await.unwrap()
}

async fn event_subjects(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT subject FROM events ORDER BY received_at").fetch_all(pool).await.unwrap()
}

fn id_of(d: &Value) -> String {
    d["decision_id"].as_str().unwrap().to_string()
}

// ---- consent modes ----

#[sqlx::test(migrations = false)]
async fn full_mode_stores_the_subject_as_sent(pool: PgPool) {
    setup(&pool, "full").await;
    let d = decided(&app(&pool), "alice", json!({})).await;
    assert_eq!(stored_subject(&pool, &id_of(&d)).await, "alice");
}

#[sqlx::test(migrations = false)]
async fn cookieless_mode_stores_a_pseudonym(pool: PgPool) {
    setup(&pool, "cookieless").await;
    let d = decided(&app(&pool), "alice", json!({})).await;
    let s = stored_subject(&pool, &id_of(&d)).await;
    assert!(s.starts_with(privacy::PSEUDONYM) && !s.contains("alice"), "{s}");
}

#[sqlx::test(migrations = false)]
async fn cookieless_decision_and_events_share_the_pseudonym(pool: PgPool) {
    setup(&pool, "cookieless").await;
    let app = app(&pool);
    let d = decided(&app, "alice", json!({})).await;
    let r = events(&app, "alice", json!({ "events": [ev("exposure", Some(&id_of(&d)))] })).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    assert_eq!(event_subjects(&pool).await, vec![stored_subject(&pool, &id_of(&d)).await]);
}

#[sqlx::test(migrations = false)]
async fn cookieless_conversion_credits_within_the_day(pool: PgPool) {
    setup(&pool, "cookieless").await;
    let app = app(&pool);
    let d = decided(&app, "alice", json!({})).await;
    events(&app, "alice", json!({ "events": [ev("exposure", Some(&id_of(&d))), ev("click", Some(&id_of(&d)))] })).await;
    let s = worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(s.credited, 1);
}

#[sqlx::test(migrations = false)]
async fn cookieless_pseudonym_changes_with_the_day(pool: PgPool) {
    setup(&pool, "cookieless").await;
    let today = Utc::now().date_naive();
    let a = privacy::salt(&pool, "demo", today).await.unwrap();
    let b = privacy::salt(&pool, "demo", today + Duration::days(1)).await.unwrap();
    assert_ne!(privacy::pseudonym(&a, "demo", "alice"), privacy::pseudonym(&b, "demo", "alice"));
}

#[sqlx::test(migrations = false)]
async fn cookieless_salt_is_stable_within_a_day(pool: PgPool) {
    setup(&pool, "cookieless").await;
    let today = Utc::now().date_naive();
    let a = privacy::salt(&pool, "demo", today).await.unwrap();
    assert_eq!(privacy::salt(&pool, "demo", today).await.unwrap(), a);
}

#[sqlx::test(migrations = false)]
async fn a_new_days_salt_deletes_the_old_one(pool: PgPool) {
    setup(&pool, "cookieless").await;
    let today = Utc::now().date_naive();
    privacy::salt(&pool, "demo", today).await.unwrap();
    privacy::salt(&pool, "demo", today + Duration::days(1)).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM privacy_salts").await, 1);
}

#[sqlx::test(migrations = false)]
async fn a_late_request_does_not_recreate_yesterdays_salt(pool: PgPool) {
    setup(&pool, "cookieless").await;
    let today = Utc::now().date_naive();
    let tomorrow = privacy::salt(&pool, "demo", today + Duration::days(1)).await.unwrap();
    assert_eq!(privacy::salt(&pool, "demo", today).await.unwrap(), tomorrow);
}

#[sqlx::test(migrations = false)]
async fn cookieless_drops_returning(pool: PgPool) {
    setup(&pool, "cookieless").await;
    let d = decided(&app(&pool), "alice", json!({ "returning": true })).await;
    assert_eq!(d["explanation"]["features"]["returning"], json!(0.0));
}

#[sqlx::test(migrations = false)]
async fn full_mode_keeps_returning(pool: PgPool) {
    setup(&pool, "full").await;
    let d = decided(&app(&pool), "alice", json!({ "returning": true })).await;
    assert_eq!(d["explanation"]["features"]["returning"], json!(1.0));
}

#[sqlx::test(migrations = false)]
async fn consent_aware_without_consent_stores_a_pseudonym(pool: PgPool) {
    setup(&pool, "consent_aware").await;
    let d = decided(&app(&pool), "alice", json!({ "returning": true })).await;
    assert!(stored_subject(&pool, &id_of(&d)).await.starts_with(privacy::PSEUDONYM));
}

#[sqlx::test(migrations = false)]
async fn consent_aware_without_consent_drops_returning(pool: PgPool) {
    setup(&pool, "consent_aware").await;
    let d = decided(&app(&pool), "alice", json!({ "returning": true })).await;
    assert_eq!(d["explanation"]["features"]["returning"], json!(0.0));
}

#[sqlx::test(migrations = false)]
async fn consent_aware_with_consent_stores_the_subject(pool: PgPool) {
    setup(&pool, "consent_aware").await;
    let d = decided(&app(&pool), "alice", json!({ "returning": true, "consent": true })).await;
    assert_eq!(stored_subject(&pool, &id_of(&d)).await, "alice");
    assert_eq!(d["explanation"]["features"]["returning"], json!(1.0));
}

#[sqlx::test(migrations = false)]
async fn consent_aware_events_with_consent_store_the_subject(pool: PgPool) {
    setup(&pool, "consent_aware").await;
    let r = events(&app(&pool), "alice", json!({ "consent": true, "events": [ev("view", None)] })).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    assert_eq!(event_subjects(&pool).await, vec!["alice".to_string()]);
}

#[sqlx::test(migrations = false)]
async fn consent_aware_events_without_consent_store_a_pseudonym(pool: PgPool) {
    setup(&pool, "consent_aware").await;
    events(&app(&pool), "alice", json!({ "events": [ev("view", None)] })).await;
    let s = event_subjects(&pool).await;
    assert!(s.len() == 1 && s[0].starts_with(privacy::PSEUDONYM), "{s:?}");
}

#[sqlx::test(migrations = false)]
async fn cookieless_ignores_a_consent_claim(pool: PgPool) {
    setup(&pool, "cookieless").await;
    let d = decided(&app(&pool), "alice", json!({ "consent": true })).await;
    assert!(stored_subject(&pool, &id_of(&d)).await.starts_with(privacy::PSEUDONYM));
}

#[sqlx::test(migrations = false)]
async fn a_new_site_defaults_to_consent_aware(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    let mut f = demo_file();
    f.privacy_mode = None;
    pratibimba::site_file::apply(&pool, &f).await.unwrap();
    let m: String = sqlx::query_scalar("SELECT privacy_mode FROM sites WHERE id = 'demo'").fetch_one(&pool).await.unwrap();
    assert_eq!(m, "consent_aware");
}

#[sqlx::test(migrations = false)]
async fn a_site_from_before_the_migration_keeps_full(pool: PgPool) {
    let all = pratibimba::db::MIGRATIONS;
    let privacy = all.iter().position(|m| m.0 == "0060_privacy").expect("the privacy migration");
    pratibimba::db::migrate_with(&pool, &all[..privacy]).await.unwrap();
    sqlx::query("INSERT INTO sites (id, origin) VALUES ('old', 'http://x')").execute(&pool).await.unwrap();
    pratibimba::db::migrate(&pool).await.unwrap();
    let m: String = sqlx::query_scalar("SELECT privacy_mode FROM sites WHERE id = 'old'").fetch_one(&pool).await.unwrap();
    assert_eq!(m, "full");
}

#[sqlx::test(migrations = false)]
async fn applying_a_file_without_a_mode_keeps_the_mode(pool: PgPool) {
    setup(&pool, "cookieless").await;
    let mut f = demo_file();
    f.privacy_mode = None;
    pratibimba::site_file::apply(&pool, &f).await.unwrap();
    let m: String = sqlx::query_scalar("SELECT privacy_mode FROM sites WHERE id = 'demo'").fetch_one(&pool).await.unwrap();
    assert_eq!(m, "cookieless");
}

#[sqlx::test(migrations = false)]
async fn apply_refuses_an_unknown_privacy_mode(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    let mut f = demo_file();
    f.privacy_mode = Some("anonymous".into());
    let e = pratibimba::site_file::apply(&pool, &f).await.expect_err("refused").to_string();
    assert!(e.contains("must be one of"), "{e}");
}

#[sqlx::test(migrations = false)]
async fn apply_refuses_an_unknown_field(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    let mut f = demo_file();
    f.fields_allowed = Some(vec!["gender".into()]);
    let e = pratibimba::site_file::apply(&pool, &f).await.expect_err("refused").to_string();
    assert!(e.contains("fields_allowed"), "{e}");
}

#[sqlx::test(migrations = false)]
async fn apply_sets_fields_allowed(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    let mut f = demo_file();
    f.fields_allowed = Some(vec!["mobile".into(), "slot".into()]);
    pratibimba::site_file::apply(&pool, &f).await.unwrap();
    let got: Option<Vec<String>> = sqlx::query_scalar("SELECT fields_allowed FROM sites WHERE id = 'demo'").fetch_one(&pool).await.unwrap();
    assert_eq!(got, Some(vec!["mobile".to_string(), "slot".to_string()]));
}

#[sqlx::test(migrations = false)]
async fn manifest_states_the_privacy_mode(pool: PgPool) {
    setup(&pool, "cookieless").await;
    let r = get(&app(&pool), "/manifest/demo").await;
    assert_eq!(r.json()["privacy_mode"], json!("cookieless"));
}

#[sqlx::test(migrations = false)]
async fn sandbox_copies_the_privacy_mode(pool: PgPool) {
    setup(&pool, "cookieless").await;
    pratibimba::sim::reset(&pool, "demo", Utc::now()).await.unwrap();
    let m: String = sqlx::query_scalar("SELECT privacy_mode FROM sites WHERE sandbox_of = 'demo'").fetch_one(&pool).await.unwrap();
    assert_eq!(m, "cookieless");
}

// ---- fields_allowed ----

#[sqlx::test(migrations = false)]
async fn decide_drops_a_field_the_site_does_not_collect(pool: PgPool) {
    setup(&pool, "full").await;
    set_fields(&pool, &["returning"]).await;
    let d = decided(&app(&pool), "alice", json!({ "mobile": true })).await;
    assert_eq!(d["explanation"]["features"]["mobile"], json!(0.0));
}

#[sqlx::test(migrations = false)]
async fn decide_keeps_a_field_the_site_collects(pool: PgPool) {
    setup(&pool, "full").await;
    set_fields(&pool, &["mobile"]).await;
    let d = decided(&app(&pool), "alice", json!({ "mobile": true })).await;
    assert_eq!(d["explanation"]["features"]["mobile"], json!(1.0));
}

#[sqlx::test(migrations = false)]
async fn decide_drops_an_uncollected_from_ad(pool: PgPool) {
    setup(&pool, "full").await;
    set_fields(&pool, &["mobile"]).await;
    let d = decided(&app(&pool), "alice", json!({ "from_ad": true })).await;
    assert_eq!(d["explanation"]["features"]["from_ad"], json!(0.0));
}

#[sqlx::test(migrations = false)]
async fn decide_drops_an_uncollected_returning(pool: PgPool) {
    setup(&pool, "full").await;
    set_fields(&pool, &["mobile"]).await;
    let d = decided(&app(&pool), "alice", json!({ "returning": true })).await;
    assert_eq!(d["explanation"]["features"]["returning"], json!(0.0));
}

#[sqlx::test(migrations = false)]
async fn decide_drops_an_uncollected_tz_offset(pool: PgPool) {
    setup(&pool, "full").await;
    set_fields(&pool, &["mobile"]).await;
    let d = decided(&app(&pool), "alice", json!({ "tz_offset": 330 })).await;
    assert!(d["explanation"]["features"].get("sin_hour").is_none(), "{}", d["explanation"]);
}

#[sqlx::test(migrations = false)]
async fn decide_drops_an_uncollected_tz_name(pool: PgPool) {
    setup(&pool, "full").await;
    set_fields(&pool, &["mobile"]).await;
    let d = decided(&app(&pool), "alice", json!({ "tz": "Asia/Kolkata" })).await;
    assert!(d["explanation"]["features"].get("sin_hour").is_none(), "{}", d["explanation"]);
}

#[sqlx::test(migrations = false)]
async fn decide_keeps_a_collected_tz_name(pool: PgPool) {
    setup(&pool, "full").await;
    set_fields(&pool, &["tz_offset"]).await;
    let d = decided(&app(&pool), "alice", json!({ "tz": "Asia/Kolkata" })).await;
    assert!(d["explanation"]["features"].get("sin_hour").is_some(), "{}", d["explanation"]);
}

#[sqlx::test(migrations = false)]
async fn decide_keeps_a_collected_tz_offset(pool: PgPool) {
    setup(&pool, "full").await;
    set_fields(&pool, &["tz_offset"]).await;
    let d = decided(&app(&pool), "alice", json!({ "tz_offset": 330 })).await;
    assert!(d["explanation"]["features"].get("sin_hour").is_some(), "{}", d["explanation"]);
}

#[sqlx::test(migrations = false)]
async fn decide_still_refuses_an_undeclared_field(pool: PgPool) {
    setup(&pool, "full").await;
    let r = decide_ctx(&app(&pool), "alice", json!({ "gender": "f" })).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn events_drop_an_uncollected_tz_offset(pool: PgPool) {
    setup(&pool, "full").await;
    set_fields(&pool, &["slot"]).await;
    events(&app(&pool), "alice", json!({ "tz_offset": 330, "events": [ev("view", None)] })).await;
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE tz_offset_min IS NULL").await, 1);
}

#[sqlx::test(migrations = false)]
async fn events_keep_a_collected_tz_offset(pool: PgPool) {
    setup(&pool, "full").await;
    set_fields(&pool, &["tz_offset"]).await;
    events(&app(&pool), "alice", json!({ "tz_offset": 330, "events": [ev("view", None)] })).await;
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE tz_offset_min = 330").await, 1);
}

#[sqlx::test(migrations = false)]
async fn events_drop_an_uncollected_decision_id(pool: PgPool) {
    setup(&pool, "full").await;
    set_fields(&pool, &["slot"]).await;
    let d = decided(&app(&pool), "alice", json!({})).await;
    events(&app(&pool), "alice", json!({ "events": [ev("exposure", Some(&id_of(&d)))] })).await;
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE decision_id IS NULL").await, 1);
}

#[sqlx::test(migrations = false)]
async fn events_keep_a_collected_decision_id(pool: PgPool) {
    setup(&pool, "full").await;
    set_fields(&pool, &["decision_id"]).await;
    let d = decided(&app(&pool), "alice", json!({})).await;
    events(&app(&pool), "alice", json!({ "events": [ev("exposure", Some(&id_of(&d)))] })).await;
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE decision_id IS NOT NULL").await, 1);
}

#[sqlx::test(migrations = false)]
async fn events_drop_an_uncollected_slot(pool: PgPool) {
    setup(&pool, "full").await;
    set_fields(&pool, &["decision_id"]).await;
    events(&app(&pool), "alice", json!({ "events": [ev("view", None)] })).await;
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE slot_id IS NULL").await, 1);
}

#[sqlx::test(migrations = false)]
async fn events_keep_a_collected_slot(pool: PgPool) {
    setup(&pool, "full").await;
    set_fields(&pool, &["slot"]).await;
    events(&app(&pool), "alice", json!({ "events": [ev("view", None)] })).await;
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE slot_id IS NOT NULL").await, 1);
}

#[sqlx::test(migrations = false)]
async fn no_list_collects_every_field(pool: PgPool) {
    setup(&pool, "full").await;
    let d = decided(&app(&pool), "alice", json!({ "mobile": true, "from_ad": true })).await;
    assert_eq!((d["explanation"]["features"]["mobile"].clone(), d["explanation"]["features"]["from_ad"].clone()), (json!(1.0), json!(1.0)));
}

// ---- visitors ----

#[sqlx::test(migrations = false)]
async fn events_create_a_visitor_row(pool: PgPool) {
    setup(&pool, "full").await;
    events(&app(&pool), "alice", json!({ "events": [ev("view", None), ev("click", None)] })).await;
    let (n, recent): (i64, Vec<String>) = sqlx::query_as("SELECT events, recent FROM visitors WHERE subject = 'alice'").fetch_one(&pool).await.unwrap();
    assert_eq!((n, recent), (2, vec!["view".to_string(), "click".to_string()]));
}

#[sqlx::test(migrations = false)]
async fn a_visitor_keeps_only_the_last_twenty_names(pool: PgPool) {
    setup(&pool, "full").await;
    let app = app(&pool);
    let first: Vec<Value> = (0..15).map(|i| ev(&format!("a{i}"), None)).collect();
    let second: Vec<Value> = (0..10).map(|i| ev(&format!("b{i}"), None)).collect();
    events(&app, "alice", json!({ "events": first })).await;
    events(&app, "alice", json!({ "events": second })).await;
    let (n, recent): (i64, Vec<String>) = sqlx::query_as("SELECT events, recent FROM visitors WHERE subject = 'alice'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 25);
    assert_eq!(recent.len(), 20);
    assert_eq!((recent[0].as_str(), recent[19].as_str()), ("a5", "b9"));
}

#[sqlx::test(migrations = false)]
async fn a_retried_batch_does_not_count_twice(pool: PgPool) {
    setup(&pool, "full").await;
    let app = app(&pool);
    let batch = json!({ "events": [ev("view", None)] });
    events(&app, "alice", batch.clone()).await;
    events(&app, "alice", batch).await;
    assert_eq!(count(&pool, "SELECT events FROM visitors WHERE subject = 'alice'").await, 1);
}

#[sqlx::test(migrations = false)]
async fn a_cookieless_visitor_row_holds_the_pseudonym(pool: PgPool) {
    setup(&pool, "cookieless").await;
    events(&app(&pool), "alice", json!({ "events": [ev("view", None)] })).await;
    assert_eq!(count(&pool, "SELECT count(*) FROM visitors WHERE subject LIKE 'p1\\_%'").await, 1);
}

// ---- erase ----

/// Alice and Bob each decide, see it and click; the worker credits both.
async fn two_visitors(pool: &PgPool) -> (String, String) {
    let app = app(pool);
    let mut ids = vec![];
    for who in ["alice", "bob"] {
        let d = decided(&app, who, json!({})).await;
        let id = id_of(&d);
        let r = events(&app, who, json!({ "tz_offset": 330, "events": [ev("exposure", Some(&id)), ev("click", Some(&id))] })).await;
        assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
        ids.push(id);
    }
    worker::run_once(pool, Utc::now()).await.unwrap();
    (ids[0].clone(), ids[1].clone())
}

async fn erase(pool: &PgPool, subject: &str) -> Value {
    privacy::erase(pool, "demo", subject, "owner@example.com", Some("request by email")).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn erase_deletes_the_subjects_events(pool: PgPool) {
    setup(&pool, "full").await;
    two_visitors(&pool).await;
    erase(&pool, "alice").await;
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE subject = 'alice'").await, 0);
}

#[sqlx::test(migrations = false)]
async fn erase_leaves_other_subjects_alone(pool: PgPool) {
    setup(&pool, "full").await;
    two_visitors(&pool).await;
    erase(&pool, "alice").await;
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE subject = 'bob'").await, 2);
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions WHERE subject = 'bob'").await, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM visitors WHERE subject = 'bob'").await, 1);
}

#[sqlx::test(migrations = false)]
async fn erase_severs_decisions_from_the_subject(pool: PgPool) {
    setup(&pool, "full").await;
    let (a, _) = two_visitors(&pool).await;
    erase(&pool, "alice").await;
    let s = stored_subject(&pool, &a).await;
    assert!(s.starts_with(privacy::ERASED) && !s.contains("alice"), "{s}");
}

#[sqlx::test(migrations = false)]
async fn erase_keeps_the_decisions_outcome(pool: PgPool) {
    setup(&pool, "full").await;
    let (a, _) = two_visitors(&pool).await;
    erase(&pool, "alice").await;
    let o: Option<i16> = sqlx::query_scalar("SELECT outcome FROM decisions WHERE id = $1::uuid").bind(&a).fetch_one(&pool).await.unwrap();
    assert_eq!(o, Some(1));
}

#[sqlx::test(migrations = false)]
async fn erase_files_one_visitors_decisions_under_one_token(pool: PgPool) {
    setup(&pool, "full").await;
    let app = app(&pool);
    let mut f = demo_file();
    f.slots.push(serde_json::from_value(json!({ "key": "nav", "selector": "#nav", "variants": [ { "key": "base", "baseline": true } ] })).unwrap());
    pratibimba::site_file::apply(&pool, &f).await.unwrap();
    decided(&app, "alice", json!({})).await;
    post(&app, "/decide", json!({ "site": "demo", "slot": "nav", "subject": "alice" })).await;
    erase(&pool, "alice").await;
    assert_eq!(count(&pool, "SELECT count(DISTINCT subject) FROM decisions WHERE subject LIKE 'erased:%'").await, 1);
}

#[sqlx::test(migrations = false)]
async fn erase_keeps_the_results(pool: PgPool) {
    setup(&pool, "full").await;
    two_visitors(&pool).await;
    let later = Utc::now() + Duration::days(1);
    let before = serde_json::to_value(pratibimba::results::site(&pool, "demo", pratibimba::results::ALL_TIME, later).await.unwrap()).unwrap();
    erase(&pool, "alice").await;
    let after = serde_json::to_value(pratibimba::results::site(&pool, "demo", pratibimba::results::ALL_TIME, later).await.unwrap()).unwrap();
    assert_eq!(before, after);
}

async fn model(pool: &PgPool) -> (i64, Value) {
    sqlx::query_as("SELECT version, state FROM models").fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn erase_keeps_what_the_model_learned(pool: PgPool) {
    setup(&pool, "full").await;
    two_visitors(&pool).await;
    let before = model(&pool).await;
    erase(&pool, "alice").await;
    assert_eq!(model(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn an_erased_open_decision_teaches_nothing(pool: PgPool) {
    setup(&pool, "full").await;
    let app = app(&pool);
    let d = decided(&app, "alice", json!({})).await;
    events(&app, "alice", json!({ "events": [ev("exposure", Some(&id_of(&d)))] })).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    let before = model(&pool).await;
    erase(&pool, "alice").await;
    worker::run_once(&pool, Utc::now() + Duration::hours(2)).await.unwrap();
    assert_eq!(model(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn an_open_decision_of_someone_else_still_teaches(pool: PgPool) {
    setup(&pool, "full").await;
    let app = app(&pool);
    let d = decided(&app, "bob", json!({})).await;
    events(&app, "bob", json!({ "events": [ev("exposure", Some(&id_of(&d)))] })).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    let before = model(&pool).await;
    erase(&pool, "alice").await;
    worker::run_once(&pool, Utc::now() + Duration::hours(2)).await.unwrap();
    assert_eq!(model(&pool).await.0, before.0 + 1);
}

#[sqlx::test(migrations = false)]
async fn erase_deletes_the_visitor_row(pool: PgPool) {
    setup(&pool, "full").await;
    two_visitors(&pool).await;
    erase(&pool, "alice").await;
    assert_eq!(count(&pool, "SELECT count(*) FROM visitors WHERE subject = 'alice'").await, 0);
}

#[sqlx::test(migrations = false)]
async fn erase_deletes_the_retention_weeks(pool: PgPool) {
    setup(&pool, "full").await;
    two_visitors(&pool).await;
    rollup::run(&pool, Utc::now()).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM insight_visitor_weeks WHERE subject = 'alice'").await, 1);
    erase(&pool, "alice").await;
    assert_eq!(count(&pool, "SELECT count(*) FROM insight_visitor_weeks WHERE subject = 'alice'").await, 0);
}

async fn rollups(pool: &PgPool) -> Value {
    let hourly: Vec<(i16, i64, String, i64, i64)> =
        sqlx::query_as("SELECT hour_local, segment_id, outcome, n, tz_known FROM rollup_hourly ORDER BY 1, 2, 3").fetch_all(pool).await.unwrap();
    let journey: Vec<(i64, bool, i64)> = sqlx::query_as("SELECT fingerprint, converted, visitors FROM rollup_journey ORDER BY 1, 2").fetch_all(pool).await.unwrap();
    json!({ "hourly": hourly, "journey": journey })
}

/// Force every hour and day to be rebuilt from decisions and events on the next run.
async fn rebuild(pool: &PgPool) {
    sqlx::query("DELETE FROM insight_jobs").execute(pool).await.unwrap();
    rollup::run(pool, Utc::now()).await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn a_rebuild_after_erase_keeps_the_rollups(pool: PgPool) {
    setup(&pool, "full").await;
    two_visitors(&pool).await;
    rollup::run(&pool, Utc::now()).await.unwrap();
    let before = rollups(&pool).await;
    erase(&pool, "alice").await;
    rebuild(&pool).await;
    assert_eq!(rollups(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn a_rebuild_after_erase_adds_no_visitor_week(pool: PgPool) {
    setup(&pool, "full").await;
    two_visitors(&pool).await;
    rollup::run(&pool, Utc::now()).await.unwrap();
    erase(&pool, "alice").await;
    rebuild(&pool).await;
    assert_eq!(count(&pool, "SELECT count(*) FROM insight_visitor_weeks").await, 1);
}

#[sqlx::test(migrations = false)]
async fn erase_scrubs_the_subject_from_webhook_payloads(pool: PgPool) {
    setup(&pool, "full").await;
    let admin = admin_app(&pool);
    let r = api_post(&admin, "/api/sites/demo/webhooks", json!({ "url": "http://127.0.0.1:9/hook", "events": ["decision", "exposure"] })).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    two_visitors(&pool).await;
    erase(&pool, "alice").await;
    assert_eq!(count(&pool, "SELECT count(*) FROM webhook_deliveries WHERE payload::text LIKE '%alice%'").await, 0);
    assert_eq!(count(&pool, "SELECT count(*) FROM webhook_deliveries WHERE payload->'data'->>'subject' = 'bob'").await, 2);
}

#[sqlx::test(migrations = false)]
async fn erase_writes_an_audit_row(pool: PgPool) {
    setup(&pool, "full").await;
    two_visitors(&pool).await;
    erase(&pool, "alice").await;
    let (actor, reason, counts): (String, Option<String>, Value) =
        sqlx::query_as("SELECT actor, reason, counts FROM erasures").fetch_one(&pool).await.unwrap();
    assert_eq!((actor.as_str(), reason.as_deref()), ("owner@example.com", Some("request by email")));
    assert_eq!((counts["events"].clone(), counts["decisions"].clone(), counts["visitors"].clone()), (json!(2), json!(1), json!(1)));
}

#[sqlx::test(migrations = false)]
async fn the_audit_row_does_not_hold_the_subject(pool: PgPool) {
    setup(&pool, "full").await;
    two_visitors(&pool).await;
    erase(&pool, "alice").await;
    assert_eq!(count(&pool, "SELECT count(*) FROM erasures WHERE erasures::text LIKE '%alice%'").await, 0);
    assert_eq!(count(&pool, "SELECT count(*) FROM erasures").await, 1);
}

#[sqlx::test(migrations = false)]
async fn erase_needs_a_named_actor(pool: PgPool) {
    setup(&pool, "full").await;
    let e = privacy::erase(&pool, "demo", "alice", "  ", None).await;
    assert!(matches!(e, Err(pratibimba::http::ApiError::BadRequest(_))), "{e:?}");
}

#[sqlx::test(migrations = false)]
async fn erase_of_an_unknown_site_is_not_found(pool: PgPool) {
    setup(&pool, "full").await;
    let e = privacy::erase(&pool, "nope", "alice", "owner", None).await;
    assert!(matches!(e, Err(pratibimba::http::ApiError::NotFound(_))), "{e:?}");
}

#[sqlx::test(migrations = false)]
async fn an_erased_subject_cannot_decide(pool: PgPool) {
    setup(&pool, "full").await;
    erase(&pool, "alice").await;
    let r = decide_ctx(&app(&pool), "alice", json!({})).await;
    assert_eq!(r.status, StatusCode::GONE, "{}", r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions").await, 0);
}

#[sqlx::test(migrations = false)]
async fn another_subject_still_decides_after_an_erase(pool: PgPool) {
    setup(&pool, "full").await;
    erase(&pool, "alice").await;
    let r = decide_ctx(&app(&pool), "bob", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn an_erased_subject_cannot_batch_decide(pool: PgPool) {
    setup(&pool, "full").await;
    erase(&pool, "alice").await;
    let r = post(&app(&pool), "/decide/batch", json!({ "site": "demo", "subject": "alice", "slots": ["hero"] })).await;
    assert_eq!(r.status, StatusCode::GONE, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn an_erased_subjects_events_are_refused(pool: PgPool) {
    setup(&pool, "full").await;
    erase(&pool, "alice").await;
    let r = events(&app(&pool), "alice", json!({ "events": [ev("view", None)] })).await;
    assert_eq!(r.status, StatusCode::GONE, "{}", r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM events").await + count(&pool, "SELECT count(*) FROM visitors").await, 0);
}

#[sqlx::test(migrations = false)]
async fn another_subjects_events_are_accepted_after_an_erase(pool: PgPool) {
    setup(&pool, "full").await;
    erase(&pool, "alice").await;
    let r = events(&app(&pool), "bob", json!({ "events": [ev("view", None)] })).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn an_erased_cookieless_subject_is_refused(pool: PgPool) {
    setup(&pool, "cookieless").await;
    erase(&pool, "alice").await;
    let r = decide_ctx(&app(&pool), "alice", json!({})).await;
    assert_eq!(r.status, StatusCode::GONE, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn erase_reaches_todays_pseudonym(pool: PgPool) {
    setup(&pool, "cookieless").await;
    two_visitors(&pool).await;
    erase(&pool, "alice").await;
    assert_eq!(count(&pool, "SELECT count(*) FROM events").await, 2, "only bob's two events remain");
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions WHERE subject LIKE 'erased:%'").await, 1);
}

async fn signed_conversion(pool: &PgPool, subject: &str) -> Resp {
    sqlx::query("UPDATE sites SET conversion_signing_secret = 'whsec_t' WHERE id = 'demo'").execute(pool).await.unwrap();
    let body = json!({ "id": format!("order-{subject}"), "subject": subject, "event": "click" }).to_string();
    let sig = pratibimba::conversions::sign("whsec_t", Utc::now().timestamp(), body.as_bytes());
    let req = axum::http::Request::post("/webhook/conversion/demo")
        .header("content-type", "application/json")
        .header("pratibimba-signature", sig)
        .body(axum::body::Body::from(body))
        .unwrap();
    send(&app(pool), req).await
}

#[sqlx::test(migrations = false)]
async fn an_erased_subjects_platform_conversion_is_not_stored(pool: PgPool) {
    setup(&pool, "full").await;
    erase(&pool, "alice").await;
    let r = signed_conversion(&pool, "alice").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM events").await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_platform_conversion_is_stored_for_a_live_subject(pool: PgPool) {
    setup(&pool, "full").await;
    let r = signed_conversion(&pool, "bob").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE subject = 'bob'").await, 1);
}

#[sqlx::test(migrations = false)]
async fn a_cookieless_platform_conversion_is_filed_under_the_pseudonym(pool: PgPool) {
    setup(&pool, "cookieless").await;
    let d = decided(&app(&pool), "alice", json!({})).await;
    signed_conversion(&pool, "alice").await;
    assert_eq!(event_subjects(&pool).await, vec![stored_subject(&pool, &id_of(&d)).await]);
}

#[sqlx::test(migrations = false)]
async fn a_consent_aware_platform_conversion_follows_a_known_visitor(pool: PgPool) {
    setup(&pool, "consent_aware").await;
    events(&app(&pool), "alice", json!({ "consent": true, "events": [ev("view", None)] })).await;
    signed_conversion(&pool, "alice").await;
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE subject = 'alice'").await, 2);
}

#[sqlx::test(migrations = false)]
async fn a_consent_aware_platform_conversion_for_a_stranger_is_pseudonymous(pool: PgPool) {
    setup(&pool, "consent_aware").await;
    signed_conversion(&pool, "alice").await;
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE subject LIKE 'p1\\_%'").await, 1);
}

/// An erase must wait for a write already in flight for the same subject, so the write is
/// either erased with the rest or refused; it can never land after the sweep.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn erase_waits_for_an_inflight_write() {
    let db = FreshDb::new().await;
    let pool = db.pool.clone();
    setup(&pool, "full").await;
    let mut tx = pool.begin().await.unwrap();
    assert!(!privacy::erased(&mut tx, "demo", "alice").await.unwrap());
    let p = pool.clone();
    let job = tokio::spawn(async move { privacy::erase(&p, "demo", "alice", "owner", None).await.unwrap() });
    tokio::time::sleep(StdDuration::from_millis(400)).await;
    assert!(!job.is_finished(), "erase ran while a write for the subject was open");
    sqlx::query("INSERT INTO events (id, site_id, subject, name) VALUES (gen_random_uuid(), 'demo', 'alice', 'view')").execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    job.await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM events").await, 0, "the in-flight write survived the erase");
    db.drop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn erase_of_another_subject_does_not_wait() {
    let db = FreshDb::new().await;
    let pool = db.pool.clone();
    setup(&pool, "full").await;
    let mut tx = pool.begin().await.unwrap();
    privacy::erased(&mut tx, "demo", "bob").await.unwrap();
    let done = tokio::time::timeout(StdDuration::from_secs(5), privacy::erase(&pool, "demo", "alice", "owner", None)).await;
    assert!(done.is_ok(), "erase of alice waited on bob's write");
    tx.rollback().await.unwrap();
    db.drop().await;
}

// ---- the API ----

#[sqlx::test(migrations = false)]
async fn api_erase_tombstones_and_reports(pool: PgPool) {
    setup(&pool, "full").await;
    two_visitors(&pool).await;
    let admin = admin_app(&pool);
    let r = api_post(&admin, "/api/sites/demo/erase", json!({ "subject": "alice", "actor": "owner", "reason": "ticket 12" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.json()["counts"]["events"], json!(2));
    let list = api_get(&admin, "/api/sites/demo/erasures").await.json();
    assert_eq!(list["erasures"].as_array().unwrap().len(), 1);
}

#[sqlx::test(migrations = false)]
async fn api_erase_refuses_without_an_actor(pool: PgPool) {
    setup(&pool, "full").await;
    let r = api_post(&admin_app(&pool), "/api/sites/demo/erase", json!({ "subject": "alice" })).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn api_erase_needs_the_admin_token(pool: PgPool) {
    setup(&pool, "full").await;
    let r = post(&admin_app(&pool), "/api/sites/demo/erase", json!({ "subject": "alice", "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn api_access_lists_what_is_held(pool: PgPool) {
    setup(&pool, "full").await;
    two_visitors(&pool).await;
    let j = api_get(&admin_app(&pool), "/api/sites/demo/subjects/alice").await.json();
    assert_eq!((j["events"].as_array().unwrap().len(), j["decisions"].as_array().unwrap().len(), j["visitors"].as_array().unwrap().len()), (2, 1, 1));
    assert_eq!(j["erased"], json!(false));
}

#[sqlx::test(migrations = false)]
async fn api_access_after_erase_holds_nothing(pool: PgPool) {
    setup(&pool, "full").await;
    two_visitors(&pool).await;
    erase(&pool, "alice").await;
    let j = api_get(&admin_app(&pool), "/api/sites/demo/subjects/alice").await.json();
    assert_eq!((j["events"].as_array().unwrap().len(), j["decisions"].as_array().unwrap().len(), j["visitors"].as_array().unwrap().len()), (0, 0, 0));
    assert_eq!(j["erased"], json!(true));
}

#[sqlx::test(migrations = false)]
async fn api_sets_the_mode_and_fields(pool: PgPool) {
    setup(&pool, "full").await;
    let admin = admin_app(&pool);
    let r = api_post(&admin, "/api/sites/demo/privacy", json!({ "privacy_mode": "cookieless", "fields_allowed": ["mobile"] })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let j = api_get(&admin, "/api/sites/demo/privacy").await.json();
    assert_eq!((j["privacy_mode"].clone(), j["fields_allowed"].clone()), (json!("cookieless"), json!(["mobile"])));
}

#[sqlx::test(migrations = false)]
async fn api_clears_the_field_list_with_null(pool: PgPool) {
    setup(&pool, "full").await;
    set_fields(&pool, &["mobile"]).await;
    let admin = admin_app(&pool);
    api_post(&admin, "/api/sites/demo/privacy", json!({ "fields_allowed": null })).await;
    let j = api_get(&admin, "/api/sites/demo/privacy").await.json();
    assert_eq!((j["privacy_mode"].clone(), j["fields_allowed"].clone()), (json!("full"), Value::Null));
}

#[sqlx::test(migrations = false)]
async fn api_leaves_the_field_list_when_it_is_omitted(pool: PgPool) {
    setup(&pool, "full").await;
    set_fields(&pool, &["mobile"]).await;
    let admin = admin_app(&pool);
    api_post(&admin, "/api/sites/demo/privacy", json!({ "privacy_mode": "cookieless" })).await;
    let j = api_get(&admin, "/api/sites/demo/privacy").await.json();
    assert_eq!(j["fields_allowed"], json!(["mobile"]));
}

#[sqlx::test(migrations = false)]
async fn api_refuses_an_unknown_mode(pool: PgPool) {
    setup(&pool, "full").await;
    let r = api_post(&admin_app(&pool), "/api/sites/demo/privacy", json!({ "privacy_mode": "anonymous" })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
}
