//! F2: the `sorted` slot kind. A decision serves an order of items; the holdout and a paused
//! site get the declared default order; the cascade teaches misses above the acted-on item, the
//! reward to it, and nothing below.

mod common;
use chrono::{Duration, Utc};
use common::*;
use pratibimba::{bandit::LinTs, worker};
use serde_json::{Value, json};
use sqlx::PgPool;

const ITEMS: [&str; 4] = ["a", "b", "c", "d"];

fn sorted_file(order: &[&str], visible: Option<u32>) -> Value {
    let mut slot = json!({ "key": "grid", "selector": "#grid", "kind": "sorted", "metric": "click",
                           "variants": order.iter().map(|k| json!({ "key": k })).collect::<Vec<_>>() });
    if let Some(v) = visible {
        slot["sorted"] = json!({ "visible": v });
    }
    json!({ "site": "demo", "origin": "http://localhost:5173", "slots": [slot] })
}

async fn setup(pool: &PgPool, order: &[&str], visible: Option<u32>) {
    pratibimba::db::migrate(pool).await.unwrap();
    pratibimba::site_file::apply(pool, &file_from(sorted_file(order, visible))).await.unwrap();
    set_holdout(pool, 0.0).await;
}

async fn apply_err(pool: &PgPool, f: Value) -> String {
    pratibimba::db::migrate(pool).await.unwrap();
    pratibimba::site_file::apply(pool, &file_from(f)).await.unwrap_err().to_string()
}

