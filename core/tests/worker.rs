mod common;
use axum::Router;
use chrono::{Duration, Utc};
use common::*;
use pratibimba::{bandit::LinTs, worker};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::Barrier;
use uuid::Uuid;

const X_RET: [f64; 4] = [1.0, 1.0, 0.0, 0.0];

async fn decided(app: &Router, subject: &str) -> Uuid {
    decide(app, subject, true).await.json()["decision_id"].as_str().unwrap().parse().unwrap()
}

async fn exposed(app: &Router, pool: &PgPool, subject: &str) -> Uuid {
    let d = decided(app, subject).await;
    event(app, subject, "exposure", Some(&d.to_string())).await;
    worker::run_once(pool, Utc::now()).await.unwrap();
    d
}

async fn model(pool: &PgPool) -> (i64, LinTs) {
    let (v, s): (i64, serde_json::Value) = sqlx::query_as("SELECT version, state FROM models").fetch_one(pool).await.unwrap();
    (v, serde_json::from_value(s).unwrap())
}

async fn outcome(pool: &PgPool, d: Uuid) -> (Option<i16>, bool) {
    sqlx::query_as("SELECT outcome, settled_at IS NOT NULL FROM decisions WHERE id = $1").bind(d).fetch_one(pool).await.unwrap()
}

async fn variant_of(pool: &PgPool, d: Uuid) -> i64 {
    sqlx::query_scalar("SELECT variant_id FROM decisions WHERE id = $1").bind(d).fetch_one(pool).await.unwrap()
}

