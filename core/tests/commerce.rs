//! F1: first-class commerce events. A purchase carries `order_id`, `value`, `currency` and
//! `items`; Core keeps one row per order, credits the order's revenue to the decisions that may
//! have caused it through each slot's attributor (whatever the slot's metric), and results show
//! conversion rate and revenue per visitor per arm with confidence sequences. A slot may ask the
//! bandit to optimise revenue instead of conversions; the default is unchanged.

mod common;

use axum::Router;
use chrono::{Duration, Utc};
use common::*;
use pratibimba::bandit::{Arm, LinTs};
use pratibimba::{results, worker};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

const X: [f64; 4] = [1.0, 1.0, 0.0, 0.0];

/// The demo site (hero, metric click) in INR, no holdout, decisions not sticky so one subject can
/// hold several. `slot` is merged into the hero slot's spec.
async fn setup_with(pool: &PgPool, site: Value, slot: Value) -> Router {
    let mut v = json!({
        "site": "demo", "origin": "http://localhost:5173", "currency": "INR",
        "slots": [{ "key": "hero", "selector": "#hero", "metric": "click", "decision_ttl_secs": 0,
                    "variants": [ { "key": "baseline", "baseline": true }, { "key": "gift", "html": "<h1>Gift sets</h1>" } ] }]
    });
    v.as_object_mut().unwrap().extend(site.as_object().unwrap().clone());
    v["slots"][0].as_object_mut().unwrap().extend(slot.as_object().unwrap().clone());
    pratibimba::db::migrate(pool).await.unwrap();
    pratibimba::site_file::apply(pool, &file_from(v)).await.unwrap();
    set_holdout(pool, 0.0).await;
    app(pool)
}

async fn setup(pool: &PgPool) -> Router {
    setup_with(pool, json!({}), json!({})).await
}

/// `n` exposed decisions for `subject`, oldest first, a minute apart.
async fn touches(app: &Router, pool: &PgPool, subject: &str, n: usize) -> Vec<Uuid> {
    let mut out = vec![];
    for _ in 0..n {
        sqlx::query("UPDATE decisions SET created_at = created_at - interval '1 minute'").execute(pool).await.unwrap();
        let d: Uuid = decide(app, subject, true).await.json()["decision_id"].as_str().unwrap().parse().unwrap();
        event(app, subject, "exposure", Some(&d.to_string())).await;
        out.push(d);
    }
    worker::run_once(pool, Utc::now()).await.unwrap();
    out
}

fn purchase(order: &str, value: Value, currency: &str) -> Value {
    json!({ "id": Uuid::new_v4(), "name": "purchase", "order_id": order, "value": value, "currency": currency,
            "items": [{ "id": "sku-1", "price": 40, "quantity": 2 }] })
}

async fn send_events(app: &Router, subject: &str, events: Vec<Value>) -> Resp {
    post(app, "/e", json!({ "site": "demo", "subject": subject, "events": events })).await
}

async fn revenue(pool: &PgPool, d: Uuid) -> Option<f64> {
    sqlx::query_scalar("SELECT revenue::float8 FROM decisions WHERE id = $1").bind(d).fetch_one(pool).await.unwrap()
}

