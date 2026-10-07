//! E4 delayed-feedback bug: the worker taught the model a conversion as soon as the click was
//! drained, but a miss only when the attribution window closed. For the newest window of traffic
//! the posterior saw successes with no matching failures (E4 measured an inferred 0.97 for an arm
//! that converts at 5%). Now every decision teaches the model once, when its window closes, with
//! its final outcome; the record of the conversion (outcome, credit, webhook) is still written at
//! once. See the comment on `worker::settle_batch` for why this, not a provisional miss.

mod common;

use axum::Router;
use chrono::{Duration, Utc};
use common::*;
use pratibimba::bandit::{LinTs, sigmoid};
use pratibimba::http::decide::from_named;
use pratibimba::worker;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Barrier;
use uuid::Uuid;

const X_RET: [f64; 4] = [1.0, 1.0, 0.0, 0.0];

fn dot(a: &[f64; 4], b: &[f64; 4]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

async fn model(pool: &PgPool) -> (i64, LinTs) {
    let (v, s): (i64, Value) = sqlx::query_as("SELECT version, state FROM models").fetch_one(pool).await.unwrap();
    (v, serde_json::from_value(s).unwrap())
}

async fn outcome(pool: &PgPool, d: Uuid) -> (Option<i16>, bool) {
    sqlx::query_as("SELECT outcome, settled_at IS NOT NULL FROM decisions WHERE id = $1").bind(d).fetch_one(pool).await.unwrap()
}

async fn exposed(app: &Router, pool: &PgPool, subject: &str) -> Uuid {
    let d: Uuid = decide(app, subject, true).await.json()["decision_id"].as_str().unwrap().parse().unwrap();
    event(app, subject, "exposure", Some(&d.to_string())).await;
    worker::run_once(pool, Utc::now()).await.unwrap();
    d
}

/// Just past the demo slot's 30-minute window.
fn closed() -> chrono::DateTime<Utc> {
    Utc::now() + Duration::minutes(31)
}

#[sqlx::test(migrations = false)]
async fn a_click_is_recorded_at_once_but_learned_when_the_window_closes(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let d = exposed(&app, &pool, "v").await;
    event(&app, "v", "click", Some(&d.to_string())).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    // The record is immediate: results, webhooks and the dashboard see the conversion now.
    assert_eq!(outcome(&pool, d).await, (Some(1), true));
    let (version, m) = model(&pool).await;
    assert_eq!((version, m.n), (0, 0), "the model learned a click before the decision's window closed");

    worker::run_once(&pool, closed()).await.unwrap();
    let (version, m) = model(&pool).await;
    assert_eq!((version, m.n), (1, 1), "the click was not learned when the window closed");
    let variant: i64 = sqlx::query_scalar("SELECT variant_id FROM decisions WHERE id = $1").bind(d).fetch_one(&pool).await.unwrap();
    assert!(dot(&m.arm(variant).mean, &X_RET) > 0.1, "the arm did not learn the conversion");
    let reason: String = sqlx::query_scalar("SELECT reason FROM model_versions WHERE version = 1").fetch_one(&pool).await.unwrap();
    assert_eq!(reason, "learn: conversion");

    // And only once: later passes change nothing.
    worker::run_once(&pool, closed() + Duration::hours(1)).await.unwrap();
    assert_eq!(model(&pool).await.0, 1, "the conversion was learned twice");
}

/// The E4 symptom itself: 100 exposures, 5 clicks (5%). While the windows are open the model must
/// not have absorbed the clicks alone; once they close, the estimate is near 5%, not near 1.
#[sqlx::test(migrations = false)]
async fn pending_exposures_do_not_inflate_the_estimate(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let mut ds = vec![];
    for i in 0..100 {
        let s = format!("v{i}");
        let d: Uuid = decide(&app, &s, true).await.json()["decision_id"].as_str().unwrap().parse().unwrap();
        event(&app, &s, "exposure", Some(&d.to_string())).await;
        if i % 20 == 0 {
            event(&app, &s, "click", Some(&d.to_string())).await;
        }
        ds.push(d);
    }
    let s = worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(s.credited, 5);
    let (_, m) = model(&pool).await;
    let arms: Vec<i64> = sqlx::query_scalar("SELECT DISTINCT variant_id FROM decisions").fetch_all(&pool).await.unwrap();
    for a in &arms {
        let rate = sigmoid(dot(&m.arm(*a).mean, &X_RET));
        assert!(rate <= 0.5 + 1e-12, "arm {a}: inferred {rate:.3} from clicks alone while 95 misses are pending");
    }

    worker::run_once(&pool, closed()).await.unwrap();
    let (_, m) = model(&pool).await;
    assert_eq!(m.n, 100);
    for a in &arms {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM decisions WHERE variant_id = $1").bind(a).fetch_one(&pool).await.unwrap();
        let rate = sigmoid(dot(&m.arm(*a).mean, &X_RET));
        if n >= 30 {
            assert!(rate < 0.15, "arm {a}: {rate:.3} after {n} closed decisions at 5%");
        }
    }
}

/// A decision credited by the previous worker (which learned at the click) has no pending lesson,
/// so the new worker must not teach it again at close: the switch-over is exact.
#[sqlx::test(migrations = false)]
async fn a_credit_learned_by_the_old_worker_is_not_learned_again(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let d = exposed(&app, &pool, "v").await;
    // What the old worker left behind: credited and settled at the click (and learned then).
    sqlx::query("UPDATE decisions SET outcome = 1, outcome_at = now(), settled_at = now(), credit = 1 WHERE id = $1")
        .bind(d)
        .execute(&pool)
        .await
        .unwrap();
    worker::run_once(&pool, closed()).await.unwrap();
    assert_eq!(model(&pool).await.0, 0, "an old-worker credit was learned a second time");
}

#[sqlx::test(migrations = false)]
async fn a_holdout_credit_is_never_learned(pool: PgPool) {
    setup_demo(&pool).await;
    set_holdout(&pool, 0.5).await;
    let app = app(&pool);
    let mut found = None;
    for i in 0..100 {
        let s = format!("v{i}");
        let r = decide(&app, &s, true).await.json();
        if r["explanation"]["in_holdout"].as_bool().unwrap() {
            found = Some((s, r["decision_id"].as_str().unwrap().parse::<Uuid>().unwrap()));
            break;
        }
    }
    let (s, d) = found.expect("no holdout visitor in 100");
    event(&app, &s, "exposure", Some(&d.to_string())).await;
    event(&app, &s, "click", Some(&d.to_string())).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    worker::run_once(&pool, closed()).await.unwrap();
    assert_eq!(outcome(&pool, d).await, (Some(1), true));
    assert_eq!(model(&pool).await.0, 0, "the control group's conversion taught the model");
}

/// Under `linear` the share is what is learned at close, decision by decision in created order.
#[sqlx::test(migrations = false)]
async fn a_shared_credit_is_learned_at_close_with_its_share(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    let f = file_from(json!({
        "site": "demo", "origin": "http://localhost:5173",
        "slots": [{ "key": "hero", "selector": "#hero", "metric": "click", "attribution": "linear",
                    "variants": [ { "key": "baseline", "baseline": true }, { "key": "gift", "html": "<h1>Gift sets</h1>" } ] }]
    }));
    pratibimba::site_file::apply(&pool, &f).await.unwrap();
    set_holdout(&pool, 0.0).await;
    let app = app(&pool);
    for _ in 0..3 {
        sqlx::query("UPDATE decisions SET created_at = created_at - interval '1 minute'").execute(&pool).await.unwrap();
        exposed(&app, &pool, "v").await;
    }
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(model(&pool).await.0, 0);
    worker::run_once(&pool, closed()).await.unwrap();

    let rows: Vec<(i64, Value, f64)> =
        sqlx::query_as("SELECT variant_id, features, credit FROM decisions ORDER BY created_at, id").fetch_all(&pool).await.unwrap();
    assert_eq!(rows.len(), 3);
    let mut want = LinTs::default();
    for (v, features, credit) in &rows {
        assert!((credit - 1.0 / 3.0).abs() < 1e-9, "credit {credit}");
        pratibimba::stated::learn_credit(&mut want, *v, &from_named(features).unwrap(), features, *credit);
    }
    let (_, got) = model(&pool).await;
    assert_eq!(got.n, 3);
    let got_arms: BTreeMap<i64, _> = got.arms.iter().map(|(k, a)| (*k, a.mean)).collect();
    let want_arms: BTreeMap<i64, _> = want.arms.iter().map(|(k, a)| (*k, a.mean)).collect();
    assert_eq!(got_arms.keys().collect::<Vec<_>>(), want_arms.keys().collect::<Vec<_>>());
    for (k, w) in &want_arms {
        for i in 0..4 {
            assert!((got_arms[k][i] - w[i]).abs() < 1e-9, "arm {k} weight {i}: {} vs {}", got_arms[k][i], w[i]);
        }
    }
}

/// Four workers closing the same 200 credited decisions at once learn each exactly once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_workers_learn_each_credit_once_at_close() {
    const N: usize = 200;
    for _ in 0..3 {
        let db = FreshDb::new().await;
        let pool = db.pool.clone();
        setup_learning(&pool).await;
        let app = app(&pool);
        let mut ids = vec![];
        for i in 0..N {
            let s = format!("v{i}");
            let d: Uuid = decide(&app, &s, true).await.json()["decision_id"].as_str().unwrap().parse().unwrap();
            ids.push((s, d));
        }
        sqlx::query("UPDATE decisions SET exposed_at = created_at").execute(&pool).await.unwrap();
        for (s, d) in &ids {
            event(&app, s, "click", Some(&d.to_string())).await;
        }
        worker::run_once(&pool, Utc::now()).await.unwrap();
        assert_eq!(model(&pool).await.1.n, 0);
        let barrier = Arc::new(Barrier::new(4));
        let mut handles = vec![];
        for _ in 0..4 {
            let (pool, b) = (pool.clone(), barrier.clone());
            handles.push(tokio::spawn(async move {
                b.wait().await;
                worker::run_once_with(&pool, closed(), 10).await
            }));
        }
        for h in handles {
            h.await.unwrap().expect("worker failed");
        }
        let (version, m) = model(&pool).await;
        assert_eq!((version, m.n), (N as i64, N as u64), "credits learned other than once each");
        db.drop().await;
    }
}
