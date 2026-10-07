//! B1: the attributor is pluggable. `last_touch` (the default) credits the latest exposed decision;
//! `linear` splits one conversion evenly across the subject's open exposed decisions in the window,
//! and the bandit learns the fractional reward. Results count credit, so one conversion is one
//! conversion whichever attributor ran.

mod common;

use axum::{Router, body::Body, http::Request};
use chrono::{Duration, Utc};
use common::*;
use hmac::{Hmac, Mac};
use pratibimba::attribution::{self, Attributor, LastTouch, Linear, Touch};
use pratibimba::bandit::{Arm, LinTs};
use pratibimba::{results, worker};
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::PgPool;
use uuid::Uuid;

const X: [f64; 4] = [1.0, 1.0, 0.0, 0.0];

fn file(site: Option<&str>, slot: Option<&str>) -> pratibimba::site_file::SiteFile {
    let mut v = json!({
        "site": "demo", "origin": "http://localhost:5173",
        "slots": [{ "key": "hero", "selector": "#hero", "metric": "click",
                    "variants": [ { "key": "baseline", "baseline": true }, { "key": "gift", "html": "<h1>Gift sets</h1>" } ] }]
    });
    if let Some(a) = site {
        v["attribution"] = json!(a);
    }
    if let Some(a) = slot {
        v["slots"][0]["attribution"] = json!(a);
    }
    file_from(v)
}

async fn setup_with(pool: &PgPool, f: &pratibimba::site_file::SiteFile) -> Router {
    pratibimba::db::migrate(pool).await.unwrap();
    pratibimba::site_file::apply(pool, f).await.unwrap();
    set_holdout(pool, 0.0).await;
    app(pool)
}

async fn setup_linear(pool: &PgPool) -> Router {
    setup_with(pool, &file(None, Some("linear"))).await
}

/// `n` exposed decisions for `subject`, oldest first, one minute apart (so stickiness never
/// hands back the same decision).
async fn touches(app: &Router, pool: &PgPool, subject: &str, n: usize) -> Vec<Uuid> {
    let mut out = vec![];
    for _ in 0..n {
        sqlx::query("UPDATE decisions SET created_at = created_at - interval '1 minute'").execute(pool).await.unwrap();
        let d: Uuid = decide(app, subject, true).await.json()["decision_id"].as_str().unwrap().parse().unwrap();
        event(app, subject, "exposure", Some(&d.to_string())).await;
        worker::run_once(pool, Utc::now()).await.unwrap();
        out.push(d);
    }
    out
}

async fn record(pool: &PgPool, d: Uuid) -> (Option<i16>, Option<f64>) {
    sqlx::query_as("SELECT outcome, credit FROM decisions WHERE id = $1").bind(d).fetch_one(pool).await.unwrap()
}

