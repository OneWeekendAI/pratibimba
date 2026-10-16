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

/// Post one event for `subject` on the grid, with optional items.
async fn grid_event(app: &axum::Router, subject: &str, name: &str, decision: Option<&str>, items: &[&str]) {
    let mut ev = json!({ "id": uuid::Uuid::new_v4(), "name": name, "slot": "grid", "decision_id": decision });
    if !items.is_empty() {
        ev["items"] = json!(items.iter().map(|k| json!({ "id": k })).collect::<Vec<_>>());
    }
    let r = post(app, "/e", json!({ "site": "demo", "subject": subject, "events": [ev] })).await;
    assert!(r.status.is_success(), "{}", r.text);
}

/// G3: per-item results read the order the exposure says was rendered (falling back to the
/// stored order), count only treated closed exposed decisions, and follow the cascade.
#[sqlx::test(migrations = false)]
async fn per_item_results_follow_the_rendered_order_and_the_cascade(pool: PgPool) {
    setup(&pool, &ITEMS, Some(2)).await;
    let app = app(&pool);
    // s1: the page rendered the order reversed (plus an unknown card), and s1 clicked its second card.
    let d1 = decide_grid(&app, "s1").await;
    let mut rendered: Vec<String> = order(&d1).into_iter().rev().collect();
    let id1 = d1["decision_id"].as_str().unwrap();
    let mut shown: Vec<&str> = rendered.iter().map(String::as_str).collect();
    shown.insert(1, "zz");
    grid_event(&app, "s1", "exposure", Some(id1), &shown).await;
    grid_event(&app, "s1", "click", None, &[&rendered[1]]).await;
    // s2: an exposure with no items (the stored order stands), no click.
    let d2 = decide_grid(&app, "s2").await;
    grid_event(&app, "s2", "exposure", d2["decision_id"].as_str(), &[]).await;
    // s3: clicked, then moved to the holdout: it must not count.
    let d3 = decide_grid(&app, "s3").await;
    grid_event(&app, "s3", "exposure", d3["decision_id"].as_str(), &[]).await;
    grid_event(&app, "s3", "click", None, &[&order(&d3)[0]]).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    let later = Utc::now() + Duration::seconds(1801);
    worker::run_once(&pool, later).await.unwrap();
    sqlx::query("UPDATE decisions SET in_holdout = true WHERE subject = 's3'").execute(&pool).await.unwrap();
    // Age everything two hours, so the windows are closed for the admin API's own clock too.
    sqlx::query("UPDATE decisions SET created_at = created_at - interval '2 hours'").execute(&pool).await.unwrap();
    sqlx::query("UPDATE events SET received_at = received_at - interval '2 hours'").execute(&pool).await.unwrap();
    // s4: exposed and clicked, but its window is still open: it must not count yet.
    let d4 = decide_grid(&app, "s4").await;
    grid_event(&app, "s4", "exposure", d4["decision_id"].as_str(), &[]).await;
    grid_event(&app, "s4", "click", None, &[&order(&d4)[0]]).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();

    let keys: Vec<String> = ITEMS.iter().map(|s| s.to_string()).collect();
    let acted = rendered[1].clone();
    rendered.retain(|k| k != "zz");
    let want = pratibimba::sorted::tally(&keys, &[(rendered, vec![acted]), (order(&d2), vec![])], 2);
    let r = pratibimba::results::site(&pool, "demo", pratibimba::results::ALL_TIME, Utc::now()).await.unwrap().unwrap();
    let grid = r.slots.iter().find(|s| s.key == "grid").unwrap();
    assert_eq!(grid.treated.n, 2, "the rollup still counts decisions");
    assert_eq!(grid.items.as_deref(), Some(&want[..]));
    assert_eq!(want.iter().map(|i| i.acted).sum::<u64>(), 1);

    // The admin API carries them, on the slot and in the site's results.
    let admin = admin_app(&pool);
    let detail = api_get(&admin, "/api/sites/demo/slots/grid").await.json();
    assert_eq!(detail["items"], serde_json::to_value(&want).unwrap(), "{detail}");
    let all = api_get(&admin, "/api/sites/demo/results").await.json();
    assert!(all["results"]["slots"][0]["items"].is_array(), "{all}");
}