async fn model(pool: &PgPool) -> LinTs {
    let s: Value = sqlx::query_scalar("SELECT state FROM models").fetch_one(pool).await.unwrap();
    serde_json::from_value(s).unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

// ---- ingest ------------------------------------------------------------------------------------

#[sqlx::test(migrations = false)]
async fn a_purchase_stores_value_currency_order_and_items(pool: PgPool) {
    let app = setup(&pool).await;
    let r = send_events(&app, "v1", vec![purchase("ord-1", json!(80.5), "inr")]).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    let (value, currency, order, items): (f64, String, String, Value) =
        sqlx::query_as("SELECT value::float8, currency, order_id, items FROM events WHERE name = 'purchase'").fetch_one(&pool).await.unwrap();
    assert!(close(value, 80.5));
    assert_eq!(currency, "INR", "currency is stored upper-case");
    assert_eq!(order, "ord-1");
    assert_eq!(items[0]["id"], "sku-1");
}

#[sqlx::test(migrations = false)]
async fn a_purchase_is_idempotent_on_order_id(pool: PgPool) {
    let app = setup(&pool).await;
    // Two sends of one order with different event ids: a thank-you page reloaded, or the browser
    // and the server both reporting it.
    let a = send_events(&app, "v1", vec![purchase("ord-7", json!(100), "INR")]).await;
    let b = send_events(&app, "v1", vec![purchase("ord-7", json!(100), "INR")]).await;
    assert_eq!(a.json()["accepted"], 1);
    assert_eq!(b.json()["accepted"], 0, "the second send of the order was stored again");
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE order_id = 'ord-7'").await, 1);
    // A different order is a different row.
    send_events(&app, "v1", vec![purchase("ord-8", json!(100), "INR")]).await;
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE name = 'purchase'").await, 2);
}

#[sqlx::test(migrations = false)]
async fn the_same_order_id_on_two_sites_is_two_orders(pool: PgPool) {
    let app = setup(&pool).await;
    let mut other = common::demo_file();
    other.site = "other".into();
    pratibimba::site_file::apply(&pool, &other).await.unwrap();
    send_events(&app, "v1", vec![purchase("ord-1", json!(10), "INR")]).await;
    let r = post(&app, "/e", json!({ "site": "other", "subject": "v1", "events": [purchase("ord-1", json!(10), "USD")] })).await;
    assert_eq!(r.json()["accepted"], 1, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn bad_commerce_fields_are_refused(pool: PgPool) {
    let app = setup(&pool).await;
    let cases = [
        json!({ "id": Uuid::new_v4(), "name": "purchase", "order_id": "o", "value": -1, "currency": "INR" }),
        json!({ "id": Uuid::new_v4(), "name": "purchase", "order_id": "o", "value": 1e13, "currency": "INR" }),
        json!({ "id": Uuid::new_v4(), "name": "purchase", "order_id": "o", "value": 10 }),
        json!({ "id": Uuid::new_v4(), "name": "purchase", "order_id": "o", "currency": "INR" }),
        json!({ "id": Uuid::new_v4(), "name": "purchase", "order_id": "o", "value": 10, "currency": "RUPEES" }),
        json!({ "id": Uuid::new_v4(), "name": "purchase", "order_id": "", "value": 10, "currency": "INR" }),
        json!({ "id": Uuid::new_v4(), "name": "purchase", "order_id": "o", "value": 10, "currency": "INR", "items": "sku" }),
        json!({ "id": Uuid::new_v4(), "name": "purchase", "order_id": "o", "value": 10, "currency": "INR", "items": [{ "price": 1 }] }),
    ];
    for c in cases {
        let r = send_events(&app, "v1", vec![c.clone()]).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "accepted {c}: {}", r.text);
    }
    assert_eq!(count(&pool, "SELECT count(*) FROM events").await, 0);
}

#[sqlx::test(migrations = false)]
async fn add_to_cart_carries_items_and_no_revenue(pool: PgPool) {
    let app = setup(&pool).await;
    let d = touches(&app, &pool, "v1", 1).await[0];
    let r = send_events(&app, "v1", vec![json!({ "id": Uuid::new_v4(), "name": "add_to_cart", "items": [{ "id": "sku-1", "price": 499, "quantity": 1 }] })]).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    worker::run_once(&pool, Utc::now()).await.unwrap();
    let items: Value = sqlx::query_scalar("SELECT items FROM events WHERE name = 'add_to_cart'").fetch_one(&pool).await.unwrap();
    assert_eq!(items[0]["price"], 499);
    assert_eq!(revenue(&pool, d).await, None, "an item's price is not revenue");
}

#[sqlx::test(migrations = false)]
async fn the_edge_batch_refuses_commerce_events(pool: PgPool) {
    // The edge queue carries plain events only; an order goes to /e, which validates it.
    setup(&pool).await;
    assert!(pratibimba::http::ebatch::is_special(&serde_json::from_value(purchase("o", json!(1), "INR")).unwrap()));
    assert!(!pratibimba::http::ebatch::is_special(&serde_json::from_value(json!({ "id": Uuid::new_v4(), "name": "click" })).unwrap()));
}

// ---- attribution -------------------------------------------------------------------------------

#[sqlx::test(migrations = false)]
async fn revenue_goes_to_the_latest_touch_under_last_touch_whatever_the_metric(pool: PgPool) {
    let app = setup(&pool).await;
    let ds = touches(&app, &pool, "v1", 2).await;
    send_events(&app, "v1", vec![purchase("ord-1", json!(250.75), "INR")]).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(revenue(&pool, ds[0]).await, None);
    assert!(close(revenue(&pool, ds[1]).await.unwrap(), 250.75));
    // The hero measures clicks: revenue does not make it a conversion.
    let outcome: Option<i16> = sqlx::query_scalar("SELECT outcome FROM decisions WHERE id = $1").bind(ds[1]).fetch_one(&pool).await.unwrap();
    assert_eq!(outcome, None);
}

#[sqlx::test(migrations = false)]
async fn a_named_decision_takes_the_revenue_in_its_own_slot(pool: PgPool) {
    let app = setup(&pool).await;
    let ds = touches(&app, &pool, "v1", 2).await;
    let mut p = purchase("ord-1", json!(90), "INR");
    p["decision_id"] = json!(ds[0]);
    p["slot"] = json!("hero");
    send_events(&app, "v1", vec![p]).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert!(close(revenue(&pool, ds[0]).await.unwrap(), 90.0));
    assert_eq!(revenue(&pool, ds[1]).await, None);
}

#[sqlx::test(migrations = false)]
async fn linear_splits_revenue_and_the_shares_sum_to_the_order_exactly(pool: PgPool) {
    let app = setup_with(&pool, json!({}), json!({ "attribution": "linear" })).await;
    let ds = touches(&app, &pool, "v1", 3).await;
    send_events(&app, "v1", vec![purchase("ord-1", json!(100), "INR")]).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    let mut got = vec![];
    for d in &ds {
        got.push(revenue(&pool, *d).await.unwrap());
    }
    for g in &got {
        assert!((g - 100.0 / 3.0).abs() < 0.001, "{got:?}");
    }
    let total: String = sqlx::query_scalar("SELECT sum(revenue)::text FROM decisions").fetch_one(&pool).await.unwrap();
    assert_eq!(total.trim_end_matches('0').trim_end_matches('.'), "100", "shares do not sum to the order: {total}");
}

#[sqlx::test(migrations = false)]
async fn several_orders_in_the_window_add_up(pool: PgPool) {
    let app = setup(&pool).await;
    let d = touches(&app, &pool, "v1", 1).await[0];
    send_events(&app, "v1", vec![purchase("ord-1", json!(10.25), "INR")]).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    send_events(&app, "v1", vec![purchase("ord-2", json!(5), "INR")]).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert!(close(revenue(&pool, d).await.unwrap(), 15.25));
}

#[sqlx::test(migrations = false)]
async fn an_order_outside_the_window_or_unexposed_takes_no_revenue(pool: PgPool) {
    let app = setup(&pool).await;
    let d = touches(&app, &pool, "v1", 1).await[0];
    // Unexposed: decided, never shown.
    let u: Uuid = decide(&app, "v2", true).await.json()["decision_id"].as_str().unwrap().parse().unwrap();
    sqlx::query("UPDATE decisions SET created_at = now() - interval '2 hours'").bind(d).execute(&pool).await.unwrap();
    send_events(&app, "v1", vec![purchase("ord-1", json!(10), "INR")]).await;
    send_events(&app, "v2", vec![purchase("ord-2", json!(10), "INR")]).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(revenue(&pool, d).await, None, "the window had closed");
    assert_eq!(revenue(&pool, u).await, None, "never exposed");
}

#[sqlx::test(migrations = false)]
async fn an_order_in_another_currency_is_kept_but_takes_no_revenue(pool: PgPool) {
    let app = setup(&pool).await;
    let d = touches(&app, &pool, "v1", 1).await[0];
    send_events(&app, "v1", vec![purchase("ord-1", json!(10), "USD")]).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(revenue(&pool, d).await, None);
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE currency = 'USD'").await, 1);
    let r = results::site(&pool, "demo", results::ALL_TIME, Utc::now() + Duration::hours(1)).await.unwrap().unwrap();
    assert_eq!(r.currency, "INR");
    assert_eq!(r.other_currency_orders, 1);
}

#[sqlx::test(migrations = false)]
async fn a_browser_order_for_a_platform_goal_is_a_hint_not_revenue(pool: PgPool) {
    let app = setup(&pool).await;
    sqlx::query("UPDATE sites SET authoritative_goals = '{purchase}'").execute(&pool).await.unwrap();
    let d = touches(&app, &pool, "v1", 1).await[0];
    send_events(&app, "v1", vec![purchase("ord-1", json!(10), "INR")]).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(revenue(&pool, d).await, None);
}

#[sqlx::test(migrations = false)]
async fn a_generic_platform_conversion_with_a_value_credits_revenue(pool: PgPool) {
    use hmac::{Hmac, Mac};
    let app = setup(&pool).await;
    let admin = admin_app(&pool);
    let r = api_post(&admin, "/api/sites/demo/conversion-sources", json!({ "conversion_signing_secret": "s3cret", "goals": ["purchase"] })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let d = touches(&app, &pool, "v1", 1).await[0];
    let body = json!({ "id": "ord-55", "subject": "v1", "value": 1234.5, "currency": "inr" }).to_string();
    let t = Utc::now().timestamp();
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"s3cret").unwrap();
    mac.update(format!("{t}.{body}").as_bytes());
    let sig: String = mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect();
    let req = axum::http::Request::post("/webhook/conversion/demo")
        .header("pratibimba-signature", format!("t={t},v1={sig}"))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body))
        .unwrap();
    let r = send(&admin, req).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert!(close(revenue(&pool, d).await.unwrap(), 1234.5));
}

