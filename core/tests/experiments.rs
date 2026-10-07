//! D1: experiments, a fixed-allocation A/B overlay on one slot. While one runs, decide assigns
//! each visitor an arm by a hash of (experiment, subject) and weights instead of asking the
//! bandit; the holdout, audiences and the kill switch still come first; stopping it hands the
//! slot back to the bandit. Results per arm carry the sequential confidence sequences.
//!
//! Every refusal test also sends the one-field-fixed body and sees it accepted, so a refusal
//! cannot pass by refusing everything.

mod common;
use axum::{Router, body::Body, http::Request};
use chrono::{Duration, Utc};
use common::*;
use pratibimba::stats::{self, ALPHA};
use serde_json::{Value, json};
use sqlx::PgPool;

/// A valid body: baseline vs gift, 1:1, running from now for a week.
fn body() -> Value {
    json!({
        "site": "demo", "slot": "hero", "name": "gift vs baseline", "actor": "owner",
        "arms": [ { "variant": "baseline", "weight": 1 }, { "variant": "gift", "weight": 1 } ],
        "stops_at": (Utc::now() + Duration::days(7)).to_rfc3339(),
    })
}

fn with(mut b: Value, key: &str, v: Value) -> Value {
    b[key] = v;
    b
}

async fn setup(pool: &PgPool) -> Router {
    setup_demo(pool).await;
    set_holdout(pool, 0.0).await;
    admin_app(pool)
}

async fn create(app: &Router, b: Value) -> Value {
    let r = api_post(app, "/api/experiments", b).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    r.json()
}

/// Refused with `status`, and the same body with `fix` applied is then accepted.
async fn refused_then_fixed(app: &Router, bad: Value, status: StatusCode, good: Value) {
    let r = api_post(app, "/api/experiments", bad).await;
    assert_eq!(r.status, status, "{}", r.text);
    let r = api_post(app, "/api/experiments", good).await;
    assert_eq!(r.status, StatusCode::CREATED, "the accepting fixture was refused: {}", r.text);
}

