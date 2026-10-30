//! The approval gate: drafts are never served; one variant, one named approver, at most 50%.

mod common;
use common::*;
use serde_json::{Value, json};
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

// ---- P1 templates through the dashboard (B3) ----

async fn copy_site(pool: &PgPool, max: u32) {
    let f = json!({ "site": "demo", "origin": "http://localhost:5173", "slots": [{
        "key": "hero", "selector": "#hero", "kind": "copy",
        "copy": { "fields": [ { "name": "title", "max": max, "required": true } ] },
        "variants": [ { "key": "own", "baseline": true } ] }] });
    pratibimba::site_file::apply(pool, &file_from(f)).await.unwrap();
}

// Mutation: drop `v.requires` from the approvals card -> red.
#[sqlx::test(migrations = false)]
async fn draft_route_accepts_requires(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    copy_site(&pool, 40).await;
    let app = admin_app(&pool);
    let body = json!({ "key": "usual", "html": r#"{"title":"Your usual {visitor.top_item.name | truncate: 20}"}"#,
                       "requires": ["visitor.top_item"] });
    let r = api_post(&app, "/api/sites/demo/slots/hero/variants", body).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    let cards = api_get(&app, "/api/sites/demo/approvals").await.json();
    let card = cards["drafts"].as_array().unwrap().iter().find(|c| c["key"] == "usual").cloned().expect("the draft is queued");
    assert_eq!(card["requires"], json!(["visitor.top_item"]), "{card}");
}

