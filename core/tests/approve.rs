//! The approval gate: drafts are never served; one variant, one named approver, at most 50%.

mod common;
use common::*;
use serde_json::json;
use sqlx::PgPool;

async fn draft(app: &axum::Router, key: &str, html: &str) -> Resp {
    api_post(app, "/api/sites/demo/slots/hero/variants", json!({ "key": key, "html": html })).await
}

async fn draft_id(app: &axum::Router, key: &str) -> i64 {
    let r = draft(app, key, "<h2>Free shipping this week</h2>").await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    r.json()["id"].as_i64().unwrap()
}

async fn served(app: &axum::Router, n: usize) -> std::collections::HashSet<String> {
    let mut seen = std::collections::HashSet::new();
    for i in 0..n {
        seen.insert(decide(app, &format!("a{i}"), false).await.json()["variant"].as_str().unwrap().to_string());
    }
    seen
}

#[sqlx::test(migrations = false)]
async fn draft_can_be_created_and_is_not_served(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    draft_id(&app, "ship").await;
    let status: String = sqlx::query_scalar("SELECT status FROM variants WHERE key = 'ship'").fetch_one(&pool).await.unwrap();
    assert_eq!(status, "draft");
    assert!(!served(&app, 80).await.contains("ship"), "a draft reached a visitor");
}

#[sqlx::test(migrations = false)]
async fn approve_makes_a_draft_servable(pool: PgPool) {
    setup_demo(&pool).await;
    set_holdout(&pool, 0.0).await;
    let app = admin_app(&pool);
    let id = draft_id(&app, "ship").await;
    let r = api_post(&app, &format!("/api/variants/{id}/approve"), json!({ "approver": "Ritwik", "traffic_share": 0.5 })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert!(served(&app, 120).await.contains("ship"), "an approved variant was never served");
}

#[sqlx::test(migrations = false)]
async fn approve_refuses_share_above_half(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    let id = draft_id(&app, "ship").await;
    let r = api_post(&app, &format!("/api/variants/{id}/approve"), json!({ "approver": "Ritwik", "traffic_share": 0.6 })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM variants WHERE status = 'draft'").await, 1);
}

#[sqlx::test(migrations = false)]
async fn approve_refuses_failing_technical_check(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    let r = draft(&app, "xss", "<img src=x onerror=alert(1)>").await;
    let id = r.json()["id"].as_i64().unwrap();
    let r = api_post(&app, &format!("/api/variants/{id}/approve"), json!({ "approver": "Ritwik", "traffic_share": 0.1 })).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text);
    assert!(r.text.contains("onerror"), "{}", r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM variants WHERE key = 'xss' AND status = 'draft'").await, 1);
}

#[sqlx::test(migrations = false)]
async fn approve_refuses_a_forbidden_claim(pool: PgPool) {
    setup_demo(&pool).await;
    sqlx::query("UPDATE sites SET forbidden_claims = '{free shipping}'").execute(&pool).await.unwrap();
    let app = admin_app(&pool);
    let id = draft_id(&app, "ship").await;
    let r = api_post(&app, &format!("/api/variants/{id}/approve"), json!({ "approver": "Ritwik", "traffic_share": 0.1 })).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn approve_records_an_audit_row(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    let id = draft_id(&app, "ship").await;
    api_post(&app, &format!("/api/variants/{id}/approve"), json!({ "approver": "Ritwik", "traffic_share": 0.2, "comment": "read the diff" })).await;
    let (actor, share, comment, checks): (String, f32, String, serde_json::Value) =
        sqlx::query_as("SELECT actor, traffic_share, comment, checks FROM approvals WHERE action = 'approve'").fetch_one(&pool).await.unwrap();
    assert_eq!((actor.as_str(), share, comment.as_str()), ("Ritwik", 0.2, "read the diff"));
    assert_eq!(checks.as_array().unwrap().len(), 4);
    let by: String = sqlx::query_scalar("SELECT approved_by FROM variants WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(by, "Ritwik");
}

#[sqlx::test(migrations = false)]
async fn approve_records_the_model_version(pool: PgPool) {
    setup_demo(&pool).await;
    sqlx::query("UPDATE models SET version = 41").execute(&pool).await.unwrap();
    let app = admin_app(&pool);
    let id = draft_id(&app, "ship").await;
    api_post(&app, &format!("/api/variants/{id}/approve"), json!({ "approver": "Ritwik", "traffic_share": 0.1 })).await;
    let v: Option<i64> = sqlx::query_scalar("SELECT approved_model_version FROM variants WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(v, Some(41));
}

#[sqlx::test(migrations = false)]
async fn approve_refuses_a_non_draft(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    let id = draft_id(&app, "ship").await;
    api_post(&app, &format!("/api/variants/{id}/reject"), json!({ "actor": "Ritwik", "comment": "off brand" })).await;
    let r = api_post(&app, &format!("/api/variants/{id}/approve"), json!({ "approver": "Ritwik", "traffic_share": 0.1 })).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn approve_refuses_a_list_of_ids(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    let a = draft_id(&app, "ship").await;
    let b = draft_id(&app, "ship_two").await;
    let r = api_post(&app, &format!("/api/variants/{a}/approve"), json!({ "approver": "Ritwik", "traffic_share": 0.1, "ids": [a, b] })).await;
    assert!(r.status.is_client_error(), "{} {}", r.status, r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM variants WHERE status = 'draft'").await, 2, "a bulk approve went through");
}

#[sqlx::test(migrations = false)]
async fn approve_needs_a_named_approver(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    let id = draft_id(&app, "ship").await;
    let r = api_post(&app, &format!("/api/variants/{id}/approve"), json!({ "approver": "  ", "traffic_share": 0.1 })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn reject_records_comment_and_never_serves(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    let id = draft_id(&app, "ship").await;
    let r = api_post(&app, &format!("/api/variants/{id}/reject"), json!({ "actor": "Ritwik", "comment": "off brand" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let status: String = sqlx::query_scalar("SELECT status FROM variants WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(status, "rejected");
    assert_eq!(count(&pool, "SELECT count(*) FROM approvals WHERE action = 'reject' AND comment = 'off brand'").await, 1);
    assert!(!served(&app, 60).await.contains("ship"));
}

#[sqlx::test(migrations = false)]
async fn approvals_list_shows_draft_with_champion_and_checks(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    draft_id(&app, "ship").await;
    let r = api_get(&app, "/api/sites/demo/approvals").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let card = &r.json()["drafts"][0];
    assert_eq!(card["key"], "ship");
    assert_eq!(card["champion"]["key"], "baseline");
    assert_eq!(card["checks"].as_array().unwrap().len(), 4);
}

#[sqlx::test(migrations = false)]
async fn draft_key_must_be_unique_in_the_slot(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    let r = draft(&app, "gift", "<p>again</p>").await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text);
}
