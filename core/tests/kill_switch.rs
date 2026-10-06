//! The site-wide kill switch: one call pauses every slot to its baseline, one call resumes.

mod common;
use common::*;
use serde_json::json;
use sqlx::PgPool;

async fn served(app: &axum::Router, prefix: &str, n: usize) -> std::collections::HashSet<String> {
    let mut seen = std::collections::HashSet::new();
    for i in 0..n {
        seen.insert(decide(app, &format!("{prefix}{i}"), false).await.json()["variant"].as_str().unwrap().to_string());
    }
    seen
}

#[sqlx::test(migrations = false)]
async fn pause_serves_only_the_baseline_and_resume_undoes_it(pool: PgPool) {
    setup_demo(&pool).await;
    set_holdout(&pool, 0.0).await;
    let app = admin_app(&pool);
    assert!(served(&app, "a", 60).await.len() > 1, "the demo should serve more than the baseline before the pause");

    let r = api_post(&app, "/api/sites/demo/pause", json!({ "actor": "Ritwik", "paused": true })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.json()["paused"]["by"], "Ritwik");
    let baseline: String = sqlx::query_scalar("SELECT key FROM variants WHERE is_baseline").fetch_one(&pool).await.unwrap();
    assert_eq!(served(&app, "b", 60).await, [baseline].into_iter().collect());
    assert_eq!(api_get(&app, "/api/sites/demo/overview").await.json()["paused"]["by"], "Ritwik");

    let r = api_post(&app, "/api/sites/demo/pause", json!({ "actor": "Ritwik", "paused": false })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert!(r.json()["paused"].is_null());
    assert!(served(&app, "c", 60).await.len() > 1, "resume should bring the variants back");
}

#[sqlx::test(migrations = false)]
async fn pause_needs_a_named_actor(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    let r = api_post(&app, "/api/sites/demo/pause", json!({ "actor": " ", "paused": true })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM sites WHERE paused_at IS NOT NULL").await, 0);
}
