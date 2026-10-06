//! The simulator and time-travel: a sandbox copy of a site, its own clock, and the guarantee
//! that nothing it does reaches the real site's results or models.

mod common;

use chrono::{Duration, Utc};
use common::*;
use pratibimba::sim::{self, Cohort};
use serde_json::{Value, json};
use sqlx::PgPool;

fn cohort(visitors: u32, gift: f64, baseline: f64) -> Cohort {
    serde_json::from_value(json!({
        "visitors": visitors, "seed": 11,
        "rates": { "hero": { "gift": gift, "baseline": baseline } }
    }))
    .unwrap()
}

/// Everything the real `demo` site has learned or reported, as one comparable value.
async fn real_snapshot(pool: &PgPool) -> Value {
    let models: Vec<(i64, Value)> = sqlx::query_as("SELECT m.version, m.state FROM models m JOIN slots s ON s.id = m.slot_id WHERE s.site_id = 'demo'")
        .fetch_all(pool)
        .await
        .unwrap();
    let results = pratibimba::results::site(pool, "demo", pratibimba::results::ALL_TIME, Utc::now() + Duration::days(400)).await.unwrap();
    json!({
        "models": models,
        "versions": count(pool, "SELECT count(*) FROM model_versions v JOIN slots s ON s.id = v.slot_id WHERE s.site_id = 'demo'").await,
        "decisions": count(pool, "SELECT count(*) FROM decisions WHERE site_id = 'demo'").await,
        "events": count(pool, "SELECT count(*) FROM events WHERE site_id = 'demo'").await,
        "learnings": count(pool, "SELECT count(*) FROM learnings WHERE site_id = 'demo'").await,
        "results": serde_json::to_value(results.map(|r| r.slots.iter().map(|s| s.variants.iter().map(|v| (v.counts.n, v.counts.conversions)).collect::<Vec<_>>()).collect::<Vec<_>>())).unwrap(),
    })
}

#[sqlx::test(migrations = false)]
async fn simulated_traffic_never_reaches_the_real_sites_models_or_results(pool: PgPool) {
    setup_learning(&pool).await;
    // The real site has a little history of its own, so "unchanged" is not "empty".
    let app = app(&pool);
    let d = decide(&app, "real-1", false).await.json();
    let id = d["decision_id"].as_str().unwrap().to_string();
    event(&app, "real-1", "exposure", Some(&id)).await;
    event(&app, "real-1", "click", Some(&id)).await;
    pratibimba::worker::run_once(&pool, Utc::now()).await.unwrap();
    let before = real_snapshot(&pool).await;

    sim::reset(&pool, "demo", Utc::now()).await.unwrap();
    let days = sim::advance(&pool, "demo", 30 * 86_400, Some(&cohort(40, 0.4, 0.05))).await.unwrap();
    assert_eq!(days.len(), 30);
    // The real worker, run as if a year had passed, still finds nothing of the sandbox's.
    pratibimba::worker::run_once(&pool, Utc::now() + Duration::days(365)).await.unwrap();
    pratibimba::guard::run(&pool, Utc::now() + Duration::days(365)).await.unwrap();

    assert_eq!(real_snapshot(&pool).await, before);
    // And the sandbox really did run: decisions, conversions and a model that moved.
    assert!(count(&pool, "SELECT count(*) FROM decisions WHERE site_id = 'demo~sim'").await >= 1200);
    assert!(count(&pool, "SELECT count(*) FROM decisions WHERE site_id = 'demo~sim' AND outcome = 1").await > 0);
    assert!(count(&pool, "SELECT max(m.version) FROM models m JOIN slots s ON s.id = m.slot_id WHERE s.site_id = 'demo~sim'").await > 100);
}

#[sqlx::test(migrations = false)]
async fn the_real_worker_leaves_sandbox_events_and_decisions_alone(pool: PgPool) {
    setup_learning(&pool).await;
    sim::reset(&pool, "demo", Utc::now()).await.unwrap();
    sim::visitors(&pool, "demo", &cohort(20, 1.0, 1.0)).await.unwrap();
    // `visitors` runs the sandbox's own pass; put fresh traffic in the outbox for the real worker.
    let open = count(&pool, "SELECT count(*) FROM decisions WHERE site_id = 'demo~sim' AND settled_at IS NULL").await;
    sqlx::query("INSERT INTO pending_events (event_id) SELECT id FROM events WHERE site_id = 'demo~sim'").execute(&pool).await.unwrap();
    let queued = count(&pool, "SELECT count(*) FROM pending_events").await;
    assert!(queued > 0);

    let s = pratibimba::worker::run_once(&pool, Utc::now() + Duration::days(365)).await.unwrap();

    assert_eq!(s, pratibimba::worker::Stats::default(), "the real pass must not see sandbox rows");
    assert_eq!(count(&pool, "SELECT count(*) FROM pending_events").await, queued);
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions WHERE site_id = 'demo~sim' AND settled_at IS NULL").await, open);
}

