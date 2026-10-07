//! Cold start: a new slot gives every approved arm an equal turn for its first N exposures
//! (per slot, default 100), then hands over to LinTS. The posterior learns all along, and
//! the decision explanation says when cold start chose.

mod common;
use chrono::{Duration, Utc};
use common::*;
use pratibimba::worker;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::collections::HashMap;

/// The demo site with a third arm, no holdout, and a cold start of `n` exposures.
async fn setup(pool: &PgPool, n: i32) {
    setup_demo(pool).await;
    set_holdout(pool, 0.0).await;
    insert_variant(pool, "sale", "approved").await;
    sqlx::query("UPDATE slots SET cold_start_exposures = $1").bind(n).execute(pool).await.unwrap();
}

async fn decisions(app: &axum::Router, n: usize, prefix: &str) -> Vec<Value> {
    let mut out = vec![];
    for i in 0..n {
        out.push(decide(app, &format!("{prefix}{i}"), false).await.json());
    }
    out
}

fn tally(ds: &[Value]) -> HashMap<String, usize> {
    let mut t = HashMap::new();
    for d in ds {
        *t.entry(d["variant"].as_str().unwrap().to_string()).or_default() += 1;
    }
    t
}

#[sqlx::test(migrations = false)]
async fn cold_start_gives_every_arm_an_equal_turn(pool: PgPool) {
    setup(&pool, 100).await;
    let t = tally(&decisions(&app(&pool), 30, "v").await);
    assert_eq!((t.get("baseline"), t.get("gift"), t.get("sale")), (Some(&10), Some(&10), Some(&10)), "{t:?}");
}

#[sqlx::test(migrations = false)]
async fn cold_start_is_in_the_explanation(pool: PgPool) {
    setup(&pool, 100).await;
    let d = decide(&app(&pool), "v1", false).await.json();
    assert_eq!(d["explanation"]["cold_start"], json!({ "exposures": 0, "until": 100 }));
}

#[sqlx::test(migrations = false)]
async fn cold_start_keeps_the_model_policy_and_its_scores(pool: PgPool) {
    // The scores are still LinTS's (the replay and the counterfactual need them).
    setup(&pool, 100).await;
    let d = decide(&app(&pool), "v1", false).await.json();
    assert_eq!(d["explanation"]["policy"], "lints");
    assert_eq!(d["explanation"]["scores"].as_object().unwrap().len(), 3, "{d}");
}

#[sqlx::test(migrations = false)]
async fn cold_start_hands_over_to_lints_after_n_exposures(pool: PgPool) {
    setup(&pool, 5).await;
    seed_decisions(&pool, "gift", false, 5, 0, 60).await;
    let d = decide(&app(&pool), "v1", false).await.json();
    assert_eq!(d["explanation"]["cold_start"], Value::Null, "{d}");
}

#[sqlx::test(migrations = false)]
async fn cold_start_runs_until_the_nth_exposure(pool: PgPool) {
    setup(&pool, 5).await;
    seed_decisions(&pool, "gift", false, 4, 0, 60).await;
    let d = decide(&app(&pool), "v1", false).await.json();
    assert_eq!(d["explanation"]["cold_start"], json!({ "exposures": 4, "until": 5 }));
}

#[sqlx::test(migrations = false)]
async fn cold_start_counts_only_exposed_decisions(pool: PgPool) {
    // A decision the page never showed is not an exposure.
    setup(&pool, 5).await;
    seed_decisions(&pool, "gift", false, 5, 0, 60).await;
    sqlx::query("UPDATE decisions SET exposed_at = NULL").execute(&pool).await.unwrap();
    let d = decide(&app(&pool), "v1", false).await.json();
    assert_eq!(d["explanation"]["cold_start"]["exposures"], 0, "{d}");
}

#[sqlx::test(migrations = false)]
async fn cold_start_does_not_count_holdout_exposures(pool: PgPool) {
    // A holdout visitor teaches the model nothing, so it does not end the cold start.
    setup(&pool, 5).await;
    seed_decisions(&pool, "baseline", true, 5, 0, 60).await;
    let d = decide(&app(&pool), "v1", false).await.json();
    assert_eq!(d["explanation"]["cold_start"]["exposures"], 0, "{d}");
}

#[sqlx::test(migrations = false)]
async fn cold_start_of_zero_is_off(pool: PgPool) {
    setup(&pool, 0).await;
    let d = decide(&app(&pool), "v1", false).await.json();
    assert_eq!(d["explanation"]["cold_start"], Value::Null, "{d}");
}

#[sqlx::test(migrations = false)]
async fn cold_start_never_picks_an_arm_outside_the_visitors_share(pool: PgPool) {
    setup(&pool, 100).await;
    sqlx::query("UPDATE variants SET traffic_share = 0.01 WHERE key = 'sale'").execute(&pool).await.unwrap();
    let sale = variant_id(&pool, "sale").await;
    decisions(&app(&pool), 30, "v").await;
    let subjects: Vec<String> = sqlx::query_scalar("SELECT subject FROM decisions WHERE variant_id = $1").bind(sale).fetch_all(&pool).await.unwrap();
    assert!(subjects.iter().all(|s| pratibimba::hash::within_share(sale, s, 0.01)), "{subjects:?}");
}

