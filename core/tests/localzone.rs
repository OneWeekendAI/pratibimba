//! The visitor's local clock from a named time zone (`ctx.tz`, an IANA name such as
//! `Asia/Kolkata`). This is the field a server-side SDK sends: `@pratibimba/next` and
//! `@pratibimba/edge` have no browser clock, so they pass the zone the edge's IP geolocation
//! reports (or the first-party cookie the browser half sets). Core resolves the zone at the
//! site's own clock, so daylight saving and a sandbox's simulated time are both honoured.
//!
//! Every test pins the clock: the demo site is made a sandbox with a fixed `sim_now`, so a
//! Saturday is a Saturday whenever the suite runs.

mod common;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// The demo site, running on a fixed clock (`sim_now`), with no holdout.
async fn at_clock(pool: &PgPool, iso: &str) -> axum::Router {
    setup_learning(pool).await;
    sqlx::query("INSERT INTO sites (id, origin) VALUES ('real', 'http://localhost')").execute(pool).await.unwrap();
    sqlx::query("UPDATE sites SET sandbox_of = 'real', sim_now = $1::timestamptz WHERE id = 'demo'")
        .bind(iso)
        .execute(pool)
        .await
        .unwrap();
    app(pool)
}

async fn decide_ctx(app: &axum::Router, subject: &str, ctx: Value) -> common::Resp {
    post(app, "/decide", json!({ "site": "demo", "slot": "hero", "subject": subject, "ctx": ctx })).await
}

async fn features(app: &axum::Router, subject: &str, ctx: Value) -> Value {
    let r = decide_ctx(app, subject, ctx).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json()["explanation"]["features"].clone()
}

fn angle(hour: f64) -> f64 {
    2.0 * std::f64::consts::PI * hour / 24.0
}

// --- a named zone gives the local clock ----------------------------------------------------------