#[sqlx::test(migrations = false)]
async fn advancing_the_clock_closes_windows_on_the_sandbox_only(pool: PgPool) {
    setup_learning(&pool).await;
    let t0 = Utc::now();
    sim::reset(&pool, "demo", t0).await.unwrap();
    sim::visitors(&pool, "demo", &cohort(10, 0.0, 0.0)).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions WHERE site_id = 'demo~sim' AND settled_at IS NULL").await, 10);

    sim::advance(&pool, "demo", 3600, None).await.unwrap();

    let clock = sim::clock(&pool, "demo").await.unwrap();
    assert_eq!((clock - t0).num_seconds(), 3600);
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions WHERE site_id = 'demo~sim' AND outcome = 0").await, 10);
}

#[sqlx::test(migrations = false)]
async fn skipping_thirty_days_learns_the_planted_winner(pool: PgPool) {
    setup_learning(&pool).await;
    sim::reset(&pool, "demo", Utc::now()).await.unwrap();
    let days = sim::advance(&pool, "demo", 30 * 86_400, Some(&cohort(60, 0.5, 0.05))).await.unwrap();
    let served = |d: &sim::Day, v: &str| d.served.get("hero").and_then(|m| m.get(v)).copied().unwrap_or(0);
    let last = days.last().unwrap();
    assert!(served(last, "gift") > 3 * served(last, "baseline"), "day 30 still split traffic: {:?}", last.served);
    let status = sim::status(&pool, "demo").await.unwrap();
    assert_eq!(status["sandbox"], "demo~sim");
}

#[sqlx::test(migrations = false)]
async fn decide_on_the_sandbox_uses_its_clock(pool: PgPool) {
    setup_learning(&pool).await;
    let t0 = Utc::now();
    sim::reset(&pool, "demo", t0).await.unwrap();
    sim::advance(&pool, "demo", 10 * 86_400, None).await.unwrap();
    let r = post(&app(&pool), "/decide", json!({ "site": "demo~sim", "slot": "hero", "subject": "dev-1" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let at: chrono::DateTime<Utc> = sqlx::query_scalar("SELECT created_at FROM decisions WHERE subject = 'dev-1'").fetch_one(&pool).await.unwrap();
    assert!((at - t0).num_days() == 10, "decided at {at}, clock started {t0}");
    let r = post(&app(&pool), "/e", json!({ "site": "demo~sim", "subject": "dev-1", "events": [{ "id": uuid::Uuid::new_v4(), "name": "click" }] })).await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    let at: chrono::DateTime<Utc> = sqlx::query_scalar("SELECT received_at FROM events WHERE subject = 'dev-1'").fetch_one(&pool).await.unwrap();
    assert!((at - t0).num_days() == 10, "event received at {at}");
}

#[sqlx::test(migrations = false)]
async fn a_sandbox_cannot_be_sandboxed_and_unknown_sites_are_refused(pool: PgPool) {
    setup_learning(&pool).await;
    sim::reset(&pool, "demo", Utc::now()).await.unwrap();
    assert!(sim::reset(&pool, "demo~sim", Utc::now()).await.is_err());
    assert!(sim::reset(&pool, "nope", Utc::now()).await.is_err());
}

#[sqlx::test(migrations = false)]
async fn reset_starts_the_sandbox_from_scratch(pool: PgPool) {
    setup_learning(&pool).await;
    sim::reset(&pool, "demo", Utc::now()).await.unwrap();
    sim::visitors(&pool, "demo", &cohort(10, 1.0, 1.0)).await.unwrap();
    sim::reset(&pool, "demo", Utc::now()).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions WHERE site_id = 'demo~sim'").await, 0);
    assert_eq!(count(&pool, "SELECT count(*) FROM variants v JOIN slots s ON s.id = v.slot_id WHERE s.site_id = 'demo~sim' AND v.status = 'approved'").await, 2);
}

#[sqlx::test(migrations = false)]
async fn the_sim_api_is_admin_only_and_drives_the_sandbox(pool: PgPool) {
    setup_learning(&pool).await;
    let app = admin_app(&pool);
    let r = post(&app, "/api/sites/demo/sim", json!({})).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    let r = api_post(&app, "/api/sites/demo/sim", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.json()["sandbox"], "demo~sim");
    let r = api_post(&app, "/api/sites/demo/sim/advance", json!({ "days": 2, "cohort": { "visitors": 5, "seed": 3, "rates": { "hero": { "gift": 0.5 } } } })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.json()["days"].as_array().unwrap().len(), 2);
    let r = api_post(&app, "/api/sites/demo/sim/visitors", json!({ "visitors": 4, "seed": 9 })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let r = api_get(&app, "/api/sites/demo/sim").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.json()["decisions"], 14);

    // The sandbox is not a site of its own in the site list.
    let sites = api_get(&app, "/api/sites").await.json();
    assert_eq!(sites.as_array().unwrap().len(), 1, "{sites}");
}
