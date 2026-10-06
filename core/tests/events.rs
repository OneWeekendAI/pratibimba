mod common;
use common::*;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
async fn events_write_event_and_outbox_together(pool: PgPool) {
    setup_demo(&pool).await;
    let r = event(&app(&pool), "v", "click", None).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE slot_id IS NOT NULL").await, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM pending_events").await, 1);
}

#[sqlx::test(migrations = false)]
async fn duplicate_event_id_is_ignored(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app(&pool);
    let body = json!({ "site": "demo", "subject": "v", "events": [{ "id": uuid::Uuid::new_v4(), "name": "click" }] });
    assert_eq!(post(&app, "/e", body.clone()).await.status, StatusCode::ACCEPTED);
    let again = post(&app, "/e", body).await;
    assert_eq!(again.status, StatusCode::ACCEPTED, "{}", again.text);
    assert_eq!(again.json()["accepted"], 0);
    assert_eq!(count(&pool, "SELECT count(*) FROM events").await, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM pending_events").await, 1);
}

#[sqlx::test(migrations = false)]
async fn events_reject_unknown_fields(pool: PgPool) {
    setup_demo(&pool).await;
    let r = post(&app(&pool), "/e", json!({ "site": "demo", "subject": "v",
        "events": [{ "id": uuid::Uuid::new_v4(), "name": "click", "email": "a@b.c" }] })).await;
    assert!(r.status.is_client_error(), "{} {}", r.status, r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM events").await, 0);
}

#[sqlx::test(migrations = false)]
async fn events_reject_bad_name(pool: PgPool) {
    setup_demo(&pool).await;
    let r = event(&app(&pool), "v", "Click!", None).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = false)]
async fn events_reject_oversized_batch(pool: PgPool) {
    setup_demo(&pool).await;
    let events: Vec<_> = (0..101).map(|_| json!({ "id": uuid::Uuid::new_v4(), "name": "click" })).collect();
    let r = post(&app(&pool), "/e", json!({ "site": "demo", "subject": "v", "events": events })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = false)]
async fn events_unknown_site_is_404(pool: PgPool) {
    setup_demo(&pool).await;
    let r = post(&app(&pool), "/e", json!({ "site": "nope", "subject": "v", "events": [{ "id": uuid::Uuid::new_v4(), "name": "click" }] })).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

/// Each event and its outbox row are one CTE statement, which Postgres already makes atomic.
/// What the transaction adds is the batch: if the second event's outbox row fails, the
/// first event must not survive either.
#[sqlx::test(migrations = false)]
async fn events_batch_is_all_or_nothing(pool: PgPool) {
    setup_demo(&pool).await;
    sqlx::query("ALTER TABLE pending_events ADD CONSTRAINT only_first CHECK (id < 2) NOT VALID").execute(&pool).await.unwrap();
    let r = post(&app(&pool), "/e", json!({ "site": "demo", "subject": "v", "events": [
        { "id": uuid::Uuid::new_v4(), "name": "click" }, { "id": uuid::Uuid::new_v4(), "name": "click" } ] })).await;
    assert_eq!(r.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(count(&pool, "SELECT count(*) FROM events").await, 0, "half a batch was committed");
}

#[sqlx::test(migrations = false)]
async fn sdk_js_is_served_with_js_type(pool: PgPool) {
    setup_demo(&pool).await;
    let r = get(&app(&pool), "/sdk.js").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.headers["content-type"].to_str().unwrap().starts_with("application/javascript"));
}

#[sqlx::test(migrations = false)]
async fn health_reports_ok(pool: PgPool) {
    setup_demo(&pool).await;
    assert_eq!(get(&app(&pool), "/health").await.status, StatusCode::OK);
}

#[sqlx::test(migrations = false)]
async fn cors_allows_configured_origin(pool: PgPool) {
    setup_demo(&pool).await;
    let app = pratibimba::http::router(pratibimba::http::AppState::with_seed(pool.clone(), 1), &["http://shop.test".into()]);
    let req = axum::http::Request::get("/manifest/demo").header("origin", "http://shop.test").body(axum::body::Body::empty()).unwrap();
    let r = send(&app, req).await;
    assert_eq!(r.headers.get("access-control-allow-origin").map(|v| v.to_str().unwrap()), Some("http://shop.test"));
}

#[sqlx::test(migrations = false)]
async fn cors_refuses_other_origin(pool: PgPool) {
    setup_demo(&pool).await;
    let app = pratibimba::http::router(pratibimba::http::AppState::with_seed(pool.clone(), 1), &["http://shop.test".into()]);
    let req = axum::http::Request::get("/manifest/demo").header("origin", "http://evil.test").body(axum::body::Body::empty()).unwrap();
    let r = send(&app, req).await;
    assert!(r.headers.get("access-control-allow-origin").is_none());
}
