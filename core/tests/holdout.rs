//! Holdout, planned traffic share and paused variants: the guardrails on the arm pool.

mod common;
use chrono::{Duration, Utc};
use common::*;
use pratibimba::{bandit::LinTs, worker};
use serde_json::Value;
use sqlx::PgPool;

async fn variant_id(pool: &PgPool, key: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM variants WHERE key = $1").bind(key).fetch_one(pool).await.unwrap()
}

/// Make the model love `gift`, so a decision that is not forced elsewhere picks it.
async fn train_gift(pool: &PgPool) {
    let gift = variant_id(pool, "gift").await;
    let base = variant_id(pool, "baseline").await;
    let mut m = LinTs::default();
    for _ in 0..200 {
        m.update(gift, &[1.0, 0.0, 0.0, 0.0], true);
        m.update(base, &[1.0, 0.0, 0.0, 0.0], false);
    }
    sqlx::query("UPDATE models SET state = $1").bind(serde_json::to_value(&m).unwrap()).execute(pool).await.unwrap();
}

async fn decisions(app: &axum::Router, n: usize, prefix: &str) -> Vec<Value> {
    let mut out = vec![];
    for i in 0..n {
        out.push(decide(app, &format!("{prefix}{i}"), false).await.json());
    }
    out
}

fn held(d: &Value) -> bool {
    d["explanation"]["in_holdout"].as_bool().unwrap()
}

#[sqlx::test(migrations = false)]
async fn holdout_visitors_get_the_baseline(pool: PgPool) {
    setup_demo(&pool).await;
    set_holdout(&pool, 0.5).await;
    train_gift(&pool).await;
    let ds = decisions(&app(&pool), 200, "h").await;
    let holdout: Vec<_> = ds.iter().filter(|d| held(d)).collect();
    assert!(holdout.len() > 50, "too few holdout visitors to judge: {}", holdout.len());
    for d in holdout {
        assert_eq!(d["variant"], "baseline", "a holdout visitor got {}", d["variant"]);
    }
}

#[sqlx::test(migrations = false)]
async fn holdout_share_matches_fraction(pool: PgPool) {
    setup_demo(&pool).await; // default fraction 0.10
    let ds = decisions(&app(&pool), 2000, "s").await;
    let n = ds.iter().filter(|d| held(d)).count();
    assert!((140..=260).contains(&n), "holdout share {n}/2000, expected about 10%");
    let rows = count(&pool, "SELECT count(*) FROM decisions WHERE in_holdout").await;
    assert_eq!(rows as usize, n, "the stored flag disagrees with the response");
}

#[sqlx::test(migrations = false)]
async fn holdout_is_per_visitor_not_per_decision(pool: PgPool) {
    setup_demo(&pool).await;
    set_holdout(&pool, 0.5).await;
    let app = app(&pool);
    let first = decisions(&app, 60, "p").await;
    // Let every decision's TTL lapse, so the second round decides afresh with new seeds.
    sqlx::query("UPDATE decisions SET created_at = now() - interval '1 hour'").execute(&pool).await.unwrap();
    let second = decisions(&app, 60, "p").await;
    for (a, b) in first.iter().zip(&second) {
        assert_ne!(a["decision_id"], b["decision_id"]);
        assert_eq!(held(a), held(b), "a visitor changed holdout group between visits");
    }
}

#[sqlx::test(migrations = false)]
async fn treated_visitors_reach_both_arms(pool: PgPool) {
    setup_demo(&pool).await;
    let ds = decisions(&app(&pool), 120, "t").await;
    let treated: std::collections::HashSet<String> =
        ds.iter().filter(|d| !held(d)).map(|d| d["variant"].as_str().unwrap().to_string()).collect();
    assert_eq!(treated.len(), 2, "treated visitors saw {treated:?}");
}

async fn exposed_holdout_decision(pool: &PgPool) -> (axum::Router, String, uuid::Uuid) {
    setup_demo(pool).await;
    set_holdout(pool, 0.5).await;
    let app = app(pool);
    for i in 0..100 {
        let s = format!("x{i}");
        let d = decide(&app, &s, false).await.json();
        if held(&d) {
            let id = d["decision_id"].as_str().unwrap().to_string();
            event(&app, &s, "exposure", Some(&id)).await;
            return (app, s, id.parse().unwrap());
        }
    }
    panic!("no holdout visitor in 100");
}

#[sqlx::test(migrations = false)]
async fn holdout_conversion_is_recorded_not_learned(pool: PgPool) {
    let (app, s, id) = exposed_holdout_decision(&pool).await;
    event(&app, &s, "click", Some(&id.to_string())).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    // Conversions are learned when the window closes, so close it before saying "never".
    worker::run_once(&pool, Utc::now() + Duration::hours(1)).await.unwrap();
    let outcome: Option<i16> = sqlx::query_scalar("SELECT outcome FROM decisions WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(outcome, Some(1), "the holdout conversion was not recorded");
    assert_eq!(count(&pool, "SELECT version FROM models").await, 0, "the model learned from the holdout");
}

#[sqlx::test(migrations = false)]
async fn holdout_miss_is_recorded_not_learned(pool: PgPool) {
    let (_app, _s, id) = exposed_holdout_decision(&pool).await;
    worker::run_once(&pool, Utc::now() + Duration::hours(1)).await.unwrap();
    let outcome: Option<i16> = sqlx::query_scalar("SELECT outcome FROM decisions WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(outcome, Some(0), "the holdout miss was not recorded");
    assert_eq!(count(&pool, "SELECT version FROM models").await, 0, "the model learned from the holdout");
}

#[sqlx::test(migrations = false)]
async fn holdout_explanation_says_so(pool: PgPool) {
    let (app, s, id) = exposed_holdout_decision(&pool).await;
    // The sticky path must report the flag too, not only a fresh decision.
    let again = decide(&app, &s, false).await.json();
    assert_eq!(again["decision_id"], id.to_string());
    assert!(held(&again), "the sticky response lost the holdout flag");
}

/// Approve `gift` with a planned share, as the dashboard gate would.
async fn cap_gift(pool: &PgPool, share: f32) {
    sqlx::query("UPDATE variants SET traffic_share = $1 WHERE key = 'gift'").bind(share).execute(pool).await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn capped_variant_stays_under_its_share(pool: PgPool) {
    setup_demo(&pool).await;
    set_holdout(&pool, 0.0).await;
    train_gift(&pool).await;
    cap_gift(&pool, 0.2).await;
    let ds = decisions(&app(&pool), 600, "c").await;
    let gift = ds.iter().filter(|d| d["variant"] == "gift").count();
    assert!(gift <= 150, "a variant capped at 20% reached {gift}/600");
}

#[sqlx::test(migrations = false)]
async fn capped_variant_still_serves(pool: PgPool) {
    setup_demo(&pool).await;
    set_holdout(&pool, 0.0).await;
    train_gift(&pool).await;
    cap_gift(&pool, 0.2).await;
    let ds = decisions(&app(&pool), 600, "c").await;
    let gift = ds.iter().filter(|d| d["variant"] == "gift").count();
    assert!(gift >= 80, "a variant capped at 20% that the model prefers reached only {gift}/600");
}

#[sqlx::test(migrations = false)]
async fn paused_variant_is_never_served(pool: PgPool) {
    setup_demo(&pool).await;
    train_gift(&pool).await;
    sqlx::query("UPDATE variants SET status = 'paused' WHERE key = 'gift'").execute(&pool).await.unwrap();
    for d in decisions(&app(&pool), 60, "q").await {
        assert_eq!(d["variant"], "baseline", "a paused variant was served");
    }
}
