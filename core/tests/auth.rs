//! The admin surface is behind PRB_ADMIN_TOKEN; the public surface is not.

mod common;
use axum::{body::Body, http::Request};
use common::*;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
async fn api_refuses_without_token(pool: PgPool) {
    setup_demo(&pool).await;
    let r = get(&admin_app(&pool), "/api/sites").await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn api_refuses_a_wrong_token(pool: PgPool) {
    setup_demo(&pool).await;
    let r = send(&admin_app(&pool), Request::get("/api/sites").header("authorization", "Bearer test-admin-tokeX").body(Body::empty()).unwrap()).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn api_accepts_bearer(pool: PgPool) {
    setup_demo(&pool).await;
    let r = api_get(&admin_app(&pool), "/api/sites").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.json()[0]["id"], "demo");
}

#[sqlx::test(migrations = false)]
async fn api_accepts_session_cookie(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    let login = post(&app, "/api/session", json!({ "token": TOKEN })).await;
    assert_eq!(login.status, StatusCode::NO_CONTENT, "{}", login.text);
    let cookie = login.headers.get("set-cookie").unwrap().to_str().unwrap().to_string();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"), "{cookie}");
    let pair = cookie.split(';').next().unwrap().to_string();
    let r = send(&app, Request::get("/api/sites").header("cookie", pair).body(Body::empty()).unwrap()).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn session_refuses_a_wrong_token(pool: PgPool) {
    setup_demo(&pool).await;
    let r = post(&admin_app(&pool), "/api/session", json!({ "token": "nope" })).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(r.headers.get("set-cookie").is_none());
}

#[sqlx::test(migrations = false)]
async fn api_is_off_without_configured_token(pool: PgPool) {
    setup_demo(&pool).await;
    // `app` has no admin token configured: even the right-looking bearer is refused.
    let r = api_get(&app(&pool), "/api/sites").await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE, "{}", r.text);
    assert!(r.text.contains("PRB_ADMIN_TOKEN"), "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn public_surface_needs_no_token(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    assert_eq!(get(&app, "/manifest/demo").await.status, StatusCode::OK);
    assert_eq!(decide(&app, "v", false).await.status, StatusCode::OK);
}

#[sqlx::test(migrations = false)]
async fn dashboard_is_served_under_app(pool: PgPool) {
    setup_demo(&pool).await;
    let dir = std::env::temp_dir().join(format!("prb-dash-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("slot")).unwrap();
    std::fs::write(dir.join("index.html"), "<p>home</p>").unwrap();
    std::fs::write(dir.join("slot/index.html"), "<p>slot page</p>").unwrap();
    std::fs::write(dir.join("404.html"), "<p>not here</p>").unwrap();
    let app = pratibimba::http::router(pratibimba::http::AppState::configured(pool.clone(), Some(1), Some(TOKEN.into()), Some(dir.clone())), &[]);
    assert_eq!(get(&app, "/app/").await.text, "<p>home</p>");
    assert_eq!(get(&app, "/app/slot/").await.text, "<p>slot page</p>");
    let missing = get(&app, "/app/nope/").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(get(&app, "/app").await.status, StatusCode::PERMANENT_REDIRECT);
    std::fs::remove_dir_all(dir).unwrap();
}