#[test]
fn stripe_amounts_are_minor_units_except_zero_decimal_currencies() {
    use pratibimba::conversions::major_units;
    assert!(close(major_units(4900, "usd"), 49.0));
    assert!(close(major_units(4900, "INR"), 49.0));
    assert!(close(major_units(4900, "jpy"), 4900.0));
    assert!(close(major_units(4900, "KRW"), 4900.0));
}

// ---- results -----------------------------------------------------------------------------------

/// Seeds closed, exposed decisions on hero: per row `(variant, holdout, n, conversions, revenue
/// per decision for the first `buyers`, buyers)`.
async fn seed(pool: &PgPool, rows: &[(&str, bool, i64, i64, f64, i64)]) {
    for (variant, held, n, conv, each, buyers) in rows {
        seed_decisions(pool, variant, *held, *n, *conv, 120).await;
        sqlx::query(
            "UPDATE decisions SET revenue = $3 WHERE id IN (
                 SELECT d.id FROM decisions d JOIN variants v ON v.id = d.variant_id
                  WHERE v.key = $1 AND d.in_holdout = $2 AND d.revenue IS NULL ORDER BY d.subject LIMIT $4)",
        )
        .bind(variant)
        .bind(held)
        .bind(each)
        .bind(buyers)
        .execute(pool)
        .await
        .unwrap();
    }
}

