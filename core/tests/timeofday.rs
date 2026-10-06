//! Time-of-day features in the bandit's input: `sin_hour`, `cos_hour` and `is_night_local`,
//! from the visitor's reported `tz_offset` and the site's night window. And the cohort: with a
//! planted "night visitors prefer gift" truth, the policy learns it; the same cohort with no
//! offset reported cannot.

mod common;
use chrono::{Duration, Timelike, Utc};
use common::*;
use pratibimba::worker;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde_json::{Value, json};
use sqlx::PgPool;

/// The offset (minutes east of UTC) that puts a visitor at `local_hour` right now.
fn offset_for(local_hour: i64) -> i64 {
    let utc = Utc::now().hour() as i64;
    let mut off = (local_hour - utc) * 60;
    while off < -720 {
        off += 1440;
    }
    while off > 840 {
        off -= 1440;
    }
    off
}

async fn decide_at(app: &axum::Router, subject: &str, tz: Option<i64>) -> Resp {
    let mut ctx = json!({});
    if let Some(t) = tz {
        ctx["tz_offset"] = json!(t);
    }
    post(app, "/decide", json!({ "site": "demo", "slot": "hero", "subject": subject, "ctx": ctx })).await
}

#[sqlx::test(migrations = false)]
async fn a_reported_offset_puts_the_local_clock_in_the_features(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app(&pool);
    let r = decide_at(&app, "night-owl", Some(offset_for(23))).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let e = &r.json()["explanation"];
    let f = &e["features"];
    let angle = 2.0 * std::f64::consts::PI * 23.0 / 24.0;
    assert!((f["sin_hour"].as_f64().unwrap() - angle.sin()).abs() < 1e-9, "{f}");
    assert!((f["cos_hour"].as_f64().unwrap() - angle.cos()).abs() < 1e-9, "{f}");
    assert_eq!(f["is_night_local"], json!(1.0));
    assert_eq!(e["policy"], "lints_time");

    let r = decide_at(&app, "lunch", Some(offset_for(13))).await.json();
    assert_eq!(r["explanation"]["features"]["is_night_local"], json!(0.0));
}

#[sqlx::test(migrations = false)]
async fn the_night_window_is_the_sites(pool: PgPool) {
    setup_demo(&pool).await;
    // 13:00 is "night" on a site whose night is 12:00-15:00.
    sqlx::query("UPDATE sites SET night_start_hour = 12, night_end_hour = 15").execute(&pool).await.unwrap();
    let r = decide_at(&app(&pool), "siesta", Some(offset_for(13))).await.json();
    assert_eq!(r["explanation"]["features"]["is_night_local"], json!(1.0));
}

#[sqlx::test(migrations = false)]
async fn no_offset_means_no_time_features(pool: PgPool) {
    setup_demo(&pool).await;
    let r = decide_at(&app(&pool), "unknown", None).await.json();
    let f = r["explanation"]["features"].as_object().unwrap();
    assert!(!f.contains_key("sin_hour") && !f.contains_key("is_night_local"), "{f:?}");
    assert_eq!(r["explanation"]["policy"], "lints");
}

#[sqlx::test(migrations = false)]
async fn an_impossible_offset_is_refused(pool: PgPool) {
    setup_demo(&pool).await;
    assert_eq!(decide_at(&app(&pool), "x", Some(900)).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(decide_at(&app(&pool), "x", Some(-721)).await.status, StatusCode::BAD_REQUEST);
}

/// A time decision settles into the model's extension and replays exactly.
#[sqlx::test(migrations = false)]
async fn a_time_decision_learns_and_replays(pool: PgPool) {
    setup_demo(&pool).await;
    set_holdout(&pool, 0.0).await;
    let app = app(&pool);
    let d = decide_at(&app, "owl", Some(offset_for(2))).await.json();
    let id = d["decision_id"].as_str().unwrap();
    post(&app, "/e", json!({ "site": "demo", "subject": "owl", "events": [
        { "id": uuid::Uuid::new_v4(), "name": "exposure", "slot": "hero", "decision_id": id },
        { "id": uuid::Uuid::new_v4(), "name": "click", "slot": "hero", "decision_id": id } ] }))
    .await;
    worker::run_once(&pool, Utc::now() + Duration::hours(2)).await.unwrap();
    let state: Value = sqlx::query_scalar("SELECT state FROM models").fetch_one(&pool).await.unwrap();
    let keys = state["stated"]["keys"].as_array().unwrap();
    assert!(keys.contains(&json!("time:is_night_local")), "{keys:?}");
    let x = api_get(&admin_app(&pool), &format!("/api/decisions/{id}")).await.json();
    assert_eq!(x["replay_matches"], json!(true), "{x}");
}

// --- the cohort --------------------------------------------------------------------------------

const VISITORS: usize = 1500;
const MEASURE: usize = 400;
const SETTLE_EVERY: usize = 100;

/// Planted truth: night visitors click the gift hero, day visitors the baseline.
fn truth(variant: &str, night: bool) -> f64 {
    match (variant, night) {
        ("gift", true) => 0.30,
        ("gift", false) => 0.04,
        (_, true) => 0.04,
        (_, false) => 0.25,
    }
}

/// The share of the last MEASURE visitors shown what their hour prefers.
async fn run(pool: &PgPool, seed: u64, report_offset: bool) -> f64 {
    setup_demo(pool).await;
    set_holdout(pool, 0.0).await;
    let app = app(pool);
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut matched = 0usize;
    for i in 0..VISITORS {
        let subject = format!("v{i}");
        let night = rng.random::<f64>() < 0.5;
        let hour = if night { [23, 0, 1, 2, 3][rng.random_range(0..5)] } else { [10, 11, 13, 15, 17][rng.random_range(0..5)] };
        let d = decide_at(&app, &subject, report_offset.then(|| offset_for(hour))).await.json();
        let variant = d["variant"].as_str().unwrap().to_string();
        let id = d["decision_id"].as_str().unwrap();
        let mut evs = vec![json!({ "id": uuid::Uuid::new_v4(), "name": "exposure", "slot": "hero", "decision_id": id })];
        if rng.random::<f64>() < truth(&variant, night) {
            evs.push(json!({ "id": uuid::Uuid::new_v4(), "name": "click", "slot": "hero", "decision_id": id }));
        }
        post(&app, "/e", json!({ "site": "demo", "subject": subject, "events": evs })).await;
        if i >= VISITORS - MEASURE {
            matched += ((variant == "gift") == night) as usize;
        }
        if (i + 1) % SETTLE_EVERY == 0 {
            worker::run_once(pool, Utc::now() + Duration::hours(2)).await.unwrap();
        }
    }
    let m = matched as f64 / MEASURE as f64;
    eprintln!("offset={report_offset} seed={seed}: shown what their hour prefers {m:.2}");
    m
}

#[sqlx::test(migrations = false)]
async fn night_visitors_are_learned_to_prefer_gift(pool: PgPool) {
    let m = run(&pool, 21, true).await;
    assert!(m >= 0.80, "visitors shown what their hour prefers: {m:.2}");
}

/// The control: no offset, so nothing tells the policy the hour; it cannot beat a coin.
#[sqlx::test(migrations = false)]
async fn without_an_offset_the_hour_cannot_be_learned(pool: PgPool) {
    let m = run(&pool, 21, false).await;
    assert!((0.30..=0.65).contains(&m), "shown what their hour prefers: {m:.2}");
}
