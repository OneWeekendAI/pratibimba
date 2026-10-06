//! Shared test helpers. Tests need `DATABASE_URL` pointing at a Postgres server where the
//! user may create databases; `#[sqlx::test]` makes one per test, and `FreshDb` does the same
//! for tests that need a multi-thread runtime (sqlx::test cannot select a flavour).
#![allow(dead_code)]

use sqlx::{Connection, PgConnection, PgPool, postgres::{PgConnectOptions, PgPoolOptions}};
use std::str::FromStr;

pub struct FreshDb {
    pub pool: PgPool,
    name: String,
}

impl FreshDb {
    pub async fn new() -> Self {
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for tests");
        let name = format!("prb_t_{}", uuid::Uuid::new_v4().simple());
        let mut admin = PgConnection::connect(&url).await.unwrap();
        sqlx::query(&format!("CREATE DATABASE {name}")).execute(&mut admin).await.unwrap();
        let opts = PgConnectOptions::from_str(&url).unwrap().database(&name);
        let pool = PgPoolOptions::new().max_connections(20).connect_with(opts).await.unwrap();
        FreshDb { pool, name }
    }

    pub async fn drop(self) {
        self.pool.close().await;
        let url = std::env::var("DATABASE_URL").unwrap();
        let mut admin = PgConnection::connect(&url).await.unwrap();
        sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", self.name)).execute(&mut admin).await.unwrap();
    }
}

pub async fn table_exists(pool: &PgPool, name: &str) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema='public' AND table_name=$1)",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The standard fixture: site `demo`, slot `hero` (#hero, metric click) with an approved
/// baseline and an approved `gift` variant.
pub fn demo_file() -> pratibimba::site_file::SiteFile {
    serde_json::from_value(serde_json::json!({
        "site": "demo", "origin": "http://localhost:5173",
        "slots": [{ "key": "hero", "selector": "#hero", "metric": "click",
                    "variants": [ { "key": "baseline", "baseline": true },
                                  { "key": "gift", "html": "<h1>Gift sets</h1>" } ] }]
    }))
    .unwrap()
}

pub fn file_from(v: serde_json::Value) -> pratibimba::site_file::SiteFile {
    serde_json::from_value(v).unwrap()
}

/// A migrated pool with the demo site applied.
pub async fn setup_demo(pool: &PgPool) {
    pratibimba::db::migrate(pool).await.unwrap();
    pratibimba::site_file::apply(pool, &demo_file()).await.unwrap();
}

/// Learning tests need every decision to teach the model, so they switch the holdout off.
pub async fn set_holdout(pool: &PgPool, fraction: f32) {
    sqlx::query("UPDATE sites SET holdout_fraction = $1").bind(fraction).execute(pool).await.unwrap();
}

/// The demo site with no holdout: every exposed decision teaches the model.
pub async fn setup_learning(pool: &PgPool) {
    setup_demo(pool).await;
    set_holdout(pool, 0.0).await;
}

pub async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

pub use axum::http::StatusCode;
use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use tower::ServiceExt;

pub fn app(pool: &PgPool) -> Router {
    pratibimba::http::router(pratibimba::http::AppState::with_seed(pool.clone(), 7), &[])
}

pub struct Resp {
    pub status: StatusCode,
    pub headers: axum::http::HeaderMap,
    pub text: String,
}

impl Resp {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.text).unwrap_or_else(|_| panic!("not JSON: {}", self.text))
    }
}

pub async fn send(app: &Router, req: Request<Body>) -> Resp {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    Resp { status, headers, text: String::from_utf8_lossy(&bytes).into_owned() }
}

pub async fn get(app: &Router, path: &str) -> Resp {
    send(app, Request::get(path).body(Body::empty()).unwrap()).await
}

pub async fn post(app: &Router, path: &str, body: serde_json::Value) -> Resp {
    send(app, Request::post(path).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap()).await
}

/// One decide call for the demo hero slot.
pub async fn decide(app: &Router, subject: &str, returning: bool) -> Resp {
    post(app, "/decide", serde_json::json!({ "site": "demo", "slot": "hero", "subject": subject, "ctx": { "returning": returning } })).await
}

pub async fn event(app: &Router, subject: &str, name: &str, decision_id: Option<&str>) -> Resp {
    post(app, "/e", serde_json::json!({ "site": "demo", "subject": subject,
        "events": [{ "id": uuid::Uuid::new_v4(), "name": name, "slot": "hero", "decision_id": decision_id }] })).await
}

pub async fn insert_variant(pool: &PgPool, key: &str, status: &str) {
    sqlx::query("INSERT INTO variants (slot_id, key, payload, status, approved_by) SELECT id, $1, '<p>x</p>', $2, CASE WHEN $2='approved' THEN 'test' END FROM slots WHERE key='hero'")
        .bind(key)
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
}