#[sqlx::test(migrations = false)]
async fn results_show_conversion_rate_and_revenue_per_visitor_per_arm_with_intervals(pool: PgPool) {
    setup(&pool).await;
    // gift: 400 visitors, 40 buy at 500 = 20,000 -> 50 per visitor; holdout 400, 20 at 500 -> 25.
    seed(&pool, &[("gift", false, 400, 40, 500.0, 40), ("baseline", false, 400, 20, 500.0, 20), ("baseline", true, 400, 20, 500.0, 20)]).await;
    let r = results::site(&pool, "demo", results::ALL_TIME, Utc::now()).await.unwrap().unwrap();
    let hero = &r.slots[0];
    let gift = hero.variants.iter().find(|v| v.key == "gift").unwrap();
    assert!(close(gift.counts.revenue, 20_000.0));
    let rpv = gift.counts.revenue_per_visitor.unwrap();
    assert!(close(rpv.estimate, 50.0), "{rpv:?}");
    assert!(rpv.lo < 50.0 && rpv.hi > 50.0 && rpv.lo >= 0.0, "{rpv:?}");
    let cr = gift.counts.conversion_rate.unwrap();
    assert!(close(cr.estimate, 0.1) && cr.lo < 0.1 && cr.hi > 0.1, "{cr:?}");
    let base = hero.variants.iter().find(|v| v.key == "baseline").unwrap();
    assert!(close(base.counts.revenue_per_visitor.unwrap().estimate, 25.0));
    // Against the holdout (25 per visitor): +25, and the interval brackets it.
    let lift = gift.revenue_lift.unwrap();
    assert!(close(lift.estimate, 25.0) && lift.lo < 25.0 && lift.hi > 25.0, "{lift:?}");
    let slot_lift = hero.revenue_lift.unwrap();
    assert!(close(slot_lift.estimate, 12.5), "{slot_lift:?}");
    assert!(close(hero.holdout.revenue_per_visitor.unwrap().estimate, 25.0));
    assert!(close(r.treated.revenue, 30_000.0));
    // The JSON an agent reads names them.
    let j = serde_json::to_value(&r).unwrap();
    assert!(j["slots"][0]["variants"][1]["counts"]["revenue_per_visitor"]["lo"].is_number(), "{j}");
    assert!(j["slots"][0]["revenue_lift"]["estimate"].is_number());
    assert_eq!(j["currency"], "INR");
}