async fn decide_grid(app: &axum::Router, subject: &str) -> Value {
    let r = post(app, "/decide", json!({ "site": "demo", "slot": "grid", "subject": subject })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json()
}

fn order(d: &Value) -> Vec<String> {
    d["order"].as_array().expect("a sorted decision has an order").iter().map(|k| k.as_str().unwrap().to_string()).collect()
}

async fn id_of(pool: &PgPool, key: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM variants WHERE key = $1").bind(key).fetch_one(pool).await.unwrap()
}

async fn model(pool: &PgPool) -> LinTs {
    serde_json::from_value(sqlx::query_scalar("SELECT state FROM models").fetch_one(pool).await.unwrap()).unwrap()
}

/// Expose `d`, optionally click `item`, and close its window.
async fn close(pool: &PgPool, app: &axum::Router, subject: &str, d: &Value, item: Option<&str>) {
    let id = d["decision_id"].as_str().unwrap();
    event(app, subject, "exposure", Some(id)).await;
    if let Some(item) = item {
        let ev = json!([{ "id": uuid::Uuid::new_v4(), "name": "click", "slot": "grid", "items": [{ "id": item }] }]);
        let r = post(app, "/e", json!({ "site": "demo", "subject": subject, "events": ev })).await;
        assert!(r.status.is_success(), "{}", r.text);
    }
    worker::run_once(pool, Utc::now()).await.unwrap();
    worker::run_once(pool, Utc::now() + Duration::seconds(1801)).await.unwrap();
}

/// The bias weight each item learned: below 0 a miss, above 0 a success, 0 untouched.
async fn bias(pool: &PgPool, key: &str) -> f64 {
    model(pool).await.arm(id_of(pool, key).await).mean[0]
}

#[sqlx::test(migrations = false)]
async fn a_sorted_slot_refuses_html_a_baseline_flag_and_a_bad_visible(pool: PgPool) {
    let mut f = sorted_file(&ITEMS, None);
    f["slots"][0]["variants"][1]["html"] = json!("<p>b</p>");
    assert!(apply_err(&pool, f).await.contains("takes no html"));
    let mut f = sorted_file(&ITEMS, None);
    f["slots"][0]["variants"][0]["baseline"] = json!(true);
    assert!(apply_err(&pool, f).await.contains("no baseline flag"));
    assert!(apply_err(&pool, sorted_file(&ITEMS, Some(5))).await.contains("sorted.visible"));
    assert!(apply_err(&pool, sorted_file(&["a"], None)).await.contains("2-50 items"));
    let mut f = sorted_file(&ITEMS, Some(2));
    f["slots"][0]["kind"] = json!("swap");
    assert!(apply_err(&pool, f).await.contains("only a kind \"sorted\" slot"));
}

#[sqlx::test(migrations = false)]
async fn the_first_listed_item_is_the_baseline_and_an_experiment_is_refused(pool: PgPool) {
    setup(&pool, &ITEMS, None).await;
    assert_eq!(count(&pool, "SELECT count(*) FROM variants WHERE is_baseline AND key = 'a'").await, 1);
    let r = api_post(&admin_app(&pool), "/api/experiments", json!({ "site": "demo", "slot": "grid", "name": "x", "actor": "owner",
        "arms": [{ "variant": "a", "weight": 1 }, { "variant": "b", "weight": 1 }],
        "stops_at": (Utc::now() + Duration::days(7)).to_rfc3339() })).await;
    assert!(r.text.contains("is sorted"), "{} {}", r.status, r.text);
}

#[sqlx::test(migrations = false)]
async fn decide_returns_every_item_ordered_by_its_score(pool: PgPool) {
    setup(&pool, &ITEMS, None).await;
    // Plant a model whose means order the items d, b, a, c, with tight covariances.
    let mut m = LinTs::default();
    for (key, mean) in [("a", -1.0), ("b", 1.0), ("c", -3.0), ("d", 3.0)] {
        let mut arm = pratibimba::bandit::Arm::prior();
        arm.mean[0] = mean;
        for (i, row) in arm.cov.iter_mut().enumerate() {
            row[i] = 1e-6;
        }
        m.arms.insert(id_of(&pool, key).await, arm);
    }
    sqlx::query("UPDATE models SET state = $1").bind(serde_json::to_value(&m).unwrap()).execute(&pool).await.unwrap();
    let app = app(&pool);
    let d = decide_grid(&app, "v1").await;
    assert_eq!(order(&d), vec!["d", "b", "a", "c"]);
    assert_eq!(d["variant"], "d");
    assert_eq!(d["payload"], Value::Null);
    assert_eq!(d["explanation"]["order"], json!(["d", "b", "a", "c"]));
    // Sticky: a reload inside the TTL gets the same decision and the same order.
    let again = decide_grid(&app, "v1").await;
    assert_eq!(again["decision_id"], d["decision_id"]);
    assert_eq!(order(&again), order(&d));
}

#[sqlx::test(migrations = false)]
async fn the_holdout_gets_the_declared_order_not_the_id_order(pool: PgPool) {
    setup(&pool, &ITEMS, None).await;
    // Re-listed in reverse: the ids keep the old order, the default order is the file's.
    pratibimba::site_file::apply(&pool, &file_from(sorted_file(&["d", "c", "b", "a"], None))).await.unwrap();
    set_holdout(&pool, 0.5).await;
    let app = app(&pool);
    let mut held = 0;
    for i in 0..40 {
        let d = decide_grid(&app, &format!("h{i}")).await;
        if d["explanation"]["in_holdout"] == json!(true) {
            held += 1;
            assert_eq!(order(&d), vec!["d", "c", "b", "a"]);
        }
    }
    assert!(held > 0);
}

#[sqlx::test(migrations = false)]
async fn a_paused_site_serves_the_default_order(pool: PgPool) {
    setup(&pool, &["c", "a", "d", "b"], None).await;
    sqlx::query("UPDATE sites SET paused_at = now(), paused_by = 'owner'").execute(&pool).await.unwrap();
    let app = app(&pool);
    for i in 0..10 {
        assert_eq!(order(&decide_grid(&app, &format!("p{i}")).await), vec!["c", "a", "d", "b"]);
    }
}

#[sqlx::test(migrations = false)]
async fn a_click_teaches_misses_above_it_the_success_to_it_and_nothing_below(pool: PgPool) {
    setup(&pool, &ITEMS, None).await;
    let app = app(&pool);
    let d = decide_grid(&app, "v1").await;
    let o = order(&d);
    close(&pool, &app, "v1", &d, Some(&o[2])).await;
    assert!(bias(&pool, &o[0]).await < 0.0, "position 0 was passed over");
    assert!(bias(&pool, &o[1]).await < 0.0, "position 1 was passed over");
    assert!(bias(&pool, &o[2]).await > 0.0, "position 2 was clicked");
    assert_eq!(bias(&pool, &o[3]).await, 0.0, "position 3 may never have been seen");
    let reason: String = sqlx::query_scalar("SELECT reason FROM model_versions ORDER BY version DESC LIMIT 1").fetch_one(&pool).await.unwrap();
    assert_eq!(reason, "learn: ranking");
}

#[sqlx::test(migrations = false)]
async fn no_click_teaches_a_miss_to_the_visible_positions_only(pool: PgPool) {
    setup(&pool, &ITEMS, Some(2)).await;
    let app = app(&pool);
    let d = decide_grid(&app, "v1").await;
    let o = order(&d);
    close(&pool, &app, "v1", &d, None).await;
    assert!(bias(&pool, &o[0]).await < 0.0);
    assert!(bias(&pool, &o[1]).await < 0.0);
    assert_eq!(bias(&pool, &o[2]).await, 0.0);
    assert_eq!(bias(&pool, &o[3]).await, 0.0);
}

#[sqlx::test(migrations = false)]
async fn a_held_out_or_unexposed_decision_teaches_nothing(pool: PgPool) {
    setup(&pool, &ITEMS, None).await;
    set_holdout(&pool, 0.5).await;
    let app = app(&pool);
    // A held-out visitor who clicks, and a treated one never shown the list.
    let (mut held, mut treated) = (None, None);
    for i in 0..40 {
        let s = format!("v{i}");
        let d = decide_grid(&app, &s).await;
        match d["explanation"]["in_holdout"].as_bool().unwrap() {
            true if held.is_none() => held = Some((s, d)),
            false if treated.is_none() => treated = Some(d),
            _ => {}
        }
    }
    let (s, d) = held.expect("a held-out visitor");
    assert!(treated.is_some());
    // No click: a credited held decision is settled at its conversion and never reaches the
    // close, so only an unclicked one exercises the close's holdout guard.
    close(&pool, &app, &s, &d, None).await;
    for k in ITEMS {
        assert_eq!(bias(&pool, k).await, 0.0, "{k} learned");
    }
}

/// F2b: the manifest marks a sorted slot, with its `visible`, and lists its items in the declared
/// default order (re-listed in reverse, so the variant ids disagree with it).
#[sqlx::test(migrations = false)]
async fn the_manifest_marks_a_sorted_slot_and_lists_its_items_in_the_declared_order(pool: PgPool) {
    setup(&pool, &ITEMS, Some(3)).await;
    pratibimba::site_file::apply(&pool, &file_from(sorted_file(&["d", "c", "b", "a"], Some(2)))).await.unwrap();
    let m = get(&app(&pool), "/manifest/demo").await.json();
    let s = &m["slots"][0];
    assert_eq!(s["kind"], "sorted", "{m}");
    assert_eq!(s["sorted"], json!({ "visible": 2 }), "{m}");
    let keys: Vec<&str> = s["variants"].as_array().unwrap().iter().map(|v| v["key"].as_str().unwrap()).collect();
    assert_eq!(keys, ["d", "c", "b", "a"]);
}

/// The shop example's site file applies, and its product grid is a sorted slot whose items are
/// the shop's product slugs in the page's order.
#[sqlx::test(migrations = false)]
async fn the_shop_example_declares_its_product_grid_as_a_sorted_slot(pool: PgPool) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/shop");
    let file: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("site.json")).unwrap()).unwrap();
    pratibimba::db::migrate(&pool).await.unwrap();
    pratibimba::site_file::apply(&pool, &serde_json::from_value(file).unwrap()).await.unwrap();
    let products = std::fs::read_to_string(dir.join("lib/products.ts")).unwrap();
    let slugs: Vec<&str> = products.split("slug: '").skip(1).map(|s| &s[..s.find('\'').unwrap()]).collect();
    assert!(slugs.len() >= 2, "{slugs:?}");
    let m = get(&app(&pool), "/manifest/shop").await.json();
    let grid = m["slots"].as_array().unwrap().iter().find(|s| s["key"] == "grid").expect("a grid slot");
    assert_eq!(grid["kind"], "sorted");
    let keys: Vec<&str> = grid["variants"].as_array().unwrap().iter().map(|v| v["key"].as_str().unwrap()).collect();
    assert_eq!(keys, slugs);
}