#[sqlx::test(migrations = false)]
async fn cold_start_holdout_visitors_still_get_the_baseline(pool: PgPool) {
    setup(&pool, 100).await;
    set_holdout(&pool, 0.5).await;
    let ds = decisions(&app(&pool), 40, "v").await;
    let held: Vec<&Value> = ds.iter().filter(|d| d["explanation"]["in_holdout"] == true).collect();
    assert!(!held.is_empty());
    assert!(held.iter().all(|d| d["variant"] == "baseline" && d["explanation"]["cold_start"].is_null()), "{held:?}");
}

#[sqlx::test(migrations = false)]
async fn cold_start_sticky_reload_keeps_its_explanation(pool: PgPool) {
    setup(&pool, 100).await;
    let app = app(&pool);
    let first = decide(&app, "v1", false).await.json();
    let again = decide(&app, "v1", false).await.json();
    assert_eq!(again["decision_id"], first["decision_id"]);
    assert_eq!(again["explanation"]["cold_start"], json!({ "exposures": 0, "until": 100 }));
}

#[sqlx::test(migrations = false)]
async fn cold_start_decisions_still_teach_the_posterior(pool: PgPool) {
    setup(&pool, 100).await;
    let app = app(&pool);
    let before: Value = sqlx::query_scalar("SELECT state FROM models").fetch_one(&pool).await.unwrap();
    decisions(&app, 12, "v").await;
    // Expose and click every decision; then let the windows close.
    let rows: Vec<(uuid::Uuid, String)> = sqlx::query_as("SELECT id, subject FROM decisions").fetch_all(&pool).await.unwrap();
    for (id, subject) in &rows {
        event(&app, subject, "exposure", Some(&id.to_string())).await;
        event(&app, subject, "click", Some(&id.to_string())).await;
    }
    worker::run_once(&pool, Utc::now() + Duration::hours(2)).await.unwrap();
    let (version, after): (i64, Value) = sqlx::query_as("SELECT version, state FROM models").fetch_one(&pool).await.unwrap();
    assert!(version > 0 && after != before, "the model learned nothing from {} cold-start decisions", rows.len());
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions WHERE cold_start IS NOT NULL AND outcome = 1").await, 12);
}

#[sqlx::test(migrations = false)]
async fn cold_start_is_told_in_the_decision_sentence(pool: PgPool) {
    setup(&pool, 100).await;
    let d = decide(&app(&pool), "v1", false).await.json();
    let id = d["decision_id"].as_str().unwrap().parse().unwrap();
    let detail = pratibimba::explain::detail(&pool, id, Utc::now()).await.unwrap().unwrap();
    assert!(detail.sentence.contains("cold start (0 of 100 exposures)"), "{}", detail.sentence);
    assert_eq!(detail.cold_start, Some(json!({ "exposures": 0, "until": 100 })));
}

#[sqlx::test(migrations = false)]
async fn cold_start_is_not_told_once_it_is_over(pool: PgPool) {
    setup(&pool, 0).await;
    let d = decide(&app(&pool), "v1", false).await.json();
    let id = d["decision_id"].as_str().unwrap().parse().unwrap();
    let detail = pratibimba::explain::detail(&pool, id, Utc::now()).await.unwrap().unwrap();
    assert!(!detail.sentence.contains("cold start"), "{}", detail.sentence);
}

// --- the per-slot setting, from the site file --------------------------------------------------

fn hero_with(extra: Value) -> pratibimba::site_file::SiteFile {
    let mut slot = json!({ "key": "hero", "selector": "#hero",
                           "variants": [ { "key": "baseline", "baseline": true }, { "key": "gift", "html": "<h1>Gift sets</h1>" } ] });
    slot.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    serde_json::from_value(json!({ "site": "demo", "origin": "http://localhost:5173", "slots": [slot] })).unwrap()
}

#[sqlx::test(migrations = false)]
async fn cold_start_defaults_to_100_exposures(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    pratibimba::site_file::apply(&pool, &hero_with(json!({}))).await.unwrap();
    assert_eq!(count(&pool, "SELECT cold_start_exposures::bigint FROM slots").await, 100);
}

#[sqlx::test(migrations = false)]
async fn cold_start_is_set_per_slot_in_the_site_file(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    pratibimba::site_file::apply(&pool, &hero_with(json!({ "cold_start_exposures": 20 }))).await.unwrap();
    assert_eq!(count(&pool, "SELECT cold_start_exposures::bigint FROM slots").await, 20);
}

#[test]
fn cold_start_that_is_negative_is_refused() {
    let e = hero_with(json!({ "cold_start_exposures": -1 })).validate().unwrap_err().to_string();
    assert!(e.contains("cold_start_exposures"), "{e}");
}

#[test]
fn cold_start_of_zero_is_accepted() {
    hero_with(json!({ "cold_start_exposures": 0 })).validate().unwrap();
}

#[sqlx::test(migrations = false)]
async fn cold_start_changes_when_the_site_file_is_applied_again(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    pratibimba::site_file::apply(&pool, &hero_with(json!({}))).await.unwrap();
    pratibimba::site_file::apply(&pool, &hero_with(json!({ "cold_start_exposures": 0 }))).await.unwrap();
    assert_eq!(count(&pool, "SELECT cold_start_exposures::bigint FROM slots").await, 0);
}