/// G3: a swap slot has no per-item results.
#[sqlx::test(migrations = false)]
async fn a_swap_slot_has_no_item_results(pool: PgPool) {
    setup_demo(&pool).await;
    let r = pratibimba::results::site(&pool, "demo", pratibimba::results::ALL_TIME, Utc::now()).await.unwrap().unwrap();
    assert!(r.slots.iter().all(|s| s.items.is_none()));
    let detail = api_get(&admin_app(&pool), "/api/sites/demo/slots/hero").await.json();
    assert!(detail["items"].is_null(), "{detail}");
}

/// Bug fix 2026-10-16: the learner reads the order the exposure says was rendered, as the
/// per-item table does, so a page that reorders or omits cards teaches what it showed.
/// Rendered: the stored order reversed with its top item omitted and an unknown card added.
async fn close_rendered(pool: &PgPool, app: &axum::Router, subject: &str, click_at: Option<usize>) -> (Vec<String>, String) {
    let d = decide_grid(app, subject).await;
    let o = order(&d);
    let rendered: Vec<String> = o[1..].iter().rev().cloned().collect();
    let mut shown: Vec<&str> = rendered.iter().map(String::as_str).collect();
    shown.insert(1, "zz");
    grid_event(app, subject, "exposure", d["decision_id"].as_str(), &shown).await;
    if let Some(p) = click_at {
        grid_event(app, subject, "click", None, &[&rendered[p]]).await;
    }
    worker::run_once(pool, Utc::now()).await.unwrap();
    worker::run_once(pool, Utc::now() + Duration::seconds(1801)).await.unwrap();
    (rendered, o[0].clone())
}

#[sqlx::test(migrations = false)]
async fn learning_follows_the_rendered_order_not_the_stored_one(pool: PgPool) {
    setup(&pool, &ITEMS, None).await;
    let app = app(&pool);
    let (r, omitted) = close_rendered(&pool, &app, "v1", Some(1)).await;
    assert!(bias(&pool, &r[0]).await < 0.0, "rendered first was passed over");
    assert!(bias(&pool, &r[1]).await > 0.0, "rendered second was clicked");
    assert_eq!(bias(&pool, &r[2]).await, 0.0, "rendered below the click: nothing");
    assert_eq!(bias(&pool, &omitted).await, 0.0, "the card the page omitted was never seen");
}

#[sqlx::test(migrations = false)]
async fn no_click_misses_the_first_visible_rendered_cards(pool: PgPool) {
    setup(&pool, &ITEMS, Some(2)).await;
    let app = app(&pool);
    let (r, omitted) = close_rendered(&pool, &app, "v1", None).await;
    assert!(bias(&pool, &r[0]).await < 0.0);
    assert!(bias(&pool, &r[1]).await < 0.0);
    assert_eq!(bias(&pool, &r[2]).await, 0.0);
    assert_eq!(bias(&pool, &omitted).await, 0.0);
}

#[sqlx::test(migrations = false)]
async fn a_rendered_card_the_file_no_longer_lists_teaches_nothing(pool: PgPool) {
    setup(&pool, &ITEMS, None).await;
    let app = app(&pool);
    let d = decide_grid(&app, "v1").await;
    // The file drops item d (it is retired), but a stale page still renders its card first.
    pratibimba::site_file::apply(&pool, &file_from(sorted_file(&["a", "b", "c"], None))).await.unwrap();
    let rest: Vec<String> = order(&d).into_iter().filter(|k| k != "d").collect();
    let shown = ["d", &rest[0], &rest[1]];
    grid_event(&app, "v1", "exposure", d["decision_id"].as_str(), &shown).await;
    grid_event(&app, "v1", "click", None, &[&rest[0]]).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    worker::run_once(&pool, Utc::now() + Duration::seconds(1801)).await.unwrap();
    assert_eq!(bias(&pool, "d").await, 0.0, "a retired card is not a position");
    assert!(bias(&pool, &rest[0]).await > 0.0, "the click is on the first declared card");
}