async fn shown(app: &Router, subject: &str, ctx: Value) -> Value {
    let r = post(app, "/decide", json!({ "site": "demo", "slot": "hero", "subject": subject, "ctx": ctx })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json()
}

async fn list(app: &Router) -> Value {
    let r = api_get(app, "/api/sites/demo/experiments").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json()
}

// ---- POST /api/experiments ----

#[sqlx::test(migrations = false)]
async fn creating_an_experiment_returns_it_running(pool: PgPool) {
    let app = setup(&pool).await;
    let e = create(&app, body()).await;
    assert_eq!(e["name"], "gift vs baseline");
    assert_eq!(e["slot"], "hero");
    assert_eq!(e["metric"], "click", "the primary metric defaults to the slot's");
    assert_eq!(e["status"], "running");
    assert_eq!(e["created_by"], "owner");
    let arms: Vec<(String, f64)> =
        e["arms"].as_array().unwrap().iter().map(|a| (a["variant"].as_str().unwrap().to_string(), a["weight"].as_f64().unwrap())).collect();
    assert_eq!(arms, vec![("baseline".into(), 1.0), ("gift".into(), 1.0)]);
}

#[sqlx::test(migrations = false)]
async fn an_experiment_that_starts_later_is_scheduled(pool: PgPool) {
    let app = setup(&pool).await;
    let e = create(&app, with(body(), "starts_at", json!((Utc::now() + Duration::days(1)).to_rfc3339()))).await;
    assert_eq!(e["status"], "scheduled");
}

#[sqlx::test(migrations = false)]
async fn one_arm_is_refused(pool: PgPool) {
    let app = setup(&pool).await;
    let one = with(body(), "arms", json!([{ "variant": "gift", "weight": 1 }]));
    refused_then_fixed(&app, one, StatusCode::BAD_REQUEST, body()).await;
}

#[sqlx::test(migrations = false)]
async fn a_draft_arm_is_refused(pool: PgPool) {
    let app = setup(&pool).await;
    insert_variant(&pool, "draft1", "draft").await;
    insert_variant(&pool, "live1", "approved").await;
    let arms = |k: &str| json!([{ "variant": "baseline", "weight": 1 }, { "variant": k, "weight": 1 }]);
    refused_then_fixed(&app, with(body(), "arms", arms("draft1")), StatusCode::BAD_REQUEST, with(body(), "arms", arms("live1"))).await;
}

#[sqlx::test(migrations = false)]
async fn an_unknown_arm_is_refused(pool: PgPool) {
    let app = setup(&pool).await;
    let arms = |k: &str| json!([{ "variant": "baseline", "weight": 1 }, { "variant": k, "weight": 1 }]);
    refused_then_fixed(&app, with(body(), "arms", arms("nope")), StatusCode::BAD_REQUEST, with(body(), "arms", arms("gift"))).await;
}

#[sqlx::test(migrations = false)]
async fn a_repeated_arm_is_refused(pool: PgPool) {
    let app = setup(&pool).await;
    let twice = json!([{ "variant": "gift", "weight": 1 }, { "variant": "gift", "weight": 1 }]);
    refused_then_fixed(&app, with(body(), "arms", twice), StatusCode::BAD_REQUEST, body()).await;
}

#[sqlx::test(migrations = false)]
async fn a_zero_weight_is_refused(pool: PgPool) {
    let app = setup(&pool).await;
    let w = |x: f64| json!([{ "variant": "baseline", "weight": 1 }, { "variant": "gift", "weight": x }]);
    refused_then_fixed(&app, with(body(), "arms", w(0.0)), StatusCode::BAD_REQUEST, with(body(), "arms", w(0.25))).await;
}

#[sqlx::test(migrations = false)]
async fn a_negative_weight_is_refused(pool: PgPool) {
    let app = setup(&pool).await;
    let w = |x: f64| json!([{ "variant": "baseline", "weight": 1 }, { "variant": "gift", "weight": x }]);
    refused_then_fixed(&app, with(body(), "arms", w(-1.0)), StatusCode::BAD_REQUEST, with(body(), "arms", w(3.0))).await;
}

#[sqlx::test(migrations = false)]
async fn a_stop_before_the_start_is_refused(pool: PgPool) {
    let app = setup(&pool).await;
    let start = Utc::now() + Duration::days(2);
    let b = with(body(), "starts_at", json!(start.to_rfc3339()));
    let bad = with(b.clone(), "stops_at", json!((start - Duration::hours(1)).to_rfc3339()));
    let good = with(b, "stops_at", json!((start + Duration::hours(1)).to_rfc3339()));
    refused_then_fixed(&app, bad, StatusCode::BAD_REQUEST, good).await;
}

#[sqlx::test(migrations = false)]
async fn a_stop_already_past_is_refused(pool: PgPool) {
    let app = setup(&pool).await;
    let bad = with(body(), "stops_at", json!((Utc::now() - Duration::minutes(5)).to_rfc3339()));
    refused_then_fixed(&app, bad, StatusCode::BAD_REQUEST, body()).await;
}

#[sqlx::test(migrations = false)]
async fn a_metric_the_slot_does_not_record_is_refused(pool: PgPool) {
    let app = setup(&pool).await;
    refused_then_fixed(&app, with(body(), "metric", json!("purchase")), StatusCode::BAD_REQUEST, with(body(), "metric", json!("click"))).await;
}

#[sqlx::test(migrations = false)]
async fn an_unknown_slot_is_not_found(pool: PgPool) {
    let app = setup(&pool).await;
    refused_then_fixed(&app, with(body(), "slot", json!("footer")), StatusCode::NOT_FOUND, body()).await;
}

#[sqlx::test(migrations = false)]
async fn an_unknown_site_is_not_found(pool: PgPool) {
    let app = setup(&pool).await;
    refused_then_fixed(&app, with(body(), "site", json!("nope")), StatusCode::NOT_FOUND, body()).await;
}

#[sqlx::test(migrations = false)]
async fn creating_needs_an_actor(pool: PgPool) {
    let app = setup(&pool).await;
    refused_then_fixed(&app, with(body(), "actor", json!("  ")), StatusCode::BAD_REQUEST, body()).await;
}

#[sqlx::test(migrations = false)]
async fn creating_needs_a_name(pool: PgPool) {
    let app = setup(&pool).await;
    refused_then_fixed(&app, with(body(), "name", json!("")), StatusCode::BAD_REQUEST, body()).await;
}

#[sqlx::test(migrations = false)]
async fn an_overlapping_experiment_on_the_slot_is_a_conflict(pool: PgPool) {
    let app = setup(&pool).await;
    create(&app, body()).await;
    let after = Utc::now() + Duration::days(8);
    let later = with(with(body(), "starts_at", json!(after.to_rfc3339())), "stops_at", json!((after + Duration::days(1)).to_rfc3339()));
    refused_then_fixed(&app, body(), StatusCode::CONFLICT, later).await;
}

#[sqlx::test(migrations = false)]
async fn the_experiment_api_needs_the_admin_token(pool: PgPool) {
    let app = setup(&pool).await;
    let r = post(&app, "/api/experiments", body()).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{}", r.text);
    let r = get(&app, "/api/sites/demo/experiments").await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{}", r.text);
    create(&app, body()).await;
}

// ---- GET /api/sites/:id/experiments ----

#[sqlx::test(migrations = false)]
async fn listing_shows_the_sites_experiments(pool: PgPool) {
    let app = setup(&pool).await;
    let e = create(&app, body()).await;
    let l = list(&app).await;
    let all = l["experiments"].as_array().unwrap();
    assert_eq!(all.len(), 1, "{l}");
    assert_eq!(all[0]["id"], e["id"]);
    assert_eq!(all[0]["control"], "baseline", "the baseline arm is the control");
}

#[sqlx::test(migrations = false)]
async fn listing_an_unknown_site_is_not_found(pool: PgPool) {
    let app = setup(&pool).await;
    assert_eq!(api_get(&app, "/api/sites/nope/experiments").await.status, StatusCode::NOT_FOUND);
    assert_eq!(api_get(&app, "/api/sites/demo/experiments").await.status, StatusCode::OK);
}

// ---- decide while it runs ----

#[sqlx::test(migrations = false)]
async fn a_running_experiment_assigns_by_the_hash(pool: PgPool) {
    let app = setup(&pool).await;
    let e = create(&app, body()).await;
    let eid = e["id"].as_i64().unwrap();
    let arms = [(variant_id(&pool, "baseline").await, 1.0), (variant_id(&pool, "gift").await, 1.0)];
    for i in 0..40 {
        let s = format!("v{i}");
        let d = shown(&app, &s, json!({})).await;
        let want = pratibimba::experiments::assign(eid, &s, &arms);
        assert_eq!(d["variant"], if want == arms[0].0 { "baseline" } else { "gift" }, "{d}");
        assert_eq!(d["explanation"]["allocation"], "experiment", "{d}");
        assert_eq!(d["explanation"]["experiment"]["id"], json!(eid), "{d}");
    }
    assert_eq!(count(&pool, &format!("SELECT count(*) FROM decisions WHERE experiment_id = {eid} AND allocation = 'experiment'")).await, 40);
}

#[sqlx::test(migrations = false)]
async fn the_assignment_survives_the_sticky_window(pool: PgPool) {
    let app = setup(&pool).await;
    create(&app, body()).await;
    let mut first = vec![];
    for i in 0..20 {
        first.push(shown(&app, &format!("v{i}"), json!({})).await["variant"].clone());
    }
    sqlx::query("UPDATE decisions SET created_at = created_at - interval '1 day'").execute(&pool).await.unwrap();
    for (i, f) in first.iter().enumerate() {
        assert_eq!(&shown(&app, &format!("v{i}"), json!({})).await["variant"], f, "subject v{i} moved arms");
    }
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions").await, 40, "the second round must be fresh decisions");
}

#[test]
fn the_weights_shape_the_split() {
    let arms = [(1, 9.0), (2, 1.0)];
    let first = (0..2000).filter(|i| pratibimba::experiments::assign(5, &format!("s{i}"), &arms) == 1).count();
    assert!((1700..=1900).contains(&first), "{first}/2000 got the 90% arm");
}

#[test]
fn the_assignment_differs_between_experiments() {
    let arms = [(1, 1.0), (2, 1.0)];
    let same = (0..400).filter(|i| {
        let s = format!("s{i}");
        pratibimba::experiments::assign(1, &s, &arms) == pratibimba::experiments::assign(2, &s, &arms)
    });
    let n = same.count();
    assert!((140..=260).contains(&n), "{n}/400 agree: the hash ignores the experiment");
}

#[sqlx::test(migrations = false)]
async fn a_heavy_weight_takes_most_visitors(pool: PgPool) {
    let app = setup(&pool).await;
    create(&app, with(body(), "arms", json!([{ "variant": "baseline", "weight": 1 }, { "variant": "gift", "weight": 19 }]))).await;
    let mut gifts = 0;
    for i in 0..100 {
        if shown(&app, &format!("v{i}"), json!({})).await["variant"] == "gift" {
            gifts += 1;
        }
    }
    assert!(gifts >= 85, "{gifts}/100 saw the 95% arm");
}

#[sqlx::test(migrations = false)]
async fn before_it_starts_the_bandit_decides(pool: PgPool) {
    let app = setup(&pool).await;
    create(&app, with(body(), "starts_at", json!((Utc::now() + Duration::days(1)).to_rfc3339()))).await;
    let d = shown(&app, "v1", json!({})).await;
    assert_eq!(d["explanation"]["allocation"], "bandit", "{d}");
    assert_eq!(d["explanation"]["experiment"], Value::Null, "{d}");
}

#[sqlx::test(migrations = false)]
async fn after_it_ends_the_bandit_decides(pool: PgPool) {
    let app = setup(&pool).await;
    create(&app, body()).await;
    sqlx::query("UPDATE experiments SET starts_at = now() - interval '2 days', stops_at = now() - interval '1 second'").execute(&pool).await.unwrap();
    let d = shown(&app, "v1", json!({})).await;
    assert_eq!(d["explanation"]["allocation"], "bandit", "{d}");
    assert_eq!(list(&app).await["experiments"][0]["status"], "stopped");
}

#[sqlx::test(migrations = false)]
async fn a_sticky_bandit_decision_gives_way_when_an_experiment_starts(pool: PgPool) {
    let app = setup(&pool).await;
    assert_eq!(shown(&app, "v1", json!({})).await["explanation"]["allocation"], "bandit");
    create(&app, body()).await;
    assert_eq!(shown(&app, "v1", json!({})).await["explanation"]["allocation"], "experiment");
}

#[sqlx::test(migrations = false)]
async fn stopping_returns_the_slot_to_the_bandit(pool: PgPool) {
    let app = setup(&pool).await;
    let e = create(&app, body()).await;
    assert_eq!(shown(&app, "v1", json!({})).await["explanation"]["allocation"], "experiment");
    let r = api_post(&app, &format!("/api/experiments/{}/stop", e["id"]), json!({ "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.json()["status"], "stopped");
    let d = shown(&app, "v1", json!({})).await;
    assert_eq!(d["explanation"]["allocation"], "bandit", "the sticky experiment decision outlived the stop: {d}");
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions WHERE allocation = 'bandit' AND experiment_id IS NULL").await, 1);
}

#[sqlx::test(migrations = false)]
async fn stopping_twice_is_a_conflict(pool: PgPool) {
    let app = setup(&pool).await;
    let e = create(&app, body()).await;
    let path = format!("/api/experiments/{}/stop", e["id"]);
    assert_eq!(api_post(&app, &path, json!({ "actor": "owner" })).await.status, StatusCode::OK);
    assert_eq!(api_post(&app, &path, json!({ "actor": "owner" })).await.status, StatusCode::CONFLICT);
}

#[sqlx::test(migrations = false)]
async fn stopping_needs_an_actor(pool: PgPool) {
    let app = setup(&pool).await;
    let e = create(&app, body()).await;
    let path = format!("/api/experiments/{}/stop", e["id"]);
    assert_eq!(api_post(&app, &path, json!({ "actor": "" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(api_post(&app, &path, json!({ "actor": "owner" })).await.status, StatusCode::OK);
}

#[sqlx::test(migrations = false)]
async fn stopping_an_unknown_experiment_is_not_found(pool: PgPool) {
    let app = setup(&pool).await;
    let e = create(&app, body()).await;
    assert_eq!(api_post(&app, "/api/experiments/999999/stop", json!({ "actor": "owner" })).await.status, StatusCode::NOT_FOUND);
    assert_eq!(api_post(&app, &format!("/api/experiments/{}/stop", e["id"]), json!({ "actor": "owner" })).await.status, StatusCode::OK);
}

#[sqlx::test(migrations = false)]
async fn a_stopped_slot_takes_a_new_experiment(pool: PgPool) {
    let app = setup(&pool).await;
    let e = create(&app, body()).await;
    api_post(&app, &format!("/api/experiments/{}/stop", e["id"]), json!({ "actor": "owner" })).await;
    create(&app, body()).await;
}

#[sqlx::test(migrations = false)]
async fn the_holdout_still_gets_the_baseline(pool: PgPool) {
    let app = setup(&pool).await;
    create(&app, with(body(), "arms", json!([{ "variant": "baseline", "weight": 1 }, { "variant": "gift", "weight": 1000 }]))).await;
    set_holdout(&pool, 0.5).await;
    let held: Vec<String> = (0..200).map(|i| format!("h{i}")).filter(|s| pratibimba::hash::in_holdout("demo", s, 0.5)).take(20).collect();
    assert_eq!(held.len(), 20);
    for s in &held {
        let d = shown(&app, s, json!({})).await;
        assert_eq!(d["variant"], "baseline", "{d}");
        assert_eq!(d["explanation"]["in_holdout"], true, "{d}");
        assert_eq!(d["explanation"]["allocation"], "holdout", "{d}");
    }
    set_holdout(&pool, 0.0).await;
    assert_eq!(shown(&app, "t1", json!({})).await["explanation"]["allocation"], "experiment");
}

#[sqlx::test(migrations = false)]
async fn audiences_still_gate_the_arms(pool: PgPool) {
    let app = setup(&pool).await;
    let r = api_post(&app, "/api/sites/demo/segments", json!({ "predicate": { "returning": true }, "actor": "owner" })).await;
    let aud = r.json()["id"].as_i64().unwrap();
    api_post(&app, &format!("/api/segments/{aud}/promote"), json!({ "actor": "owner" })).await;
    let gift = variant_id(&pool, "gift").await;
    let r = api_post(&app, &format!("/api/variants/{gift}/audience"), json!({ "audience": aud, "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    create(&app, with(body(), "arms", json!([{ "variant": "baseline", "weight": 1 }, { "variant": "gift", "weight": 1000 }]))).await;
    for i in 0..20 {
        let d = shown(&app, &format!("new{i}"), json!({ "returning": false })).await;
        assert_eq!(d["variant"], "baseline", "a visitor outside the audience got gift: {d}");
    }
    let mut gifts = 0;
    for i in 0..20 {
        if shown(&app, &format!("back{i}"), json!({ "returning": true })).await["variant"] == "gift" {
            gifts += 1;
        }
    }
    assert!(gifts >= 18, "{gifts}/20 matching visitors got the 99.9% arm");
}

#[sqlx::test(migrations = false)]
async fn a_paused_site_serves_the_baseline_during_an_experiment(pool: PgPool) {
    let app = setup(&pool).await;
    create(&app, with(body(), "arms", json!([{ "variant": "baseline", "weight": 1 }, { "variant": "gift", "weight": 1000 }]))).await;
    let r = api_post(&app, "/api/sites/demo/pause", json!({ "paused": true, "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    for i in 0..10 {
        assert_eq!(shown(&app, &format!("p{i}"), json!({})).await["variant"], "baseline");
    }
    api_post(&app, "/api/sites/demo/pause", json!({ "paused": false, "actor": "owner" })).await;
    let mut n = 0;
    for i in 0..10 {
        if shown(&app, &format!("u{i}"), json!({})).await["variant"] == "gift" {
            n += 1;
        }
    }
    assert!(n >= 9, "unpaused, the experiment should serve gift: {n}/10");
}

#[sqlx::test(migrations = false)]
async fn experiment_decisions_teach_the_model(pool: PgPool) {
    let app = setup(&pool).await;
    create(&app, body()).await;
    let d = shown(&app, "v1", json!({})).await;
    let id = d["decision_id"].as_str().unwrap().to_string();
    assert_eq!(event(&app, "v1", "exposure", Some(&id)).await.status, StatusCode::ACCEPTED);
    assert_eq!(event(&app, "v1", "click", Some(&id)).await.status, StatusCode::ACCEPTED);
    pratibimba::worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions WHERE outcome = 1 AND allocation = 'experiment'").await, 1);
    let learned = count(&pool, "SELECT count(*) FROM model_versions WHERE reason = 'learn: conversion'").await;
    assert_eq!(learned, 1, "the bandit did not learn from the experiment's conversion");
}

#[sqlx::test(migrations = false)]
async fn the_explanation_sentence_names_the_experiment(pool: PgPool) {
    let app = setup(&pool).await;
    create(&app, body()).await;
    let d = shown(&app, "v1", json!({})).await;
    let r = api_get(&app, &format!("/api/decisions/{}", d["decision_id"].as_str().unwrap())).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let s = r.json()["sentence"].as_str().unwrap().to_string();
    assert!(s.contains("experiment `gift vs baseline`"), "{s}");
}

// ---- results ----

/// `n` closed, exposed decisions of `variant` under experiment `eid`, `conv` of them converted.
async fn seed(pool: &PgPool, eid: i64, variant: &str, allocation: &str, n: i64, conv: i64, age_minutes: i64) {
    sqlx::query(
        "INSERT INTO decisions (id, site_id, slot_id, variant_id, subject, pool, features, scores, policy, policy_version, seed,
                                in_holdout, created_at, exposed_at, outcome, experiment_id, allocation)
         SELECT gen_random_uuid(), 'demo', v.slot_id, v.id, 'x-' || $1 || '-' || $5 || '-' || $6::text || '-' || i, '{}', '{}', '{}', 'lints', 0, 0,
                false, now() - make_interval(mins => $6::int), now() - make_interval(mins => $6::int),
                CASE WHEN i <= $4 THEN 1 END, $2, $5
           FROM variants v, generate_series(1, $3::int) i WHERE v.key = $1",
    )
    .bind(variant)
    .bind(eid)
    .bind(n)
    .bind(conv)
    .bind(allocation)
    .bind(age_minutes)
    .execute(pool)
    .await
    .unwrap();
}

fn arm<'a>(l: &'a Value, key: &str) -> &'a Value {
    l["experiments"][0]["results"]["arms"].as_array().unwrap().iter().find(|a| a["variant"] == key).unwrap()
}

#[sqlx::test(migrations = false)]
async fn results_give_each_arm_its_rate_and_sequence(pool: PgPool) {
    let app = setup(&pool).await;
    let eid = create(&app, body()).await["id"].as_i64().unwrap();
    seed(&pool, eid, "baseline", "experiment", 200, 10, 60).await;
    seed(&pool, eid, "gift", "experiment", 200, 30, 60).await;
    let l = list(&app).await;
    let g = arm(&l, "gift");
    assert_eq!(g["counts"]["n"], 200);
    assert_eq!(g["counts"]["conversions"], 30);
    let want = stats::rate_cs(30, 200, ALPHA).unwrap();
    assert_eq!(g["rate"]["lo"].as_f64().unwrap(), want.lo);
    assert_eq!(g["rate"]["hi"].as_f64().unwrap(), want.hi);
    assert_eq!(l["alpha"], json!(ALPHA));
}

#[sqlx::test(migrations = false)]
async fn results_give_each_arm_its_lift_over_the_control(pool: PgPool) {
    let app = setup(&pool).await;
    let eid = create(&app, body()).await["id"].as_i64().unwrap();
    seed(&pool, eid, "baseline", "experiment", 200, 10, 60).await;
    seed(&pool, eid, "gift", "experiment", 200, 30, 60).await;
    let l = list(&app).await;
    let want = stats::lift_cs((30, 200), (10, 200), ALPHA).unwrap();
    assert_eq!(arm(&l, "gift")["lift"]["lo"].as_f64().unwrap(), want.lo);
    assert_eq!(arm(&l, "gift")["lift"]["hi"].as_f64().unwrap(), want.hi);
    assert_eq!(arm(&l, "baseline")["lift"], Value::Null, "the control has no lift over itself");
}

#[sqlx::test(migrations = false)]
async fn results_count_only_the_experiments_own_assignments(pool: PgPool) {
    let app = setup(&pool).await;
    let eid = create(&app, body()).await["id"].as_i64().unwrap();
    seed(&pool, eid, "gift", "experiment", 50, 5, 60).await;
    // Bandit fallbacks made while it ran (e.g. every arm gated away) are not the experiment's.
    seed(&pool, eid, "gift", "bandit", 70, 70, 60).await;
    seed_decisions(&pool, "gift", false, 30, 30, 60).await;
    let l = list(&app).await;
    assert_eq!(arm(&l, "gift")["counts"]["n"], 50, "{l}");
}

#[sqlx::test(migrations = false)]
async fn results_wait_for_the_window_to_close(pool: PgPool) {
    let app = setup(&pool).await;
    let eid = create(&app, body()).await["id"].as_i64().unwrap();
    seed(&pool, eid, "gift", "experiment", 50, 5, 60).await;
    seed(&pool, eid, "gift", "experiment", 40, 0, 1).await;
    let l = list(&app).await;
    assert_eq!(arm(&l, "gift")["counts"]["n"], 50, "open windows were counted: {l}");
}

// ---- MCP list_experiments ----

async fn mcp(app: &Router, token: &str, tool: &str) -> Value {
    let body = json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call",
                       "params": { "name": tool, "arguments": { "agent": "Claude Code", "justification": "check the test" } } });
    let req = Request::post("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("authorization", format!("Bearer {token}"));
    let r = send(app, req.body(Body::from(body.to_string())).unwrap()).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let res = r.json()["result"].clone();
    assert_ne!(res["isError"], json!(true), "tool failed: {res}");
    serde_json::from_str(res["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[sqlx::test(migrations = false)]
async fn list_experiments_shows_the_tokens_sites_experiments(pool: PgPool) {
    let app = setup(&pool).await;
    let e = create(&app, body()).await;
    let t = pratibimba::mcp::issue_token(&pool, "demo", "test agent").await.unwrap();
    let l = mcp(&app, &t, "list_experiments").await;
    assert_eq!(l["experiments"][0]["id"], e["id"], "{l}");
    assert_eq!(l["experiments"][0]["status"], "running", "{l}");
}

#[sqlx::test(migrations = false)]
async fn list_experiments_is_audited(pool: PgPool) {
    let app = setup(&pool).await;
    let t = pratibimba::mcp::issue_token(&pool, "demo", "test agent").await.unwrap();
    mcp(&app, &t, "list_experiments").await;
    assert_eq!(count(&pool, "SELECT count(*) FROM mcp_calls WHERE tool = 'list_experiments' AND outcome = 'ok'").await, 1);
}

#[sqlx::test(migrations = false)]
async fn there_is_no_mcp_tool_that_writes_an_experiment(pool: PgPool) {
    let app = setup(&pool).await;
    let t = pratibimba::mcp::issue_token(&pool, "demo", "test agent").await.unwrap();
    let req = Request::post("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("authorization", format!("Bearer {t}"));
    let r = send(&app, req.body(Body::from(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }).to_string())).unwrap()).await;
    let names: Vec<String> = r.json()["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    assert!(names.contains(&"list_experiments".to_string()), "{names:?}");
    assert!(names.iter().filter(|n| n.contains("experiment")).count() == 1, "{names:?}");
}