async fn effective(pool: &PgPool) -> String {
    sqlx::query_scalar("SELECT coalesce(s.attribution, t.attribution) FROM slots s JOIN sites t ON t.id = s.site_id WHERE s.key = 'hero'")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn model(pool: &PgPool) -> (i64, LinTs) {
    let (v, s): (i64, Value) = sqlx::query_as("SELECT version, state FROM models").fetch_one(pool).await.unwrap();
    (v, serde_json::from_value(s).unwrap())
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn ids(n: u128) -> Vec<Uuid> {
    (1..=n).map(Uuid::from_u128).collect()
}

fn open(ids: &[Uuid]) -> Vec<Touch> {
    ids.iter().map(|&id| Touch { id, open: true }).collect()
}

// ---- the attributors themselves ----

#[test]
fn last_touch_credits_only_the_latest_touch() {
    let t = ids(3);
    assert_eq!(LastTouch.credit(&open(&t), None), vec![(t[2], 1.0)]);
}

#[test]
fn last_touch_credits_the_named_decision_in_full() {
    let t = ids(3);
    assert_eq!(LastTouch.credit(&open(&t), Some(t[0])), vec![(t[0], 1.0)]);
}

#[test]
fn last_touch_refuses_a_named_decision_that_is_not_a_touch() {
    assert_eq!(LastTouch.credit(&open(&ids(2)), Some(Uuid::from_u128(9))), vec![]);
}

#[test]
fn linear_splits_evenly_across_open_touches() {
    let t = ids(4);
    let c = Linear.credit(&open(&t), None);
    assert_eq!(c.iter().map(|x| x.0).collect::<Vec<_>>(), t);
    assert!(c.iter().all(|x| close(x.1, 0.25)), "{c:?}");
}

#[test]
fn linear_skips_touches_that_are_not_open() {
    let t = ids(3);
    let mut touches = open(&t);
    touches[0].open = false;
    let c = Linear.credit(&touches, None);
    assert_eq!(c, vec![(t[1], 0.5), (t[2], 0.5)]);
}

#[test]
fn linear_refuses_a_named_decision_that_is_not_a_touch() {
    assert_eq!(Linear.credit(&open(&ids(2)), Some(Uuid::from_u128(9))), vec![]);
}

#[test]
fn linear_accepts_a_named_decision_and_spreads_across_the_slot() {
    let t = ids(2);
    assert_eq!(Linear.credit(&open(&t), Some(t[1])), vec![(t[0], 0.5), (t[1], 0.5)]);
}

#[test]
fn attributor_names_resolve() {
    assert_eq!(attribution::by_name("last_touch").map(|a| a.name()), Some("last_touch"));
    assert_eq!(attribution::by_name("linear").map(|a| a.name()), Some("linear"));
}

#[test]
fn an_unknown_attributor_name_resolves_to_nothing() {
    assert!(attribution::by_name("shapley").is_none());
}

// ---- fractional rewards in the bandit ----

#[test]
fn credit_of_one_teaches_like_a_conversion() {
    let (mut a, mut b) = (Arm::default(), Arm::default());
    a.update(&X, true);
    b.update_credit(&X, 1.0);
    assert_eq!((a.mean, a.cov), (b.mean, b.cov));
}

#[test]
fn credit_of_zero_teaches_like_a_miss() {
    let (mut a, mut b) = (Arm::default(), Arm::default());
    a.update(&X, false);
    b.update_credit(&X, 0.0);
    assert_eq!((a.mean, a.cov), (b.mean, b.cov));
}

#[test]
fn half_credit_lands_between_a_miss_and_a_conversion() {
    let score = |y: f64| {
        let mut a = Arm::default();
        a.update_credit(&X, y);
        a.mean.iter().zip(&X).map(|(m, x)| m * x).sum::<f64>()
    };
    let (miss, half, conv) = (score(0.0), score(0.5), score(1.0));
    assert!(miss < half && half < conv, "{miss} {half} {conv}");
}

#[test]
fn a_shared_correction_moves_the_mean_by_its_share() {
    let (mut full, mut half) = (Arm::default(), Arm::default());
    full.flip_miss(&X);
    half.flip_miss_by(&X, 0.5);
    for i in 0..4 {
        assert!(close(half.mean[i], 0.5 * full.mean[i]), "{:?} vs {:?}", half.mean, full.mean);
    }
}

// ---- the site file chooses ----

#[sqlx::test(migrations = false)]
async fn last_touch_is_the_default(pool: PgPool) {
    let _ = setup_with(&pool, &file(None, None)).await;
    assert_eq!(effective(&pool).await, "last_touch");
}

#[sqlx::test(migrations = false)]
async fn site_attribution_applies_to_its_slots(pool: PgPool) {
    let _ = setup_with(&pool, &file(Some("linear"), None)).await;
    assert_eq!(effective(&pool).await, "linear");
}

#[sqlx::test(migrations = false)]
async fn slot_attribution_overrides_the_site(pool: PgPool) {
    let _ = setup_with(&pool, &file(Some("linear"), Some("last_touch"))).await;
    assert_eq!(effective(&pool).await, "last_touch");
}

#[sqlx::test(migrations = false)]
async fn omitted_site_attribution_keeps_the_current_value(pool: PgPool) {
    let _ = setup_with(&pool, &file(Some("linear"), None)).await;
    pratibimba::site_file::apply(&pool, &file(None, None)).await.unwrap();
    assert_eq!(effective(&pool).await, "linear");
}

#[test]
fn an_unknown_site_attribution_is_refused() {
    let e = file(Some("shapley"), None).validate().unwrap_err().to_string();
    assert!(e.contains("attribution"), "{e}");
}

#[test]
fn a_known_site_attribution_is_accepted() {
    file(Some("linear"), None).validate().unwrap();
}

#[test]
fn an_unknown_slot_attribution_is_refused() {
    let e = file(None, Some("first_touch")).validate().unwrap_err().to_string();
    assert!(e.contains("attribution"), "{e}");
}

#[test]
fn a_known_slot_attribution_is_accepted() {
    file(None, Some("last_touch")).validate().unwrap();
}

// ---- the worker under linear ----

#[sqlx::test(migrations = false)]
async fn linear_credits_every_exposed_decision_in_the_window(pool: PgPool) {
    let app = setup_linear(&pool).await;
    let t = touches(&app, &pool, "v", 3).await;
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    for d in t {
        let (o, c) = record(&pool, d).await;
        assert_eq!(o, Some(1));
        assert!(close(c.unwrap(), 1.0 / 3.0), "{c:?}");
    }
}

#[sqlx::test(migrations = false)]
async fn linear_spreads_a_named_goal_across_the_slot(pool: PgPool) {
    let app = setup_linear(&pool).await;
    let t = touches(&app, &pool, "v", 2).await;
    event(&app, "v", "click", Some(&t[1].to_string())).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(record(&pool, t[0]).await, (Some(1), Some(0.5)));
    assert_eq!(record(&pool, t[1]).await, (Some(1), Some(0.5)));
}

#[sqlx::test(migrations = false)]
async fn last_touch_records_full_credit(pool: PgPool) {
    let app = setup_with(&pool, &file(None, None)).await;
    let t = touches(&app, &pool, "v", 2).await;
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(record(&pool, t[1]).await, (Some(1), Some(1.0)));
    assert_eq!(record(&pool, t[0]).await, (None, None));
}

#[sqlx::test(migrations = false)]
async fn linear_ignores_an_unexposed_decision(pool: PgPool) {
    let app = setup_linear(&pool).await;
    let t = touches(&app, &pool, "v", 1).await;
    sqlx::query("UPDATE decisions SET created_at = created_at - interval '1 minute'").execute(&pool).await.unwrap();
    let hidden: Uuid = decide(&app, "v", true).await.json()["decision_id"].as_str().unwrap().parse().unwrap();
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(record(&pool, t[0]).await, (Some(1), Some(1.0)));
    assert_eq!(record(&pool, hidden).await, (None, None));
}

#[sqlx::test(migrations = false)]
async fn linear_ignores_a_decision_outside_the_window(pool: PgPool) {
    let app = setup_linear(&pool).await;
    let t = touches(&app, &pool, "v", 2).await;
    sqlx::query("UPDATE decisions SET created_at = now() - interval '2 hours' WHERE id = $1").bind(t[0]).execute(&pool).await.unwrap();
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(record(&pool, t[1]).await, (Some(1), Some(1.0)));
    assert_eq!(record(&pool, t[0]).await.1, None);
}

#[sqlx::test(migrations = false)]
async fn linear_never_credits_a_decision_twice(pool: PgPool) {
    let app = setup_linear(&pool).await;
    let first = touches(&app, &pool, "v", 1).await;
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    let second = touches(&app, &pool, "v", 1).await;
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(record(&pool, first[0]).await, (Some(1), Some(1.0)));
    assert_eq!(record(&pool, second[0]).await, (Some(1), Some(1.0)));
}

#[sqlx::test(migrations = false)]
async fn linear_teaches_the_bandit_each_share(pool: PgPool) {
    let app = setup_linear(&pool).await;
    let t = touches(&app, &pool, "v", 2).await;
    let before = model(&pool).await;
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    let after = model(&pool).await;
    assert_eq!(after.0, before.0 + 2, "one model version per credited decision");
    let mut expected = before.1.clone();
    for d in &t {
        let (v, f): (i64, Value) = sqlx::query_as("SELECT variant_id, features FROM decisions WHERE id = $1").bind(d).fetch_one(&pool).await.unwrap();
        let x = pratibimba::http::decide::from_named(&f).unwrap();
        expected.update_credit(v, &x, 0.5);
    }
    assert_eq!(serde_json::to_value(&after.1).unwrap(), serde_json::to_value(&expected).unwrap());
}

#[sqlx::test(migrations = false)]
async fn linear_holdout_records_credit_but_never_learns(pool: PgPool) {
    let app = setup_linear(&pool).await;
    let t = touches(&app, &pool, "v", 2).await;
    sqlx::query("UPDATE decisions SET in_holdout = true").execute(&pool).await.unwrap();
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    assert_eq!(record(&pool, t[0]).await, (Some(1), Some(0.5)));
    assert_eq!(model(&pool).await.0, 0);
}

// ---- results stay consistent ----

#[sqlx::test(migrations = false)]
async fn results_count_one_linear_conversion_once(pool: PgPool) {
    let app = setup_linear(&pool).await;
    touches(&app, &pool, "v", 4).await;
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    let r = results::site(&pool, "demo", results::ALL_TIME, Utc::now() + Duration::hours(1)).await.unwrap().unwrap();
    assert_eq!(r.treated.n, 4);
    assert!(close(r.treated.credit, 1.0), "{:?}", r.treated);
    assert!(close(r.treated.rate().unwrap(), 0.25), "{:?}", r.treated);
}

#[sqlx::test(migrations = false)]
async fn results_credit_equals_conversions_under_last_touch(pool: PgPool) {
    setup_demo(&pool).await;
    seed_decisions(&pool, "gift", false, 50, 7, 60).await;
    let r = results::site(&pool, "demo", results::ALL_TIME, Utc::now()).await.unwrap().unwrap();
    assert_eq!(r.treated.conversions, 7);
    assert!(close(r.treated.credit, 7.0), "{:?}", r.treated);
}

#[sqlx::test(migrations = false)]
async fn results_curve_counts_credit(pool: PgPool) {
    let app = setup_linear(&pool).await;
    touches(&app, &pool, "v", 2).await;
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    // A holdout that never converts, so the policy's lift is the treated rate itself.
    seed_decisions(&pool, "baseline", true, 10, 0, 60).await;
    let slot: i64 = sqlx::query_scalar("SELECT id FROM slots").fetch_one(&pool).await.unwrap();
    let curve = results::curve(&pool, slot, Utc::now() + Duration::hours(1)).await.unwrap();
    let last = serde_json::to_value(curve.last().unwrap()).unwrap();
    let p = last["policy"]["estimate"].as_f64().unwrap();
    assert!(close(p, 0.5), "{last}");
}

// ---- a late platform conversion under linear ----

const CONV_SECRET: &str = "prb_conv_test_secret";

fn sign(secret: &str, t: i64, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{t}.{body}").as_bytes());
    mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

#[sqlx::test(migrations = false)]
async fn linear_late_platform_conversion_corrects_each_miss_by_its_share(pool: PgPool) {
    let app = setup_linear(&pool).await;
    sqlx::query("UPDATE slots SET metric = 'purchase', decision_ttl_secs = 0").execute(&pool).await.unwrap();
    let admin = admin_app(&pool);
    let r = api_post(&admin, "/api/sites/demo/conversion-sources", json!({ "conversion_signing_secret": CONV_SECRET, "goals": ["purchase"] })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let mut t = vec![];
    for _ in 0..2 {
        let d: Uuid = decide(&admin, "s1", false).await.json()["decision_id"].as_str().unwrap().parse().unwrap();
        event(&admin, "s1", "exposure", Some(&d.to_string())).await;
        t.push(d);
    }
    worker::run_once(&pool, Utc::now() + Duration::hours(2)).await.unwrap();
    assert_eq!(record(&pool, t[0]).await.0, Some(0));
    let body = json!({ "id": "order-1", "subject": "s1", "event": "purchase", "occurred_at": Utc::now().to_rfc3339() }).to_string();
    let ts = Utc::now().timestamp();
    send(&app, Request::post("/webhook/conversion/demo").header("pratibimba-signature", format!("t={ts},v1={}", sign(CONV_SECRET, ts, &body)))
        .header("content-type", "application/json").body(Body::from(body)).unwrap()).await;
    let stats = worker::run_once(&pool, Utc::now() + Duration::hours(2)).await.unwrap();
    assert_eq!(stats.corrected, 2, "{stats:?}");
    assert_eq!(record(&pool, t[0]).await, (Some(1), Some(0.5)));
    assert_eq!(record(&pool, t[1]).await, (Some(1), Some(0.5)));
}

// ---- B1b: insights and explain sum credit too ----
//
// Each pair: a linear slot whose numbers must be the shares, and the same shape under last touch
// whose output must not change (whole counts stay JSON integers, explain adds nothing).

use pratibimba::insights::{self, rollup};

async fn roll(pool: &PgPool) {
    rollup::run(pool, Utc::now() + Duration::hours(1)).await.unwrap();
}

/// The heatmap's conversions over every cell, and whether each one is a JSON integer.
async fn heatmap_conversions(pool: &PgPool) -> (f64, bool) {
    let h = api_get(&admin_app(pool), "/api/sites/demo/insights/heatmap?axis=utc").await.json();
    let cells = h["cells"].as_array().unwrap().clone();
    (cells.iter().map(|c| c["conversions"].as_f64().unwrap()).sum(), cells.iter().all(|c| c["conversions"].is_i64()))
}

#[sqlx::test(migrations = false)]
async fn heatmap_sums_linear_credit_like_results(pool: PgPool) {
    let app = setup_linear(&pool).await;
    touches(&app, &pool, "v", 4).await;
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    roll(&pool).await;
    let (c, _) = heatmap_conversions(&pool).await;
    let r = results::site(&pool, "demo", results::ALL_TIME, Utc::now() + Duration::hours(1)).await.unwrap().unwrap();
    assert!(close(c, 1.0), "{c}");
    assert!(close(c, r.treated.credit), "{c} vs {:?}", r.treated);
}

#[sqlx::test(migrations = false)]
async fn heatmap_under_last_touch_counts_whole_conversions(pool: PgPool) {
    let app = setup_with(&pool, &file(None, None)).await;
    touches(&app, &pool, "v", 4).await;
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    roll(&pool).await;
    assert_eq!(heatmap_conversions(&pool).await, (1.0, true));
}

/// An active segment everyone in these fixtures matches, made before the rollup runs.
async fn everyone_segment(pool: &PgPool) {
    let r = api_post(&admin_app(pool), "/api/sites/demo/segments", json!({ "predicate": { "from_ad": false }, "actor": "t", "name": "All" })).await;
    assert_eq!(r.status, axum::http::StatusCode::CREATED, "{}", r.text);
}

async fn segment_28_days(pool: &PgPool) -> Value {
    let segs = api_get(&admin_app(pool), "/api/sites/demo/segments").await.json();
    segs["segments"].as_array().unwrap().iter().find(|s| s["name"] == "All").unwrap()["last_28_days"].clone()
}

#[sqlx::test(migrations = false)]
async fn segment_totals_sum_linear_credit(pool: PgPool) {
    let app = setup_linear(&pool).await;
    everyone_segment(&pool).await;
    touches(&app, &pool, "v", 4).await;
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    roll(&pool).await;
    let t = segment_28_days(&pool).await;
    assert!(close(t["conversions"].as_f64().unwrap(), 1.0), "{t}");
}

#[sqlx::test(migrations = false)]
async fn segment_totals_under_last_touch_count_whole_conversions(pool: PgPool) {
    let app = setup_with(&pool, &file(None, None)).await;
    everyone_segment(&pool).await;
    touches(&app, &pool, "v", 4).await;
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    roll(&pool).await;
    let t = segment_28_days(&pool).await;
    assert_eq!(t["conversions"].as_i64(), Some(1), "{t}");
}

/// 400 settled mobile exposures, 200 converted with `credit` each; 400 desktop with 4 whole conversions.
async fn seed_mobile_pattern(pool: &PgPool, credit: Option<f64>) {
    for (mobile, n, conv, cr) in [(1, 400, 200, credit), (0, 400, 4, None)] {
        sqlx::query(
            "INSERT INTO decisions (id, site_id, slot_id, variant_id, subject, pool, features, scores, policy, policy_version, seed,
                                    created_at, exposed_at, outcome, outcome_at, settled_at, credit)
             SELECT gen_random_uuid(), 'demo', v.slot_id, v.id, 's' || $1 || '-' || i, '{}',
                    jsonb_build_object('bias', 1, 'returning', 0, 'mobile', $1::int, 'from_ad', 0), '{}', 'lints', 0, 0,
                    now() - interval '3 hours', now() - interval '3 hours', CASE WHEN i <= $3 THEN 1 ELSE 0 END,
                    now() - interval '2 hours', now() - interval '2 hours', CASE WHEN i <= $3 THEN $4::float8 END
               FROM variants v, generate_series(1, $2::int) i WHERE v.key = 'baseline'",
        )
        .bind(mobile)
        .bind(n)
        .bind(conv)
        .bind(cr)
        .execute(pool)
        .await
        .unwrap();
    }
    insights::nightly(pool, Utc::now() + Duration::hours(1)).await.unwrap();
}

async fn mobile_evidence(pool: &PgPool) -> Value {
    let rows: Vec<(Value, Value)> = sqlx::query_as("SELECT predicate, evidence FROM segments WHERE origin = 'discovered'").fetch_all(pool).await.unwrap();
    rows.into_iter().find(|(p, _)| *p == json!({ "mobile": true })).map(|(_, e)| e).expect("the mobile draft")["converts_more"]["inside"].clone()
}

#[sqlx::test(migrations = false)]
async fn discovery_evidence_sums_linear_credit(pool: PgPool) {
    let _ = setup_linear(&pool).await;
    seed_mobile_pattern(&pool, Some(0.25)).await;
    let inside = mobile_evidence(&pool).await;
    assert!(close(inside["conversions"].as_f64().unwrap(), 50.0), "{inside}");
    assert!(close(inside["rate"].as_f64().unwrap(), 0.125), "{inside}");
}

#[sqlx::test(migrations = false)]
async fn discovery_evidence_under_last_touch_counts_whole_conversions(pool: PgPool) {
    let _ = setup_with(&pool, &file(None, None)).await;
    seed_mobile_pattern(&pool, Some(1.0)).await;
    let inside = mobile_evidence(&pool).await;
    assert_eq!(inside["conversions"].as_i64(), Some(200), "{inside}");
}

#[sqlx::test(migrations = false)]
async fn explain_states_a_linear_share(pool: PgPool) {
    let app = setup_linear(&pool).await;
    let t = touches(&app, &pool, "v", 2).await;
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    let d = serde_json::to_value(pratibimba::explain::detail(&pool, t[0], Utc::now()).await.unwrap().unwrap()).unwrap();
    assert_eq!(d["credit"].as_f64(), Some(0.5), "{d}");
    let s = d["sentence"].as_str().unwrap();
    assert!(s.contains("linear attribution gave this decision 0.5 of that conversion, and the model learned from that share."), "{s}");
}

#[sqlx::test(migrations = false)]
async fn explain_under_last_touch_is_unchanged(pool: PgPool) {
    let app = setup_with(&pool, &file(None, None)).await;
    let t = touches(&app, &pool, "v", 1).await;
    event(&app, "v", "click", None).await;
    worker::run_once(&pool, Utc::now()).await.unwrap();
    let d = serde_json::to_value(pratibimba::explain::detail(&pool, t[0], Utc::now()).await.unwrap().unwrap()).unwrap();
    assert!(d.get("credit").is_none(), "{d}");
    let s = d["sentence"].as_str().unwrap();
    assert!(s.contains("later, and the model learned from it."), "{s}");
}
