//! Auto-rollback: a variant that clearly loses to the holdout is paused, its model restored.

mod common;
use chrono::Utc;
use common::*;
use pratibimba::{bandit::LinTs, guard};
use sqlx::PgPool;

/// The model at version 3 when `gift` was approved, then two more (bad) versions after it.
async fn history(pool: &PgPool) -> (LinTs, i64) {
    let slot: i64 = sqlx::query_scalar("SELECT id FROM slots WHERE key = 'hero'").fetch_one(pool).await.unwrap();
    let gift = variant_id(pool, "gift").await;
    let mut m = LinTs::default();
    let mut checkpoint = None;
    for v in 1..=5 {
        m.update(gift, &[1.0, 0.0, 0.0, 0.0], true);
        let state = serde_json::to_value(&m).unwrap();
        sqlx::query("INSERT INTO model_versions (slot_id, version, state, reason) VALUES ($1, $2, $3, 'test')")
            .bind(slot)
            .bind(v as i64)
            .bind(&state)
            .execute(pool)
            .await
            .unwrap();
        if v == 3 {
            checkpoint = Some(m.clone());
        }
    }
    sqlx::query("UPDATE models SET state = $1, version = 5").bind(serde_json::to_value(&m).unwrap()).execute(pool).await.unwrap();
    sqlx::query("UPDATE variants SET approved_model_version = 3 WHERE key = 'gift'").execute(pool).await.unwrap();
    (checkpoint.unwrap(), slot)
}

async fn losing(pool: &PgPool) -> (LinTs, i64) {
    setup_demo(pool).await;
    let h = history(pool).await;
    seed_decisions(pool, "gift", false, 1000, 20, 60).await; // 2%
    seed_decisions(pool, "baseline", true, 1000, 100, 60).await; // 10%
    h
}

async fn status(pool: &PgPool, key: &str) -> String {
    sqlx::query_scalar("SELECT status FROM variants WHERE key = $1").bind(key).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn losing_variant_is_paused_with_a_certificate(pool: PgPool) {
    losing(&pool).await;
    let paused = guard::run(&pool, Utc::now()).await.unwrap();
    assert_eq!(paused.len(), 1, "{paused:?}");
    assert_eq!(status(&pool, "gift").await, "paused");
    let cert: serde_json::Value = sqlx::query_scalar("SELECT pause_reason FROM variants WHERE key = 'gift'").fetch_one(&pool).await.unwrap();
    assert_eq!(cert["variant"]["n"], 1000, "{cert}");
    assert!(cert["lift"]["hi"].as_f64().unwrap() < -0.01, "{cert}");
}

#[sqlx::test(migrations = false)]
async fn rollback_restores_the_approval_version(pool: PgPool) {
    let (checkpoint, _) = losing(&pool).await;
    guard::run(&pool, Utc::now()).await.unwrap();
    let (version, state): (i64, serde_json::Value) = sqlx::query_as("SELECT version, state FROM models").fetch_one(&pool).await.unwrap();
    assert_eq!(version, 6, "the restore must be a new version, not a rewrite of history");
    assert_eq!(serde_json::from_value::<LinTs>(state).unwrap(), checkpoint, "the model was not restored to the approval version");
    assert_eq!(count(&pool, "SELECT count(*) FROM model_versions WHERE version = 6 AND reason LIKE 'rollback:%'").await, 1);
}

#[sqlx::test(migrations = false)]
async fn variant_inside_the_margin_is_not_paused(pool: PgPool) {
    setup_demo(&pool).await;
    // The estimate is below zero (9.5% vs 10%), but the interval spans it.
    seed_decisions(&pool, "gift", false, 1000, 95, 60).await;
    seed_decisions(&pool, "baseline", true, 1000, 100, 60).await;
    assert!(guard::run(&pool, Utc::now()).await.unwrap().is_empty());
    assert_eq!(status(&pool, "gift").await, "approved");
}

#[sqlx::test(migrations = false)]
async fn rollback_waits_for_minimum_evidence(pool: PgPool) {
    setup_demo(&pool).await;
    sqlx::query("UPDATE sites SET rollback_min_exposures = 500").execute(&pool).await.unwrap();
    // 0 of 400 against 30%: damning, but under the minimum.
    seed_decisions(&pool, "gift", false, 400, 0, 60).await;
    seed_decisions(&pool, "baseline", true, 1000, 300, 60).await;
    assert!(guard::run(&pool, Utc::now()).await.unwrap().is_empty());
    assert_eq!(status(&pool, "gift").await, "approved");
}

#[sqlx::test(migrations = false)]
async fn rollback_writes_a_learning(pool: PgPool) {
    losing(&pool).await;
    guard::run(&pool, Utc::now()).await.unwrap();
    let sentence: String = sqlx::query_scalar("SELECT sentence FROM learnings WHERE kind = 'rollback'").fetch_one(&pool).await.unwrap();
    assert!(sentence.contains("`gift`") && sentence.contains("2.0%") && sentence.contains("10.0%"), "{sentence}");
    assert!(sentence.contains("restored to version 3"), "{sentence}");
}

#[sqlx::test(migrations = false)]
async fn baseline_is_never_paused(pool: PgPool) {
    setup_demo(&pool).await;
    seed_decisions(&pool, "baseline", false, 1000, 10, 60).await;
    seed_decisions(&pool, "baseline", true, 1000, 100, 60).await;
    guard::run(&pool, Utc::now()).await.unwrap();
    assert_eq!(status(&pool, "baseline").await, "approved");
}

#[sqlx::test(migrations = false)]
async fn winner_gets_one_learning(pool: PgPool) {
    setup_demo(&pool).await;
    seed_decisions(&pool, "gift", false, 3000, 600, 60).await;
    seed_decisions(&pool, "baseline", true, 3000, 150, 60).await;
    guard::run(&pool, Utc::now()).await.unwrap();
    guard::run(&pool, Utc::now()).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM learnings WHERE kind = 'winning'").await, 1);
}

#[sqlx::test(migrations = false)]
async fn the_worker_loop_runs_the_guard(pool: PgPool) {
    losing(&pool).await;
    let p = pool.clone();
    let handle = tokio::spawn(async move {
        pratibimba::worker::run_loop_with(p, std::time::Duration::from_millis(100), std::time::Duration::from_millis(100)).await
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while status(&pool, "gift").await != "paused" {
        assert!(std::time::Instant::now() < deadline, "the running worker never paused the losing variant");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    handle.abort();
}