fn dot(a: &[f64; 4], b: &[f64; 4]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[sqlx::test(migrations = false)]
async fn exposure_marks_decision_exposed(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let d = exposed(&app, &pool, "v").await;
    let n = count(&pool, &format!("SELECT count(*) FROM decisions WHERE id = '{d}' AND exposed_at IS NOT NULL")).await;
    assert_eq!(n, 1);
}

#[sqlx::test(migrations = false)]
async fn goal_with_decision_id_credits_and_learns(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let d = exposed(&app, &pool, "v").await;
    event(&app, "v", "click", Some(&d.to_string())).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(outcome(&pool, d).await, (Some(1), true));
    let (version, m) = model(&pool).await;
    assert_eq!(version, 1);
    assert!(dot(&m.arm(variant_of(&pool, d).await).mean, &X_RET) > 0.1, "the arm did not learn the conversion");
}

#[sqlx::test(migrations = false)]
async fn second_goal_does_not_learn_twice(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let d = exposed(&app, &pool, "v").await;
    event(&app, "v", "click", Some(&d.to_string())).await;
    event(&app, "v", "click", Some(&d.to_string())).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(model(&pool).await.0, 1);
}

#[sqlx::test(migrations = false)]
async fn goal_on_unexposed_decision_is_not_credited(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let d = decided(&app, "v").await;
    event(&app, "v", "click", Some(&d.to_string())).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(outcome(&pool, d).await, (None, false));
    assert_eq!(model(&pool).await.0, 0);
}

#[sqlx::test(migrations = false)]
async fn goal_after_window_is_not_credited(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let d = exposed(&app, &pool, "v").await;
    sqlx::query("UPDATE decisions SET created_at = now() - interval '2 hours'").execute(&pool).await.unwrap();
    event(&app, "v", "click", Some(&d.to_string())).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(outcome(&pool, d).await, (Some(0), true), "a click two hours late was credited");
}

#[sqlx::test(migrations = false)]
async fn goal_with_wrong_metric_is_not_credited(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let d = exposed(&app, &pool, "v").await;
    event(&app, "v", "add_to_cart", Some(&d.to_string())).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(outcome(&pool, d).await, (None, false));
}

#[sqlx::test(migrations = false)]
async fn goal_from_other_subject_is_not_credited(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let d = exposed(&app, &pool, "victim").await;
    event(&app, "forger", "click", Some(&d.to_string())).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(outcome(&pool, d).await, (None, false));
}

#[sqlx::test(migrations = false)]
async fn last_touch_credits_latest_exposed_decision(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let older = exposed(&app, &pool, "v").await;
    sqlx::query("UPDATE decisions SET created_at = now() - interval '10 minutes'").execute(&pool).await.unwrap();
    let newer = exposed(&app, &pool, "v").await;
    assert_ne!(older, newer);
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(outcome(&pool, newer).await.0, Some(1));
    assert_eq!(outcome(&pool, older).await.0, None);
}

#[sqlx::test(migrations = false)]
async fn settle_expired_exposed_decision_learns_negative(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let d = exposed(&app, &pool, "v").await;
    worker::run_once(&pool, Utc::now() + Duration::hours(1)).await.unwrap();
    assert_eq!(outcome(&pool, d).await, (Some(0), true));
    let (version, m) = model(&pool).await;
    assert_eq!(version, 1);
    assert!(dot(&m.arm(variant_of(&pool, d).await).mean, &X_RET) < -0.1, "the arm did not learn the miss");
}

#[sqlx::test(migrations = false)]
async fn settle_leaves_open_window_alone(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let d = exposed(&app, &pool, "v").await;
    worker::run_once(&pool, Utc::now() + Duration::minutes(29)).await.unwrap();
    assert_eq!(outcome(&pool, d).await, (None, false));
}

#[sqlx::test(migrations = false)]
async fn settle_skips_unexposed_without_learning(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let d = decided(&app, "v").await;
    worker::run_once(&pool, Utc::now() + Duration::hours(1)).await.unwrap();
    assert_eq!(outcome(&pool, d).await, (None, true));
    assert_eq!(model(&pool).await.0, 0);
}

#[sqlx::test(migrations = false)]
async fn late_processed_goal_inside_window_still_credits(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let d = decided(&app, "v").await;
    event(&app, "v", "exposure", Some(&d.to_string())).await;
    event(&app, "v", "click", Some(&d.to_string())).await;
    // The worker was down; it first runs two hours later.
    worker::run_once(&pool, Utc::now() + Duration::hours(2)).await.unwrap();
    assert_eq!(outcome(&pool, d).await.0, Some(1));
}

#[sqlx::test(migrations = false)]
async fn outbox_is_drained(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    event(&app, "v", "click", None).await;
    event(&app, "v", "missed", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM pending_events").await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_workers_lose_no_updates() {
    const N: usize = 200;
    for _ in 0..3 {
        let db = FreshDb::new().await;
        let pool = db.pool.clone();
        setup_learning(&pool).await;
        let app = app(&pool);
        let mut ids = vec![];
        for i in 0..N {
            ids.push((format!("v{i}"), decided(&app, &format!("v{i}")).await));
        }
        sqlx::query("UPDATE decisions SET exposed_at = created_at").execute(&pool).await.unwrap();
        for (subject, d) in &ids {
            event(&app, subject, "click", Some(&d.to_string())).await;
        }
        let barrier = Arc::new(Barrier::new(4));
        let mut handles = vec![];
        for _ in 0..4 {
            let (pool, b) = (pool.clone(), barrier.clone());
            handles.push(tokio::spawn(async move {
                b.wait().await;
                worker::run_once_with(&pool, Utc::now(), 10).await
            }));
        }
        for h in handles {
            h.await.unwrap().expect("worker failed");
        }
        assert_eq!(model(&pool).await.1.n, N as u64, "lost model updates");
        assert_eq!(count(&pool, "SELECT count(*) FROM pending_events").await, 0);
        db.drop().await;
    }
}

#[sqlx::test(migrations = false)]
async fn notify_wakes_the_loop(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let loop_pool = pool.clone();
    let handle = tokio::spawn(async move { worker::run_loop(loop_pool, std::time::Duration::from_secs(60)).await });
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let d = decided(&app, "v").await;
    event(&app, "v", "exposure", Some(&d.to_string())).await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if count(&pool, "SELECT count(*) FROM decisions WHERE exposed_at IS NOT NULL").await == 1 {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the worker did not wake on NOTIFY");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    handle.abort();
}

#[sqlx::test(migrations = false)]
async fn every_update_appends_a_model_version(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let d = exposed(&app, &pool, "v").await;
    event(&app, "v", "click", Some(&d.to_string())).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    let versions: Vec<(i64, serde_json::Value, String)> =
        sqlx::query_as("SELECT version, state, reason FROM model_versions ORDER BY version").fetch_all(&pool).await.unwrap();
    assert_eq!(versions.iter().map(|v| v.0).collect::<Vec<_>>(), vec![0, 1], "{versions:?}");
    let (_, current) = model(&pool).await;
    let stored: LinTs = serde_json::from_value(versions[1].1.clone()).unwrap();
    assert_eq!(stored, current, "the history row is not the state that was written");
    assert_eq!(versions[1].2, "learn: conversion");
}

/// `pratibimba worker --once` (the Cloud Run job): one full pass, then exit 0. It must drain the
/// outbox and learn, and it must not fall into the forever loop.
#[sqlx::test(migrations = false)]
async fn worker_once_drains_the_outbox_and_exits(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let d = decided(&app, "v").await;
    event(&app, "v", "exposure", Some(&d.to_string())).await;
    event(&app, "v", "click", Some(&d.to_string())).await;
    assert!(count(&pool, "SELECT count(*) FROM pending_events").await > 0, "nothing queued to drain");
    let db: String = sqlx::query_scalar("SELECT current_database()").fetch_one(&pool).await.unwrap();
    let base = std::env::var("DATABASE_URL").unwrap();
    let url = format!("{}/{db}", base.rsplit_once('/').unwrap().0);
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_pratibimba"))
        .args(["worker", "--once"])
        .env("PRB_DB", &url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if start.elapsed() > std::time::Duration::from_secs(30) {
            child.kill().ok();
            panic!("worker --once did not exit within 30s");
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    assert!(status.success(), "worker --once exited with {status}");
    assert_eq!(count(&pool, "SELECT count(*) FROM pending_events").await, 0, "the outbox was not drained");
    assert_eq!(outcome(&pool, d).await, (Some(1), true));
    assert_eq!(model(&pool).await.0, 1, "the model did not learn");
}