/// Wednesday 20:00 UTC is 01:30 on Thursday in Kolkata (UTC+5:30): night.
#[sqlx::test(migrations = false)]
async fn a_named_zone_puts_the_local_clock_in_the_features(pool: PgPool) {
    let app = at_clock(&pool, "2026-10-07T20:00:00Z").await;
    let r = decide_ctx(&app, "owl", json!({ "tz": "Asia/Kolkata" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let e = &r.json()["explanation"];
    let f = &e["features"];
    assert!((f["sin_hour"].as_f64().unwrap() - angle(1.0).sin()).abs() < 1e-9, "{f}");
    assert!((f["cos_hour"].as_f64().unwrap() - angle(1.0).cos()).abs() < 1e-9, "{f}");
    assert_eq!(f["is_night_local"], json!(1.0), "{f}");
    assert_eq!(e["policy"], "lints_time");
}

/// The same instant is 16:00 in New York (UTC-4 in October): not night.
#[sqlx::test(migrations = false)]
async fn a_named_day_zone_is_not_night(pool: PgPool) {
    let app = at_clock(&pool, "2026-10-07T20:00:00Z").await;
    let f = features(&app, "lunch", json!({ "tz": "America/New_York" })).await;
    assert_eq!(f["is_night_local"], json!(0.0), "{f}");
    assert!((f["sin_hour"].as_f64().unwrap() - angle(16.0).sin()).abs() < 1e-9, "{f}");
}

/// 21:30 UTC in July is 22:30 in London (BST, UTC+1): inside the 22-05 night.
#[sqlx::test(migrations = false)]
async fn a_zone_is_resolved_with_daylight_saving_in_summer(pool: PgPool) {
    let app = at_clock(&pool, "2026-07-15T21:30:00Z").await;
    let f = features(&app, "summer", json!({ "tz": "Europe/London" })).await;
    assert_eq!(f["is_night_local"], json!(1.0), "{f}");
}

/// 21:30 UTC in January is 21:30 in London (GMT): not yet night. The offset is resolved at the
/// site's clock, not at the moment the test runs.
#[sqlx::test(migrations = false)]
async fn a_zone_is_resolved_at_the_sites_clock_in_winter(pool: PgPool) {
    let app = at_clock(&pool, "2026-01-15T21:30:00Z").await;
    let f = features(&app, "winter", json!({ "tz": "Europe/London" })).await;
    assert_eq!(f["is_night_local"], json!(0.0), "{f}");
}

/// A named zone is what the edge saw; it wins over a raw offset (backend.md: "the engine will
/// prefer it").
#[sqlx::test(migrations = false)]
async fn a_named_zone_wins_over_an_offset(pool: PgPool) {
    let app = at_clock(&pool, "2026-10-07T20:00:00Z").await;
    let f = features(&app, "both", json!({ "tz": "Asia/Kolkata", "tz_offset": -240 })).await;
    assert_eq!(f["is_night_local"], json!(1.0), "{f}");
}

#[sqlx::test(migrations = false)]
async fn an_unknown_zone_is_refused(pool: PgPool) {
    let app = at_clock(&pool, "2026-10-07T20:00:00Z").await;
    let r = decide_ctx(&app, "mars", json!({ "tz": "Mars/Olympus_Mons" })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
}

/// A time audience opens for a visitor whose clock came from a named zone alone.
#[sqlx::test(migrations = false)]
async fn a_named_zone_counts_as_a_known_clock_for_an_audience(pool: PgPool) {
    let app = at_clock(&pool, "2026-10-07T20:00:00Z").await;
    let admin = admin_app(&pool);
    let r = api_post(&admin, "/api/sites/demo/segments", json!({ "predicate": { "hour_local": [0, 23] }, "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    let aud = r.json()["id"].as_i64().unwrap();
    api_post(&admin, &format!("/api/segments/{aud}/promote"), json!({ "actor": "owner" })).await;
    api_post(&admin, &format!("/api/variants/{}/audience", variant_id(&pool, "gift").await), json!({ "audience": aud, "actor": "owner" })).await;
    let r = decide_ctx(&app, "zoned", json!({ "tz": "Asia/Kolkata" })).await.json();
    assert_eq!(r["explanation"]["audiences"][0]["matched"], json!(true), "{r}");
}

// --- weekend and business hours, as bandit inputs ------------------------------------------------

/// Friday 20:00 UTC is Saturday 01:30 in Kolkata: the visitor's weekday, not UTC's.
#[sqlx::test(migrations = false)]
async fn a_local_saturday_is_weekend_even_when_utc_is_friday(pool: PgPool) {
    let app = at_clock(&pool, "2026-10-09T20:00:00Z").await;
    let f = features(&app, "sat", json!({ "tz": "Asia/Kolkata" })).await;
    assert_eq!(f["is_weekend_local"], json!(1.0), "{f}");
}

/// Wednesday 08:00 UTC is 13:30 in Kolkata: a weekday, inside 09-17.
#[sqlx::test(migrations = false)]
async fn a_weekday_afternoon_is_business_hours(pool: PgPool) {
    let app = at_clock(&pool, "2026-10-07T08:00:00Z").await;
    let f = features(&app, "desk", json!({ "tz": "Asia/Kolkata" })).await;
    assert_eq!(f["is_business_hours_local"], json!(1.0), "{f}");
    assert_eq!(f["is_weekend_local"], json!(0.0), "{f}");
}

/// Wednesday 13:00 UTC is 18:30 in Kolkata: after hours.
#[sqlx::test(migrations = false)]
async fn a_weekday_evening_is_not_business_hours(pool: PgPool) {
    let app = at_clock(&pool, "2026-10-07T13:00:00Z").await;
    let f = features(&app, "home", json!({ "tz": "Asia/Kolkata" })).await;
    assert_eq!(f["is_business_hours_local"], json!(0.0), "{f}");
}

/// The browser's raw offset (the client script's path) carries the same two features.
#[sqlx::test(migrations = false)]
async fn an_offset_carries_weekend_and_business_hours_too(pool: PgPool) {
    let app = at_clock(&pool, "2026-10-10T10:00:00Z").await;
    let f = features(&app, "sat-utc", json!({ "tz_offset": 0 })).await;
    assert_eq!(f["is_weekend_local"], json!(1.0), "{f}");
    assert_eq!(f["is_business_hours_local"], json!(0.0), "{f}");
}

/// No clock, no time features at all: none of the five names appears.
#[sqlx::test(migrations = false)]
async fn no_clock_means_no_weekend_or_business_features(pool: PgPool) {
    let app = at_clock(&pool, "2026-10-10T10:00:00Z").await;
    let f = features(&app, "nobody", json!({})).await;
    let f = f.as_object().unwrap();
    assert!(!f.contains_key("is_weekend_local") && !f.contains_key("is_business_hours_local"), "{f:?}");
}
