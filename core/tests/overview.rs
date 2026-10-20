//! The site home's numbers, and the SSE stream that carries them.

mod common;
use axum::{body::Body, http::Request};
use common::*;
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;

#[sqlx::test(migrations = false)]
async fn overview_counts_last_minute_decisions(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    for i in 0..3 {
        decide(&app, &format!("o{i}"), false).await;
    }
    decide(&app, "old", false).await;
    sqlx::query("UPDATE decisions SET created_at = now() - interval '5 minutes' WHERE subject = 'old'").execute(&pool).await.unwrap();
    let o = api_get(&app, "/api/sites/demo/overview").await.json();
    assert_eq!(o["decisions_last_minute"], 3, "{o}");
    assert_eq!(o["sparkline"].as_array().unwrap().len(), 30);
}

#[sqlx::test(migrations = false)]
async fn overview_counts_last_hour_conversions(pool: PgPool) {
    setup_demo(&pool).await;
    seed_decisions(&pool, "gift", false, 10, 4, 10).await;
    sqlx::query("UPDATE decisions SET outcome_at = now() - interval '2 hours' WHERE subject IN ('seed-gift-false-1')").execute(&pool).await.unwrap();
    let o = api_get(&admin_app(&pool), "/api/sites/demo/overview").await.json();
    assert_eq!(o["conversions_last_hour"], 3, "{o}");
}

#[sqlx::test(migrations = false)]
async fn overview_flags_a_silent_site(pool: PgPool) {
    setup_demo(&pool).await;
    seed_decisions(&pool, "gift", false, 50, 0, 20).await;
    let o = api_get(&admin_app(&pool), "/api/sites/demo/overview").await.json();
    let stale = o["stale"].as_str().unwrap_or_else(|| panic!("a silent site was not flagged: {o}"));
    assert!(stale.contains("20 min"), "{stale}");
}

#[sqlx::test(migrations = false)]
async fn overview_does_not_flag_a_busy_site(pool: PgPool) {
    setup_demo(&pool).await;
    seed_decisions(&pool, "gift", false, 50, 0, 20).await;
    seed_decisions(&pool, "baseline", false, 5, 0, 1).await;
    let o = api_get(&admin_app(&pool), "/api/sites/demo/overview").await.json();
    assert!(o["stale"].is_null(), "{o}");
}

#[sqlx::test(migrations = false)]
async fn overview_lists_slots_with_champion(pool: PgPool) {
    setup_demo(&pool).await;
    seed_decisions(&pool, "gift", false, 30, 3, 60).await;
    seed_decisions(&pool, "baseline", false, 10, 1, 60).await;
    let o = api_get(&admin_app(&pool), "/api/sites/demo/overview").await.json();
    assert_eq!(o["slots"][0]["key"], "hero");
    assert_eq!(o["slots"][0]["champion"], "gift", "{o}");
}

#[sqlx::test(migrations = false)]
async fn stream_emits_overview(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    let res = app
        .oneshot(Request::get("/api/sites/demo/stream").header("authorization", format!("Bearer {TOKEN}")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers()["content-type"].to_str().unwrap().starts_with("text/event-stream"));
    let mut body = res.into_body();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), body.frame()).await.expect("no event within 5 s").unwrap().unwrap();
    let text = String::from_utf8_lossy(frame.data_ref().unwrap()).to_string();
    assert!(text.starts_with("event: overview"), "{text}");
    assert!(text.contains("\"decisions_last_minute\""), "{text}");
}

#[sqlx::test(migrations = false)]
async fn slot_detail_has_variants_curve_and_lift(pool: PgPool) {
    setup_demo(&pool).await;
    seed_decisions(&pool, "gift", false, 100, 10, 60).await;
    seed_decisions(&pool, "baseline", true, 100, 5, 60).await;
    let r = api_get(&admin_app(&pool), "/api/sites/demo/slots/hero").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let s = r.json();
    assert_eq!(s["variants"].as_array().unwrap().len(), 2);
    assert!(!s["curve"].as_array().unwrap().is_empty());
    assert!((s["lift"]["estimate"].as_f64().unwrap() - 0.05).abs() < 1e-9, "{s}");
}

#[sqlx::test(migrations = false)]
async fn results_endpoint_has_learnings(pool: PgPool) {
    setup_demo(&pool).await;
    sqlx::query("INSERT INTO learnings (site_id, kind, sentence) VALUES ('demo', 'winning', 'gift wins')").execute(&pool).await.unwrap();
    let r = api_get(&admin_app(&pool), "/api/sites/demo/results").await.json();
    assert_eq!(r["learnings"][0]["sentence"], "gift wins");
    assert!(r["curves"]["hero"].is_array());
}

/// The slot page and the results page both carry the decisions still in their window.
#[sqlx::test(migrations = false)]
async fn slot_detail_and_results_carry_pending(pool: PgPool) {
    setup_demo(&pool).await;
    seed_decisions(&pool, "gift", false, 12, 3, 5).await;
    let app = admin_app(&pool);
    let s = api_get(&app, "/api/sites/demo/slots/hero").await.json();
    assert_eq!(s["treated"]["n"], 0, "{s}");
    assert_eq!(s["pending"]["decisions"], 12, "{s}");
    assert_eq!(s["pending"]["conversions"], 3, "{s}");
    let gift = s["variants"].as_array().unwrap().iter().find(|v| v["key"] == "gift").unwrap().clone();
    assert_eq!(gift["pending"]["exposures"], 12, "{gift}");
    let r = api_get(&app, "/api/sites/demo/results").await.json();
    let hero = r["results"]["slots"].as_array().unwrap().iter().find(|s| s["key"] == "hero").unwrap().clone();
    assert_eq!(hero["pending"]["decisions"], 12, "{hero}");
}