#[test]
fn the_revenue_sequence_is_scale_free_and_narrows() {
    use pratibimba::stats::{ALPHA, mean_cs};
    // 1,000 visitors, 50 buy at 400: mean 20.
    let (n, s, ss) = (1000u64, 50.0 * 400.0, 50.0 * 400.0 * 400.0);
    let a = mean_cs(s, ss, n, ALPHA).unwrap();
    // The same data in paise: every bound scales by 100.
    let b = mean_cs(s * 100.0, ss * 10_000.0, n, ALPHA).unwrap();
    assert!(close(a.estimate, 20.0));
    assert!((b.lo / 100.0 - a.lo).abs() < 1e-6 && (b.hi / 100.0 - a.hi).abs() < 1e-6, "{a:?} vs {b:?}");
    let big = mean_cs(s * 10.0, ss * 10.0, n * 10, ALPHA).unwrap();
    assert!(big.hi - big.lo < 0.5 * (a.hi - a.lo), "{a:?} vs {big:?}");
    assert!(mean_cs(0.0, 0.0, 0, ALPHA).is_none());
}

#[test]
fn revenue_lift_under_the_null_stays_below_alpha_when_peeking() {
    use pratibimba::stats::{ALPHA, mean_lift_cs};
    use rand::{Rng, SeedableRng};
    // A/A: 5% buy, order value uniform 100-900. Peek every 50 visitors up to 4,000.
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(20261014);
    let runs = 200;
    let mut rejected = 0;
    for _ in 0..runs {
        let (mut t, mut c) = ((0.0, 0.0), (0.0, 0.0));
        let mut hit = false;
        for i in 1..=4000u64 {
            for g in [&mut t, &mut c] {
                if rng.random::<f64>() < 0.05 {
                    let v = 100.0 + 800.0 * rng.random::<f64>();
                    g.0 += v;
                    g.1 += v * v;
                }
            }
            if i % 50 == 0 && mean_lift_cs((t.0, t.1, i), (c.0, c.1, i), ALPHA).unwrap().excludes_zero() {
                hit = true;
                break;
            }
        }
        rejected += hit as usize;
    }
    assert!(rejected as f64 / runs as f64 <= ALPHA, "false positives: {rejected}/{runs}");
}

#[test]
fn a_real_revenue_lift_is_detected() {
    use pratibimba::stats::{ALPHA, mean_lift_cs};
    // 8,000 per group; treated buys at 8%, control at 5%, both at 500.
    let (t, c) = (640.0 * 500.0, 400.0 * 500.0);
    let i = mean_lift_cs((t, 640.0 * 250_000.0, 8000), (c, 400.0 * 250_000.0, 8000), ALPHA).unwrap();
    assert!(i.lo > 0.0, "{i:?}");
}