// Found in the F2 live run (2026-10-14): the decision detail told a sorted decision as a swap
// ("We showed `estate-filter`"), with no order, and a held-out one as "saw the baseline".
#[sqlx::test(migrations = false)]
async fn explain_tells_a_sorted_decision_by_its_order(pool: PgPool) {
    setup(&pool, &ITEMS, None).await;
    set_holdout(&pool, 0.5).await;
    let (app, admin) = (app(&pool), admin_app(&pool));
    let (mut treated, mut held) = (false, false);
    for i in 0..40 {
        let d = decide_grid(&app, &format!("x{i}")).await;
        let r = api_get(&admin, &format!("/api/decisions/{}", d["decision_id"].as_str().unwrap())).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text);
        let x = r.json();
        let served = order(&d);
        assert_eq!(x["order"], json!(served));
        let s = x["sentence"].as_str().unwrap();
        let listed = served.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", ");
        assert!(s.starts_with(&format!("We ordered {listed} for visitor")), "{s}");
        if d["explanation"]["in_holdout"] == json!(true) {
            held = true;
            assert!(s.contains("saw the declared default order"), "{s}");
        } else {
            treated = true;
            assert!(s.contains(&format!("scored `{}` first at", served[0])), "{s}");
        }
    }
    assert!(treated && held);
}
