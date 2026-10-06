//! `GET /api/decisions/:id`: replay against the stored version, the counterfactual, the timeline.

mod common;
use chrono::Utc;
use common::*;
use pratibimba::worker;
use serde_json::Value;
use sqlx::PgPool;

/// A treated decision, exposed and clicked, then more learning so the model moves on.
async fn learned_then_moved(pool: &PgPool) -> (axum::Router, String) {
    setup_learning(pool).await;
    let app = admin_app(pool);
    let d = decide(&app, "r1", true).await.json();
    let id = d["decision_id"].as_str().unwrap().to_string();
    event(&app, "r1", "exposure", Some(&id)).await;
    event(&app, "r1", "click", Some(&id)).await;
    worker::run_once(pool, Utc::now()).await.unwrap();
    // Twenty more conversions after it, so the current model is not the one that scored it.
    for i in 0..20 {
        let s = format!("m{i}");
        let o = decide(&app, &s, true).await.json();
        let oid = o["decision_id"].as_str().unwrap().to_string();
        event(&app, &s, "exposure", Some(&oid)).await;
        event(&app, &s, "click", Some(&oid)).await;
    }
    worker::run_once(pool, Utc::now()).await.unwrap();
    (app, id)
}

async fn detail(app: &axum::Router, id: &str) -> Value {
    let r = api_get(app, &format!("/api/decisions/{id}")).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json()
}

#[sqlx::test(migrations = false)]
async fn replay_uses_the_version_it_was_scored_with(pool: PgPool) {
    let (app, id) = learned_then_moved(&pool).await;
    assert!(count(&pool, "SELECT version FROM models").await >= 10);
    let d = detail(&app, &id).await;
    assert_eq!(d["policy_version"], 0);
    assert_eq!(d["replay_matches"], true, "replay did not reproduce the stored scores: {d}");
}

#[sqlx::test(migrations = false)]
async fn decision_detail_has_features_scores_and_timeline(pool: PgPool) {
    let (app, id) = learned_then_moved(&pool).await;
    let d = detail(&app, &id).await;
    assert_eq!(d["features"]["returning"], 1.0);
    assert_eq!(d["scores"].as_array().unwrap().len(), 2);
    let names: Vec<&str> = d["timeline"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["exposure", "click"], "{d}");
    assert!(d["sentence"].as_str().unwrap().starts_with("We showed `"), "{d}");
    assert!(d["sentence"].as_str().unwrap().contains("They converted (`click`)"), "{d}");
}

#[sqlx::test(migrations = false)]
async fn counterfactual_of_treated_is_the_baseline(pool: PgPool) {
    let (app, id) = learned_then_moved(&pool).await;
    let d = detail(&app, &id).await;
    assert_eq!(d["in_holdout"], false);
    assert_eq!(d["counterfactual"]["kind"], "baseline");
    assert_eq!(d["counterfactual"]["variant"], "baseline");
    let r = d["counterfactual"]["inferred_rate"].as_f64().unwrap();
    assert!(r > 0.0 && r < 1.0, "{d}");
}

#[sqlx::test(migrations = false)]
async fn counterfactual_of_holdout_is_the_policy_pick(pool: PgPool) {
    setup_demo(&pool).await;
    set_holdout(&pool, 0.5).await;
    let app = admin_app(&pool);
    // Make the policy prefer gift strongly, so its pick for a holdout visitor is gift.
    let gift = variant_id(&pool, "gift").await;
    let mut m = pratibimba::bandit::LinTs::default();
    for _ in 0..300 {
        m.update(gift, &[1.0, 0.0, 0.0, 0.0], true);
    }
    let state = serde_json::to_value(&m).unwrap();
    sqlx::query("UPDATE models SET state = $1, version = 1").bind(&state).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO model_versions (slot_id, version, state, reason) SELECT slot_id, 1, $1, 'test' FROM models").bind(&state).execute(&pool).await.unwrap();
    for i in 0..60 {
        let d = decide(&app, &format!("h{i}"), false).await.json();
        if d["explanation"]["in_holdout"] == true {
            let x = detail(&app, d["decision_id"].as_str().unwrap()).await;
            assert_eq!(x["variant"], "baseline");
            assert_eq!(x["counterfactual"]["kind"], "policy");
            assert_eq!(x["counterfactual"]["variant"], "gift", "{x}");
            return;
        }
    }
    panic!("no holdout visitor in 60");
}

#[sqlx::test(migrations = false)]
async fn unknown_decision_is_404(pool: PgPool) {
    setup_demo(&pool).await;
    let r = api_get(&admin_app(&pool), &format!("/api/decisions/{}", uuid::Uuid::new_v4())).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = false)]
async fn decisions_list_filters_by_outcome(pool: PgPool) {
    let (app, id) = learned_then_moved(&pool).await;
    let _ = decide(&app, "never-shown", false).await;
    // Two settled misses, which "converted" must leave out.
    seed_decisions(&pool, "gift", false, 2, 0, 60).await;
    sqlx::query("UPDATE decisions SET outcome = 0, settled_at = now() WHERE subject LIKE 'seed-%'").execute(&pool).await.unwrap();
    let conv = api_get(&app, "/api/sites/demo/decisions?outcome=converted").await.json();
    assert_eq!(conv.as_array().unwrap().len(), 21);
    let missed = api_get(&app, "/api/sites/demo/decisions?outcome=missed").await.json();
    assert_eq!(missed.as_array().unwrap().len(), 2);
    assert!(conv.as_array().unwrap().iter().any(|d| d["id"] == id));
    let unexposed = api_get(&app, "/api/sites/demo/decisions?outcome=unexposed").await.json();
    assert_eq!(unexposed.as_array().unwrap().len(), 1, "{unexposed}");
}

#[sqlx::test(migrations = false)]
async fn manual_rollback_restores_a_version(pool: PgPool) {
    let (app, _) = learned_then_moved(&pool).await;
    let slot: i64 = sqlx::query_scalar("SELECT slot_id FROM models").fetch_one(&pool).await.unwrap();
    let before: i64 = count(&pool, "SELECT version FROM models").await;
    let r = api_post(&app, &format!("/api/models/{slot}/rollback"), serde_json::json!({ "version": 1, "actor": "Ritwik" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let (v, state): (i64, Value) = sqlx::query_as("SELECT version, state FROM models").fetch_one(&pool).await.unwrap();
    let v1: Value = sqlx::query_scalar("SELECT state FROM model_versions WHERE version = 1").fetch_one(&pool).await.unwrap();
    assert_eq!(v, before + 1);
    assert_eq!(state, v1, "the model was not restored");
}