#[sqlx::test(migrations = false)]
async fn mcp_get_results_carries_revenue(pool: PgPool) {
    setup(&pool).await;
    seed(&pool, &[("gift", false, 100, 10, 200.0, 10), ("baseline", true, 100, 5, 200.0, 5)]).await;
    let token = pratibimba::mcp::issue_token(&pool, "demo", "test agent").await.unwrap();
    let app = admin_app(&pool);
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "get_results", "arguments": { "agent": "test", "justification": "revenue" } } });
    let req = axum::http::Request::post("/mcp")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();
    let r = send(&app, req).await;
    let text = r.json()["result"]["content"][0]["text"].as_str().unwrap().to_string();
    let j: Value = serde_json::from_str(&text).unwrap();
    let gift = j["slots"][0]["variants"].as_array().unwrap().iter().find(|v| v["key"] == "gift").unwrap().clone();
    assert_eq!(gift["counts"]["revenue"], 2000.0);
    assert!(close(gift["counts"]["revenue_per_visitor"]["estimate"].as_f64().unwrap(), 20.0));
    assert!(gift["revenue_lift"]["estimate"].is_number());
}

#[sqlx::test(migrations = false)]
async fn the_decision_detail_shows_its_revenue(pool: PgPool) {
    let app = setup(&pool).await;
    let d = touches(&app, &pool, "v1", 1).await[0];
    send_events(&app, "v1", vec![purchase("ord-1", json!(42), "INR")]).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    let detail = pratibimba::explain::detail(&pool, d, Utc::now()).await.unwrap().unwrap();
    assert!(close(detail.revenue.unwrap(), 42.0));
}

// ---- rollups and insights ----------------------------------------------------------------------

#[sqlx::test(migrations = false)]
async fn rollups_carry_revenue_and_follow_a_late_order(pool: PgPool) {
    let app = setup(&pool).await;
    let d = touches(&app, &pool, "v1", 1).await[0];
    let t0 = Utc::now();
    pratibimba::insights::rollup::run(&pool, t0).await.unwrap();
    let rev = |pool: PgPool| async move {
        sqlx::query_scalar::<_, f64>("SELECT coalesce(sum(revenue), 0)::float8 FROM rollup_hourly WHERE segment_id = 0").fetch_one(&pool).await.unwrap()
    };
    assert!(close(rev(pool.clone()).await, 0.0));
    // The order lands after the first run; the next run picks the hour up again.
    send_events(&app, "v1", vec![purchase("ord-1", json!(300), "INR")]).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert!(close(revenue(&pool, d).await.unwrap(), 300.0));
    pratibimba::insights::rollup::run(&pool, Utc::now() + Duration::seconds(1)).await.unwrap();
    assert!(close(rev(pool.clone()).await, 300.0), "the rollup missed the late order");
    // The heatmap counts settled decisions only, as for its rates: close the window.
    worker::run_once(&pool, Utc::now() + Duration::hours(1)).await.unwrap();
    pratibimba::insights::rollup::run(&pool, Utc::now() + Duration::seconds(2)).await.unwrap();
    let daily: f64 = sqlx::query_scalar("SELECT coalesce(sum(revenue), 0)::float8 FROM rollup_daily WHERE segment_id = 0").fetch_one(&pool).await.unwrap();
    assert!(close(daily, 300.0));
    // The heatmap cell shows revenue per visitor with an interval.
    let admin = admin_app(&pool);
    let h = api_get(&admin, "/api/sites/demo/insights/heatmap?axis=utc").await;
    assert_eq!(h.status, StatusCode::OK, "{}", h.text);
    let cells = h.json()["cells"].as_array().unwrap().clone();
    let total: f64 = cells.iter().map(|c| c["revenue"].as_f64().unwrap()).sum();
    assert!(close(total, 300.0), "{cells:?}");
    let cell = cells.iter().find(|c| c["revenue"].as_f64().unwrap() > 0.0).unwrap();
    assert!(close(cell["revenue_per_visitor"]["estimate"].as_f64().unwrap(), 300.0), "{cell}");
    assert!(cell["conversion_rate"]["hi"].is_number(), "{cell}");
}

// ---- the bandit's goal -------------------------------------------------------------------------

