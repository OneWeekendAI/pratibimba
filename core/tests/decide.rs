mod common;
use common::*;
use pratibimba::bandit::LinTs;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
async fn decide_returns_an_approved_variant_and_records_it(pool: PgPool) {
    setup_demo(&pool).await;
    let r = decide(&app(&pool), "v1", true).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let d = r.json();
    assert!(["baseline", "gift"].contains(&d["variant"].as_str().unwrap()));
    let id: uuid::Uuid = d["decision_id"].as_str().unwrap().parse().unwrap();
    let (features, pool_ids): (serde_json::Value, Vec<i64>) =
        sqlx::query_as("SELECT features, pool FROM decisions WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(features["returning"], 1.0);
    assert_eq!(pool_ids.len(), 2);
}

#[sqlx::test(migrations = false)]
async fn decide_never_serves_a_draft(pool: PgPool) {
    setup_demo(&pool).await;
    insert_variant(&pool, "draft_one", "draft").await;
    let app = app(&pool);
    for i in 0..60 {
        let d = decide(&app, &format!("v{i}"), false).await.json();
        assert_ne!(d["variant"], "draft_one", "a draft reached a visitor");
    }
}

#[sqlx::test(migrations = false)]
async fn decide_serves_both_arms_under_the_prior(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app(&pool);
    let mut seen = std::collections::HashSet::new();
    for i in 0..60 {
        seen.insert(decide(&app, &format!("v{i}"), false).await.json()["variant"].as_str().unwrap().to_string());
    }
    assert_eq!(seen.len(), 2, "{seen:?}");
}

#[sqlx::test(migrations = false)]
async fn decide_is_sticky_within_ttl(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app(&pool);
    let a = decide(&app, "same", false).await.json();
    let b = decide(&app, "same", false).await.json();
    assert_eq!(a["decision_id"], b["decision_id"]);
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions").await, 1);
}

#[sqlx::test(migrations = false)]
async fn decide_rerolls_after_ttl(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app(&pool);
    let a = decide(&app, "same", false).await.json();
    sqlx::query("UPDATE decisions SET created_at = now() - interval '61 seconds'").execute(&pool).await.unwrap();
    let b = decide(&app, "same", false).await.json();
    assert_ne!(a["decision_id"], b["decision_id"]);
}

#[sqlx::test(migrations = false)]
async fn decide_rejects_demographic_context(pool: PgPool) {
    setup_demo(&pool).await;
    let r = post(&app(&pool), "/decide", json!({ "site": "demo", "slot": "hero", "subject": "v", "ctx": { "returning": true, "gender": "f" } })).await;
    assert!(r.status.is_client_error(), "{} {}", r.status, r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions").await, 0);
}

#[sqlx::test(migrations = false)]
async fn decide_rejects_unknown_top_level_field(pool: PgPool) {
    setup_demo(&pool).await;
    let r = post(&app(&pool), "/decide", json!({ "site": "demo", "slot": "hero", "subject": "v", "age": 40 })).await;
    assert!(r.status.is_client_error(), "{} {}", r.status, r.text);
}

#[sqlx::test(migrations = false)]
async fn decide_unknown_slot_is_404(pool: PgPool) {
    setup_demo(&pool).await;
    let r = post(&app(&pool), "/decide", json!({ "site": "demo", "slot": "nope", "subject": "v" })).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = false)]
async fn decide_rejects_oversized_subject(pool: PgPool) {
    setup_demo(&pool).await;
    let r = decide(&app(&pool), &"x".repeat(300), false).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = false)]
async fn decide_explanation_replays(pool: PgPool) {
    setup_demo(&pool).await;
    // Give the model a non-trivial state so a replay against the prior would differ.
    let mut m = LinTs::default();
    let gift: i64 = sqlx::query_scalar("SELECT id FROM variants WHERE key='gift'").fetch_one(&pool).await.unwrap();
    for _ in 0..5 {
        m.update(gift, &[1.0, 1.0, 0.0, 0.0], true);
    }
    sqlx::query("UPDATE models SET state = $1, version = 5").bind(serde_json::to_value(&m).unwrap()).execute(&pool).await.unwrap();
    let d = decide(&app(&pool), "v", true).await.json();
    let seed: u64 = d["explanation"]["seed"].as_str().unwrap().parse().unwrap();
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM variants ORDER BY id").fetch_all(&pool).await.unwrap();
    let (_, scores) = m.choose(&ids, &[1.0, 1.0, 0.0, 0.0], seed);
    let replayed = scores.iter().find(|(id, _)| *id == gift).unwrap().1;
    assert_eq!(d["explanation"]["scores"]["gift"].as_f64().unwrap(), replayed);
    assert_eq!(d["explanation"]["policy_version"], 5);
}
