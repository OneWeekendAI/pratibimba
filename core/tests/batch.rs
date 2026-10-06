//! `POST /decide/batch`: the call `@pratibimba/next` makes once per page render.

mod common;
use common::*;
use serde_json::json;
use sqlx::PgPool;

/// The demo site plus a second slot, `promo`.
async fn two_slots(pool: &PgPool) {
    pratibimba::db::migrate(pool).await.unwrap();
    let f = file_from(json!({
        "site": "demo", "origin": "http://localhost:5173",
        "slots": [
            { "key": "hero", "selector": "#hero", "variants": [{ "key": "baseline", "baseline": true }, { "key": "gift", "html": "<h1>g</h1>" }] },
            { "key": "promo", "selector": "#promo", "variants": [{ "key": "plain", "baseline": true }, { "key": "ten_off", "html": "<p>10%</p>" }] }
        ]
    }));
    pratibimba::site_file::apply(pool, &f).await.unwrap();
}

async fn batch(app: &axum::Router, subject: &str, slots: &[&str]) -> Resp {
    post(app, "/decide/batch", json!({ "site": "demo", "subject": subject, "slots": slots, "ctx": { "mobile": true } })).await
}

#[sqlx::test(migrations = false)]
async fn batch_decides_each_slot(pool: PgPool) {
    two_slots(&pool).await;
    let r = batch(&app(&pool), "v", &["hero", "promo"]).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let ds = r.json()["decisions"].as_array().unwrap().clone();
    assert_eq!(ds.len(), 2);
    assert_eq!(ds[0]["slot"], "hero");
    assert_eq!(ds[1]["slot"], "promo");
    assert!(ds.iter().all(|d| d["decision_id"].is_string()), "{ds:?}");
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions WHERE (features->>'mobile')::float = 1").await, 2);
}

#[sqlx::test(migrations = false)]
async fn batch_shares_stickiness_with_single_decide(pool: PgPool) {
    two_slots(&pool).await;
    let app = app(&pool);
    let single = decide(&app, "v", false).await.json();
    let b = batch(&app, "v", &["hero"]).await.json();
    assert_eq!(b["decisions"][0]["decision_id"], single["decision_id"]);
}

#[sqlx::test(migrations = false)]
async fn batch_reports_unknown_slot_without_failing_others(pool: PgPool) {
    two_slots(&pool).await;
    let r = batch(&app(&pool), "v", &["nope", "hero"]).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let ds = r.json()["decisions"].clone();
    assert!(ds[0]["error"].as_str().unwrap().contains("unknown slot"), "{ds}");
    assert!(ds[1]["decision_id"].is_string(), "{ds}");
}

#[sqlx::test(migrations = false)]
async fn batch_refuses_too_many_slots(pool: PgPool) {
    two_slots(&pool).await;
    let many: Vec<&str> = std::iter::repeat_n("hero", 21).collect();
    let r = batch(&app(&pool), "v", &many).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions").await, 0);
}

#[sqlx::test(migrations = false)]
async fn batch_refuses_demographic_context(pool: PgPool) {
    two_slots(&pool).await;
    let r = post(&app(&pool), "/decide/batch", json!({ "site": "demo", "subject": "v", "slots": ["hero"], "ctx": { "age": 40 } })).await;
    assert!(r.status.is_client_error(), "{} {}", r.status, r.text);
}