/// One exposed decision on hero for `subject`, a purchase of `value`, and its window closed.
async fn buy_and_close(app: &Router, pool: &PgPool, value: f64) -> Uuid {
    let d = touches(app, pool, "v1", 1).await[0];
    send_events(app, "v1", vec![purchase("ord-1", json!(value), "INR")]).await;
    worker::run_once(pool, Utc::now()).await.unwrap();
    worker::run_once(pool, Utc::now() + Duration::hours(1)).await.unwrap();
    d
}

async fn arm_of(pool: &PgPool, d: Uuid) -> i64 {
    sqlx::query_scalar("SELECT variant_id FROM decisions WHERE id = $1").bind(d).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn a_revenue_goal_teaches_the_order_value_over_the_cap(pool: PgPool) {
    let app = setup_with(&pool, json!({}), json!({ "goal": "revenue", "revenue_cap": 200 })).await;
    let d = buy_and_close(&app, &pool, 100.0).await;
    let arm = arm_of(&pool, d).await;
    let mut want = Arm::prior();
    want.update_credit(&X, 0.5);
    let got = model(&pool).await.arm(arm);
    assert!(got.mean.iter().zip(&want.mean).all(|(a, b)| close(*a, *b)), "learned {:?}, want {:?}", got.mean, want.mean);
}

#[sqlx::test(migrations = false)]
async fn a_revenue_goal_caps_a_huge_order_at_one(pool: PgPool) {
    let app = setup_with(&pool, json!({}), json!({ "goal": "revenue", "revenue_cap": 200 })).await;
    let d = buy_and_close(&app, &pool, 5000.0).await;
    let mut want = Arm::prior();
    want.update_credit(&X, 1.0);
    let got = model(&pool).await.arm(arm_of(&pool, d).await);
    assert!(got.mean.iter().zip(&want.mean).all(|(a, b)| close(*a, *b)));
}

#[sqlx::test(migrations = false)]
async fn the_default_goal_still_learns_conversions_not_revenue(pool: PgPool) {
    let app = setup(&pool).await;
    // A purchase but no click: under the default goal the hero (metric click) learned a miss.
    let d = buy_and_close(&app, &pool, 100.0).await;
    let mut want = Arm::prior();
    want.update_credit(&X, 0.0);
    let got = model(&pool).await.arm(arm_of(&pool, d).await);
    assert!(got.mean.iter().zip(&want.mean).all(|(a, b)| close(*a, *b)), "learned {:?}", got.mean);
    let goal: String = sqlx::query_scalar("SELECT goal FROM slots").fetch_one(&pool).await.unwrap();
    assert_eq!(goal, "conversion");
}

#[sqlx::test(migrations = false)]
async fn a_revenue_goal_learns_a_converted_decision_by_its_revenue_too(pool: PgPool) {
    let app = setup_with(&pool, json!({}), json!({ "goal": "revenue", "revenue_cap": 400 })).await;
    let d = touches(&app, &pool, "v1", 1).await[0];
    // A click (the metric) and an order of 100: the lesson is 100/400, not the click's 1.
    event(&app, "v1", "click", Some(&d.to_string())).await;
    send_events(&app, "v1", vec![purchase("ord-1", json!(100), "INR")]).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    worker::run_once(&pool, Utc::now() + Duration::hours(1)).await.unwrap();
    let mut want = Arm::prior();
    want.update_credit(&X, 0.25);
    let got = model(&pool).await.arm(arm_of(&pool, d).await);
    assert!(got.mean.iter().zip(&want.mean).all(|(a, b)| close(*a, *b)), "learned {:?}", got.mean);
}

#[sqlx::test(migrations = false)]
async fn the_site_file_checks_goal_cap_and_currency(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    let base = json!({ "site": "demo", "origin": "http://x", "slots": [{ "key": "hero", "selector": "#h", "variants": [{ "key": "b", "baseline": true }] }] });
    let bad = [
        (json!({}), json!({ "goal": "revenue" })),
        (json!({}), json!({ "goal": "revenue", "revenue_cap": 0 })),
        (json!({}), json!({ "goal": "profit" })),
        (json!({}), json!({ "revenue_cap": 100 })),
        (json!({ "currency": "rupees" }), json!({})),
    ];
    for (site, slot) in bad {
        let mut v = base.clone();
        v.as_object_mut().unwrap().extend(site.as_object().unwrap().clone());
        v["slots"][0].as_object_mut().unwrap().extend(slot.as_object().unwrap().clone());
        let f = file_from(v.clone());
        assert!(f.validate().is_err(), "accepted {v}");
    }
    let mut ok = base.clone();
    ok["currency"] = json!("EUR");
    ok["slots"][0]["goal"] = json!("revenue");
    ok["slots"][0]["revenue_cap"] = json!(150.5);
    pratibimba::site_file::apply(&pool, &file_from(ok)).await.unwrap();
    let (goal, cap, cur): (String, f64, String) =
        sqlx::query_as("SELECT s.goal, s.revenue_cap::float8, t.currency FROM slots s JOIN sites t ON t.id = s.site_id").fetch_one(&pool).await.unwrap();
    assert_eq!((goal.as_str(), cur.as_str()), ("revenue", "EUR"));
    assert!(close(cap, 150.5));
}

/// Live (Seoul, F1 cohort): the exposure travels the edge queue while the purchase goes straight
/// to Core, so the order was drained before its exposure landed and the revenue was lost (INR
/// 58860 of 80127 credited). An order waits, bounded, while the subject has an unexposed decision
/// in its window; one never shown is drained once the hold runs out.
#[sqlx::test(migrations = false)]
async fn an_order_that_arrives_before_its_exposure_waits_for_it(pool: PgPool) {
    let app = setup(&pool).await;
    let d: Uuid = decide(&app, "v1", true).await.json()["decision_id"].as_str().unwrap().parse().unwrap();
    let u: Uuid = decide(&app, "v2", true).await.json()["decision_id"].as_str().unwrap().parse().unwrap();
    send_events(&app, "v1", vec![purchase("ord-1", json!(1234.5), "INR")]).await;
    send_events(&app, "v2", vec![purchase("ord-2", json!(10), "INR")]).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    event(&app, "v1", "exposure", Some(&d.to_string())).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert!(close(revenue(&pool, d).await.unwrap_or(0.0), 1234.5), "the order drained before its exposure lost its revenue");
    // v2's decision is never shown: its order waits out the hold, then drains with no credit.
    let pending = "SELECT count(*) FROM pending_events";
    assert_eq!(count(&pool, pending).await, 1, "the unshown subject's order did not wait");
    worker::run_once(&pool, Utc::now() + Duration::seconds(worker::ORPHAN_HOLD_SECS + 1)).await.unwrap();
    assert_eq!(count(&pool, pending).await, 0, "the hold is not bounded");
    assert_eq!(revenue(&pool, u).await, None);
}

/// The wait is only for a decision the event could credit. A click that names no decision does
/// not wait for an unexposed decision in a slot measuring another goal (common: a slot decided
/// but never rendered), and a click that names its exposed decision does not wait at all.
#[sqlx::test(migrations = false)]
async fn a_click_does_not_wait_for_an_unexposed_decision_it_cannot_credit(pool: PgPool) {
    let footer = json!({ "key": "footer", "selector": "#footer", "metric": "signup", "decision_ttl_secs": 0,
                         "variants": [ { "key": "plain", "baseline": true }, { "key": "bold", "html": "<b>Join</b>" } ] });
    let hero = json!({ "key": "hero", "selector": "#hero", "metric": "click", "decision_ttl_secs": 0,
                       "variants": [ { "key": "baseline", "baseline": true }, { "key": "gift", "html": "<h1>Gift sets</h1>" } ] });
    let app = setup_with(&pool, json!({ "slots": [hero, footer] }), json!({})).await;
    let d = touches(&app, &pool, "v1", 1).await[0];
    let other = post(&app, "/decide", json!({ "site": "demo", "slot": "footer", "subject": "v1", "ctx": {} })).await;
    assert!(other.json()["decision_id"].is_string(), "{:?}", other.json());
    event(&app, "v1", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    let outcome: Option<i16> = sqlx::query_scalar("SELECT outcome FROM decisions WHERE id = $1").bind(d).fetch_one(&pool).await.unwrap();
    assert_eq!(outcome, Some(1), "the click waited for a decision in a slot that measures another goal");
}