// Mutation: drop the copy::check_covered_stored call in create_draft -> red.
#[sqlx::test(migrations = false)]
async fn draft_route_refuses_uncovered_required_placeholder(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    copy_site(&pool, 40).await;
    let app = admin_app(&pool);
    let body = json!({ "key": "usual", "html": r#"{"title":"{visitor.top_item.name | truncate: 20}"}"# });
    let r = api_post(&app, "/api/sites/demo/slots/hero/variants", body).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    assert!(r.text.contains("is required") && r.text.contains("requires") && r.text.contains("fallback"), "{}", r.text);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM variants WHERE key = 'usual'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0);
}

// Mutation: skip copy::check_stored in approve -> red.
#[sqlx::test(migrations = false)]
async fn approve_rechecks_template_length(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    copy_site(&pool, 40).await;
    set_holdout(&pool, 0.0).await;
    let app = admin_app(&pool);
    let body = json!({ "key": "usual", "html": r#"{"title":"Your usual {visitor.top_item.name | truncate: 20}"}"#,
                       "requires": ["visitor.top_item"] });
    let r = api_post(&app, "/api/sites/demo/slots/hero/variants", body).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    let id = r.json()["id"].as_i64().unwrap();
    // The site file lowers the title's max to 25 after the draft; "Your usual " + 20 is 31.
    copy_site(&pool, 25).await;
    let r = api_post(&app, &format!("/api/variants/{id}/approve"), json!({ "approver": "Ritwik", "traffic_share": 0.2 })).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text);
    assert!(r.text.contains("no longer fits") && r.text.contains("visitor.top_item.name"), "{}", r.text);
    let st: String = sqlx::query_scalar("SELECT status FROM variants WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(st, "draft");
}

// Mutation: build renders from no persona facts (every render None) -> red.
#[sqlx::test(migrations = false)]
async fn approvals_carries_one_render_per_persona(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    copy_site(&pool, 40).await;
    let app = admin_app(&pool);
    let body = json!({ "key": "usual", "html": r#"{"title":"Your usual {visitor.top_item.name | truncate: 20}"}"#,
                       "requires": ["visitor.top_item"] });
    assert_eq!(api_post(&app, "/api/sites/demo/slots/hero/variants", body).await.status, StatusCode::CREATED);
    let plain = json!({ "key": "plain", "html": r#"{"title":"Fresh today"}"# });
    assert_eq!(api_post(&app, "/api/sites/demo/slots/hero/variants", plain).await.status, StatusCode::CREATED);
    let cards = api_get(&app, "/api/sites/demo/approvals").await.json();
    let card = |k: &str| cards["drafts"].as_array().unwrap().iter().find(|c| c["key"] == k).cloned().unwrap();
    let renders = card("usual")["renders"].clone();
    assert_eq!(renders.as_array().map(Vec::len), Some(4), "{renders}");
    assert_eq!(renders[0], json!({ "persona": "first_visit", "label": "First visit", "fields": null, "falls_to": "own" }));
    assert_eq!(renders[2]["persona"], "usual_rajma");
    assert_eq!(renders[2]["fields"], json!({ "title": "Your usual rajma-chawal" }), "{renders}");
    assert!(renders[2]["falls_to"].is_null());
    assert!(card("plain").get("renders").is_none_or(Value::is_null), "a plain draft carries no renders");
}

// ---- B8: approving or rejecting an advisor card ----

/// The demo hero slot with a third approved arm and a draft, and a pending pause card for it.
async fn pause_card(pool: &PgPool) -> i64 {
    setup_demo(pool).await;
    insert_variant(pool, "ship", "approved").await;
    insert_variant(pool, "later", "draft").await;
    sqlx::query_scalar(
        "INSERT INTO advisor_cards (site_id, slot_id, kind, state, evidence)
         SELECT 'demo', id, 'pause_slot', 'confident', '{\"claim\": \"hero: all 2 versions are within ±3 conversions per 100 of the page''s own over 8,000 visits. It makes no difference.\"}'
           FROM slots WHERE key = 'hero' RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn statuses(pool: &PgPool) -> Vec<(String, String)> {
    sqlx::query_as("SELECT key, status FROM variants ORDER BY key").fetch_all(pool).await.unwrap()
}

// Mutation: pause the baseline too (drop `NOT is_baseline`) -> red.
#[sqlx::test(migrations = false)]
async fn approve_card_pauses_non_baseline_only(pool: PgPool) {
    let id = pause_card(&pool).await;
    let app = admin_app(&pool);
    let r = api_post(&app, &format!("/api/advisor-cards/{id}/approve"), json!({ "actor": "Asha" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let s = |k: &str, v: &str| (k.to_string(), v.to_string());
    assert_eq!(statuses(&pool).await, vec![s("baseline", "approved"), s("gift", "paused"), s("later", "draft"), s("ship", "paused")]);
    let (status, by): (String, Option<String>) = sqlx::query_as("SELECT status, decided_by FROM advisor_cards WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!((status.as_str(), by.as_deref()), ("approved", Some("Asha")));
    assert_eq!(count(&pool, &format!("SELECT count(*) FROM approvals WHERE action = 'pause' AND actor = 'Asha' AND advisor_card_id = {id}")).await, 2);
    assert_eq!(count(&pool, "SELECT count(*) FROM variants WHERE status = 'paused' AND paused_at IS NOT NULL AND pause_reason->>'advisor_card' IS NOT NULL").await, 2);
}

// Mutation: skip the `actor` check on card approve -> red.
#[sqlx::test(migrations = false)]
async fn approve_card_needs_named_approver(pool: PgPool) {
    let id = pause_card(&pool).await;
    let before = statuses(&pool).await;
    let r = api_post(&admin_app(&pool), &format!("/api/advisor-cards/{id}/approve"), json!({ "actor": "  " })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    assert_eq!(statuses(&pool).await, before);
    assert_eq!(count(&pool, "SELECT count(*) FROM advisor_cards WHERE status = 'pending'").await, 1);
}

// Mutation: reject runs the approve path's variant UPDATE -> red.
#[sqlx::test(migrations = false)]
async fn reject_card_changes_no_variant(pool: PgPool) {
    let id = pause_card(&pool).await;
    let before = statuses(&pool).await;
    let r = api_post(&admin_app(&pool), &format!("/api/advisor-cards/{id}/reject"), json!({ "actor": "Asha", "comment": "keep testing" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(statuses(&pool).await, before);
    let (status, at): (String, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as("SELECT status, decided_at FROM advisor_cards WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(status, "rejected");
    assert!(at.is_some(), "the quiet period counts from decided_at");
}

// Mutation: drop the `status = 'pending'` check on the card -> red.
#[sqlx::test(migrations = false)]
async fn approve_twice_conflicts(pool: PgPool) {
    let id = pause_card(&pool).await;
    let app = admin_app(&pool);
    assert_eq!(api_post(&app, &format!("/api/advisor-cards/{id}/approve"), json!({ "actor": "Asha" })).await.status, StatusCode::OK);
    let r = api_post(&app, &format!("/api/advisor-cards/{id}/approve"), json!({ "actor": "Ravi" })).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text);
    let r = api_post(&app, &format!("/api/advisor-cards/{id}/reject"), json!({ "actor": "Ravi" })).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM approvals WHERE action = 'pause'").await, 2, "the second approve paused nothing more");
}

// Mutation: approvals JSON leaves out `cards` (or lists decided cards) -> red.
#[sqlx::test(migrations = false)]
async fn approvals_lists_open_advisor_cards(pool: PgPool) {
    let id = pause_card(&pool).await;
    let app = admin_app(&pool);
    let j = api_get(&app, "/api/sites/demo/approvals").await.json();
    let cards = j["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 1, "{j}");
    assert_eq!((&cards[0]["id"], &cards[0]["slot"], &cards[0]["state"]), (&json!(id), &json!("hero"), &json!("confident")));
    assert_eq!(cards[0]["pauses"], json!(["gift", "ship"]), "the versions approving would pause");
    assert!(cards[0]["evidence"]["claim"].as_str().unwrap().starts_with("hero:"));
    api_post(&app, &format!("/api/advisor-cards/{id}/reject"), json!({ "actor": "Asha" })).await;
    assert_eq!(api_get(&app, "/api/sites/demo/approvals").await.json()["cards"], json!([]));
}
