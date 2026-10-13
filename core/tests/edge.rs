//! EC1, the edge snapshot (edge-cache.md): Core pushes a versioned per-site snapshot of what
//! `/decide` reads to an edge store whenever that changes, and does nothing when no store is
//! configured. One test per trigger; each fails if that trigger's publish call is removed.

mod common;
use common::*;
use pratibimba::edge::{Edge, MemoryStore, Store};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;

fn edge_app(pool: &PgPool) -> (axum::Router, Arc<MemoryStore>) {
    let store = Arc::new(MemoryStore::default());
    let state = pratibimba::http::AppState::configured_with_edge(pool.clone(), Some(7), Some(TOKEN.into()), None, Edge::with_store(store.clone()));
    (pratibimba::http::router(state, &[]), store)
}

/// The demo site, published once, so each test starts from version 1 and sees its own push.
async fn published(pool: &PgPool) -> (axum::Router, Arc<MemoryStore>) {
    setup_demo(pool).await;
    let (app, store) = edge_app(pool);
    let p = pratibimba::edge::publish(pool, &Edge::with_store(store.clone()), "demo").await.unwrap();
    assert_eq!(p.map(|p| p.version), Some(1));
    (app, store)
}

/// A stored f32 (shares, holdout) as decide reads it, `f64::from`, compared at f32 precision.
fn f32s(v: &Value) -> f32 {
    v.as_f64().unwrap() as f32
}

fn version(s: &Value) -> i64 {
    s["version"].as_i64().unwrap()
}

fn arm_keys(s: &Value, slot: &str) -> Vec<String> {
    s["slots"][slot]["arms"].as_array().unwrap().iter().map(|a| a["key"].as_str().unwrap().to_string()).collect()
}

/// The push the trigger made: version 2, exactly one more put than before.
fn pushed(store: &MemoryStore) -> Value {
    assert_eq!(store.count("demo"), 2, "the trigger made no push (or more than one)");
    let s = store.last("demo").unwrap();
    assert_eq!(version(&s), 2);
    s
}

async fn ok(r: Resp) -> Value {
    assert!(r.status.is_success(), "{} {}", r.status, r.text);
    r.json()
}

// ---- each trigger publishes ----

#[sqlx::test(migrations = false)]
async fn approve_publishes(pool: PgPool) {
    let (app, store) = published(&pool).await;
    insert_variant(&pool, "spring", "draft").await;
    let id = variant_id(&pool, "spring").await;
    ok(api_post(&app, &format!("/api/variants/{id}/approve"), json!({ "approver": "owner", "traffic_share": 0.3 })).await).await;
    let s = pushed(&store);
    assert_eq!(arm_keys(&s, "hero"), ["baseline", "gift", "spring"]);
    let spring = &s["slots"]["hero"]["arms"][2];
    assert_eq!(f32s(&spring["traffic_share"]), 0.3, "decide reads the share as f64::from(f32)");
    assert_eq!(spring["payload"], json!("<p>x</p>"));
}

#[sqlx::test(migrations = false)]
async fn retire_publishes(pool: PgPool) {
    let (app, store) = published(&pool).await;
    let gift = variant_id(&pool, "gift").await;
    let req: i64 = sqlx::query_scalar("INSERT INTO retire_requests (variant_id, agent, reason) VALUES ($1, 'agent', 'loses') RETURNING id")
        .bind(gift)
        .fetch_one(&pool)
        .await
        .unwrap();
    ok(api_post(&app, &format!("/api/retire-requests/{req}/confirm"), json!({ "actor": "owner" })).await).await;
    assert_eq!(arm_keys(&pushed(&store), "hero"), ["baseline"]);
}

#[sqlx::test(migrations = false)]
async fn kill_switch_publishes(pool: PgPool) {
    let (app, store) = published(&pool).await;
    ok(api_post(&app, "/api/sites/demo/pause", json!({ "actor": "owner", "paused": true })).await).await;
    assert_eq!(pushed(&store)["settings"]["paused"], json!(true));
}

#[sqlx::test(migrations = false)]
async fn privacy_setting_publishes(pool: PgPool) {
    let (app, store) = published(&pool).await;
    ok(api_post(&app, "/api/sites/demo/privacy", json!({ "privacy_mode": "cookieless" })).await).await;
    assert_eq!(pushed(&store)["settings"]["privacy_mode"], json!("cookieless"));
}

async fn audience(app: &axum::Router) -> i64 {
    let seg = ok(api_post(app, "/api/sites/demo/segments", json!({ "predicate": { "returning": true }, "actor": "owner" })).await).await;
    seg["id"].as_i64().unwrap()
}

#[sqlx::test(migrations = false)]
async fn variant_audience_publishes(pool: PgPool) {
    setup_demo(&pool).await;
    let (app, store) = edge_app(&pool);
    let seg = audience(&app).await;
    ok(api_post(&app, &format!("/api/segments/{seg}/promote"), json!({ "actor": "owner" })).await).await;
    let before = store.count("demo");
    let gift = variant_id(&pool, "gift").await;
    ok(api_post(&app, &format!("/api/variants/{gift}/audience"), json!({ "audience": seg, "actor": "owner" })).await).await;
    assert_eq!(store.count("demo"), before + 1, "setting an audience made no push");
    let gates = &store.last("demo").unwrap()["slots"]["hero"]["gates"];
    assert_eq!(gates[0]["variant"], json!("gift"));
    assert_eq!(gates[0]["status"], json!("audience"));
}

#[sqlx::test(migrations = false)]
async fn audience_archive_publishes(pool: PgPool) {
    setup_demo(&pool).await;
    let (app, store) = edge_app(&pool);
    let seg = audience(&app).await;
    ok(api_post(&app, &format!("/api/segments/{seg}/promote"), json!({ "actor": "owner" })).await).await;
    let gift = variant_id(&pool, "gift").await;
    ok(api_post(&app, &format!("/api/variants/{gift}/audience"), json!({ "audience": seg, "actor": "owner" })).await).await;
    let before = store.count("demo");
    ok(api_post(&app, &format!("/api/segments/{seg}/archive"), json!({ "actor": "owner" })).await).await;
    assert_eq!(store.count("demo"), before + 1, "archiving a gating audience made no push");
    assert_eq!(store.last("demo").unwrap()["slots"]["hero"]["gates"][0]["status"], json!("archived"));
}

#[sqlx::test(migrations = false)]
async fn audience_promote_publishes(pool: PgPool) {
    setup_demo(&pool).await;
    let (app, store) = edge_app(&pool);
    let seg = audience(&app).await;
    // A variant gated by a not-yet-promoted segment (only the database can say this): promoting
    // it changes the gate decide reads from "matches nobody" to the predicate.
    sqlx::query("UPDATE variants SET audience_id = $1 WHERE key = 'gift'").bind(seg).execute(&pool).await.unwrap();
    pratibimba::edge::publish(&pool, &Edge::with_store(store.clone()), "demo").await.unwrap();
    let before = store.count("demo");
    ok(api_post(&app, &format!("/api/segments/{seg}/promote"), json!({ "actor": "owner" })).await).await;
    assert_eq!(store.count("demo"), before + 1, "promoting a gating segment made no push");
    assert_eq!(store.last("demo").unwrap()["slots"]["hero"]["gates"][0]["status"], json!("audience"));
}

fn experiment() -> Value {
    json!({
        "site": "demo", "slot": "hero", "name": "gift vs baseline", "actor": "owner",
        "arms": [ { "variant": "baseline", "weight": 1 }, { "variant": "gift", "weight": 3 } ],
        "stops_at": (chrono::Utc::now() + chrono::Duration::days(7)).to_rfc3339(),
    })
}

#[sqlx::test(migrations = false)]
async fn experiment_start_publishes(pool: PgPool) {
    let (app, store) = published(&pool).await;
    let e = ok(api_post(&app, "/api/experiments", experiment()).await).await;
    let s = pushed(&store);
    assert_eq!(s["slots"]["hero"]["experiment"]["id"], e["id"]);
    assert_eq!(s["slots"]["hero"]["experiment"]["arms"][1]["weight"], json!(3.0));
}

#[sqlx::test(migrations = false)]
async fn experiment_stop_publishes(pool: PgPool) {
    let (app, store) = published(&pool).await;
    let e = ok(api_post(&app, "/api/experiments", experiment()).await).await;
    assert_eq!(store.count("demo"), 2);
    ok(api_post(&app, &format!("/api/experiments/{}/stop", e["id"]), json!({ "actor": "owner" })).await).await;
    assert_eq!(store.count("demo"), 3, "stopping the experiment made no push");
    assert_eq!(store.last("demo").unwrap()["slots"]["hero"]["experiment"], Value::Null);
}

#[sqlx::test(migrations = false)]
async fn model_rollback_publishes(pool: PgPool) {
    let (app, store) = published(&pool).await;
    let slot: i64 = sqlx::query_scalar("SELECT id FROM slots WHERE key = 'hero'").fetch_one(&pool).await.unwrap();
    let r = ok(api_post(&app, &format!("/api/models/{slot}/rollback"), json!({ "version": 0, "actor": "owner" })).await).await;
    let s = pushed(&store);
    assert_eq!(s["slots"]["hero"]["model"]["version"], r["version"]);
    assert_eq!(s["slots"]["hero"]["model"]["version"], json!(1));
}

#[sqlx::test(migrations = false)]
async fn plugin_install_publishes(pool: PgPool) {
    let (app, store) = published(&pool).await;
    ok(api_post(&app, "/api/plugins/attribution.linear/install", json!({ "site": "demo", "actor": "owner" })).await).await;
    assert_eq!(pushed(&store)["plugins"][0]["name"], json!("attribution.linear"));
}

#[sqlx::test(migrations = false)]
async fn plugin_uninstall_publishes(pool: PgPool) {
    let (app, store) = published(&pool).await;
    ok(api_post(&app, "/api/plugins/attribution.linear/install", json!({ "site": "demo", "actor": "owner" })).await).await;
    ok(api_post(&app, "/api/plugins/attribution.linear/uninstall", json!({ "site": "demo", "actor": "owner" })).await).await;
    assert_eq!(store.count("demo"), 3, "uninstalling made no push");
    assert_eq!(store.last("demo").unwrap()["plugins"], json!([]));
}

#[sqlx::test(migrations = false)]
async fn plugin_request_confirm_publishes(pool: PgPool) {
    let (app, store) = published(&pool).await;
    let req: i64 = sqlx::query_scalar("INSERT INTO plugin_requests (site_id, name, agent, reason) VALUES ('demo', 'attribution.linear', 'agent', 'fair credit') RETURNING id")
        .fetch_one(&pool)
        .await
        .unwrap();
    ok(api_post(&app, &format!("/api/plugin-requests/{req}/confirm"), json!({ "actor": "owner" })).await).await;
    assert_eq!(pushed(&store)["plugins"][0]["name"], json!("attribution.linear"));
}

#[sqlx::test(migrations = false)]
async fn apply_publishes(pool: PgPool) {
    let (_, store) = published(&pool).await;
    let f = file_from(json!({
        "site": "demo", "origin": "http://localhost:5173", "holdout_fraction": 0.25,
        "slots": [{ "key": "hero", "selector": "#hero", "metric": "click",
                    "variants": [ { "key": "baseline", "baseline": true }, { "key": "gift", "html": "<h1>Gift sets</h1>" } ] }]
    }));
    pratibimba::site_file::apply_edge(&pool, &f, &Edge::with_store(store.clone())).await.unwrap();
    assert_eq!(pushed(&store)["settings"]["holdout_fraction"], json!(0.25));
}

/// The worker pass publishes what the worker changes: here, a learned model version.
#[sqlx::test(migrations = false)]
async fn worker_pass_publishes_a_learned_model(pool: PgPool) {
    let (app, store) = published(&pool).await;
    set_holdout(&pool, 0.0).await;
    // A real decision, exposed and clicked, its window already closed: the pass learns it.
    let d = decide(&app, "w-1", false).await.json();
    let id = d["decision_id"].as_str().unwrap().to_string();
    event(&app, "w-1", "exposure", Some(&id)).await;
    event(&app, "w-1", "click", Some(&id)).await;
    sqlx::query("UPDATE decisions SET created_at = now() - interval '2 hours'").execute(&pool).await.unwrap();
    pratibimba::worker::run_drain_edge(&pool, &Edge::with_store(store.clone())).await.unwrap();
    let db: i64 = sqlx::query_scalar("SELECT version FROM models").fetch_one(&pool).await.unwrap();
    assert!(db > 0, "the pass learned nothing, so this test proves nothing");
    let s = store.last("demo").unwrap();
    assert!(version(&s) >= 2, "the worker pass made no push");
    assert_eq!(s["slots"]["hero"]["model"]["version"], json!(db));
}

// ---- versioning and failure ----

#[sqlx::test(migrations = false)]
async fn an_unchanged_snapshot_is_not_pushed_again(pool: PgPool) {
    let (_, store) = published(&pool).await;
    let edge = Edge::with_store(store.clone());
    assert_eq!(pratibimba::edge::publish(&pool, &edge, "demo").await.unwrap(), None);
    pratibimba::worker::run_drain_edge(&pool, &edge).await.unwrap();
    assert_eq!(store.count("demo"), 1);
}

/// A store that fails its first put.
#[derive(Default)]
struct Flaky {
    calls: std::sync::Mutex<u32>,
    inner: MemoryStore,
}

impl Store for Flaky {
    fn put<'a>(&'a self, key: &'a str, body: Vec<u8>) -> futures_util::future::BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let n = {
                let mut c = self.calls.lock().unwrap();
                *c += 1;
                *c
            };
            if n == 1 {
                anyhow::bail!("KV is down");
            }
            self.inner.put(key, body).await
        })
    }
}

#[sqlx::test(migrations = false)]
async fn a_failed_push_is_retried_by_the_next_worker_pass(pool: PgPool) {
    setup_demo(&pool).await;
    let flaky = Arc::new(Flaky::default());
    let state = pratibimba::http::AppState::configured_with_edge(pool.clone(), Some(7), Some(TOKEN.into()), None, Edge::with_store(flaky.clone()));
    let app = pratibimba::http::router(state, &[]);
    // The approval still succeeds when the push fails.
    ok(api_post(&app, "/api/sites/demo/pause", json!({ "actor": "owner", "paused": true })).await).await;
    assert_eq!(count(&pool, "SELECT count(*) FROM edge_snapshots").await, 0, "a failed push was recorded as published");
    pratibimba::worker::run_drain_edge(&pool, &Edge::with_store(flaky.clone())).await.unwrap();
    let s = flaky.inner.last("demo").expect("the worker pass did not retry");
    assert_eq!(version(&s), 1);
    assert_eq!(s["settings"]["paused"], json!(true));
}

// ---- not configured ----

#[test]
fn no_variables_means_off() {
    assert!(!Edge::from_vars(|_| None).unwrap().is_on());
}

#[test]
fn all_three_variables_turn_it_on() {
    let e = Edge::from_vars(|k| Some(format!("v-{k}"))).unwrap();
    assert!(e.is_on());
}

#[test]
fn some_variables_is_an_error_not_off() {
    let r = Edge::from_vars(|k| (k == "PRB_EDGE_KV_TOKEN").then(|| "t".to_string()));
    assert!(r.is_err(), "a token without account and namespace was accepted");
}

#[sqlx::test(migrations = false)]
async fn unconfigured_core_builds_and_writes_nothing(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    insert_variant(&pool, "spring", "draft").await;
    let id = variant_id(&pool, "spring").await;
    ok(api_post(&app, &format!("/api/variants/{id}/approve"), json!({ "approver": "owner", "traffic_share": 0.3 })).await).await;
    pratibimba::worker::run_drain(&pool).await.unwrap();
    assert_eq!(pratibimba::edge::publish(&pool, &Edge::off(), "demo").await.unwrap(), None);
    assert_eq!(count(&pool, "SELECT count(*) FROM edge_snapshots").await, 0);
}

/// The Cloudflare store speaks the KV REST API: PUT to the namespace's value URL with the token.
#[sqlx::test(migrations = false)]
async fn cloudflare_store_puts_to_the_kv_api(pool: PgPool) {
    setup_demo(&pool).await;
    type Seen = Arc<std::sync::Mutex<Vec<(String, String, String, Value)>>>;
    let seen: Seen = Default::default();
    let s2 = seen.clone();
    let mock = axum::Router::new().fallback(move |req: axum::extract::Request| {
        let seen = s2.clone();
        async move {
            let (parts, body) = req.into_parts();
            let bytes = http_body_util::BodyExt::collect(body).await.unwrap().to_bytes();
            let auth = parts.headers.get("authorization").map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
            seen.lock().unwrap().push((parts.method.to_string(), parts.uri.path().to_string(), auth, serde_json::from_slice(&bytes).unwrap_or(Value::Null)));
            axum::Json(json!({ "success": true }))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let vars = [
        ("PRB_EDGE_KV_ACCOUNT", "acct"),
        ("PRB_EDGE_KV_NAMESPACE", "ns"),
        ("PRB_EDGE_KV_TOKEN", "tok"),
        ("PRB_EDGE_KV_API", &format!("http://{addr}/client/v4")),
    ]
    .map(|(k, v)| (k.to_string(), v.to_string()));
    let edge = Edge::from_vars(|k| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())).unwrap();
    pratibimba::edge::publish(&pool, &edge, "demo").await.unwrap().expect("published");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let (method, path, auth, body) = &seen[0];
    assert_eq!(method, "PUT");
    assert_eq!(path, "/client/v4/accounts/acct/storage/kv/namespaces/ns/values/site%2Fdemo");
    assert_eq!(auth, "Bearer tok");
    assert_eq!(body["site"], json!("demo"));
    assert_eq!(body["version"], json!(1));
}

// ---- the snapshot holds what /decide reads ----

/// Every input decide reads, set to a non-default value, and the snapshot holding each one:
/// two live slots and one draft slot; approved, draft, retired and paused-site states; a share
/// cap; an audience gate; a running experiment; a learned model; cold-start counts; holdout,
/// night window, privacy mode, declared features.
async fn rich(pool: &PgPool) -> (axum::Router, Arc<MemoryStore>) {
    pratibimba::db::migrate(pool).await.unwrap();
    let f = file_from(json!({
        "site": "demo", "origin": "http://localhost:5173", "holdout_fraction": 0.2, "privacy_mode": "consent_aware",
        "features": [ { "key": "vip", "pii": false }, { "key": "email_known", "pii": true } ],
        "slots": [
            { "key": "hero", "selector": "#hero", "metric": "click", "cold_start_exposures": 50,
              "variants": [ { "key": "baseline", "baseline": true }, { "key": "gift", "html": "<h1>Gift sets</h1>" },
                            { "key": "sale", "html": "<h1>Sale</h1>" } ] },
            { "key": "promo", "selector": "#promo", "metric": "click",
              "variants": [ { "key": "base", "baseline": true }, { "key": "free-ship", "html": "<p>Free shipping</p>" } ] },
        ]
    }));
    pratibimba::site_file::apply(pool, &f).await.unwrap();
    let (app, store) = edge_app(pool);
    sqlx::query("UPDATE sites SET night_start_hour = 21, night_end_hour = 5").execute(pool).await.unwrap();
    sqlx::query("UPDATE variants SET traffic_share = 0.4 WHERE key = 'sale'").execute(pool).await.unwrap();
    insert_variant(pool, "draft-one", "draft").await;
    insert_variant(pool, "old-one", "retired").await;
    // A slot with only a draft baseline (an agent's create_slot) decides nothing.
    let draft_slot: i64 = sqlx::query_scalar("INSERT INTO slots (site_id, key, selector, metric) VALUES ('demo', 'agent-slot', '#a', 'click') RETURNING id")
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO variants (slot_id, key, is_baseline, status) VALUES ($1, 'b', true, 'draft')").bind(draft_slot).execute(pool).await.unwrap();
    // An audience on gift, an experiment on promo.
    let seg = audience(&app).await;
    ok(api_post(&app, &format!("/api/segments/{seg}/promote"), json!({ "actor": "owner" })).await).await;
    let gift = variant_id(pool, "gift").await;
    ok(api_post(&app, &format!("/api/variants/{gift}/audience"), json!({ "audience": seg, "actor": "owner" })).await).await;
    let mut e = experiment();
    e["slot"] = json!("promo");
    e["arms"] = json!([ { "variant": "base", "weight": 1 }, { "variant": "free-ship", "weight": 2 } ]);
    ok(api_post(&app, "/api/experiments", e).await).await;
    // Traffic: decisions exposed in cold start on hero, then a model version learned on promo.
    for i in 0..6 {
        let d = post(&app, "/decide", json!({ "site": "demo", "slot": "hero", "subject": format!("r-{i}"), "ctx": { "consent": true } })).await.json();
        let r = post(&app, "/e", json!({ "site": "demo", "subject": format!("r-{i}"), "consent": true,
            "events": [{ "id": uuid::Uuid::new_v4(), "name": "exposure", "slot": "hero", "decision_id": d["decision_id"] }] })).await;
        assert!(r.status.is_success(), "{}", r.text);
    }
    pratibimba::worker::run_once(pool, chrono::Utc::now()).await.unwrap();
    let promo: i64 = sqlx::query_scalar("SELECT id FROM slots WHERE key = 'promo'").fetch_one(pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let state: Value = sqlx::query_scalar("SELECT state FROM models WHERE slot_id = $1").bind(promo).fetch_one(&mut *tx).await.unwrap();
    pratibimba::worker::write_model(&mut tx, promo, &state, "test").await.unwrap();
    pratibimba::worker::write_model(&mut tx, promo, &state, "test").await.unwrap();
    tx.commit().await.unwrap();
    pratibimba::edge::publish(pool, &Edge::with_store(store.clone()), "demo").await.unwrap();
    (app, store)
}

#[sqlx::test(migrations = false)]
async fn snapshot_holds_every_input_decide_reads(pool: PgPool) {
    let (_, store) = rich(&pool).await;
    let s = store.last("demo").unwrap();
    assert_eq!(s["format"], json!(1));
    assert_eq!(s["site"], json!("demo"));
    assert!(s["generated_at"].is_string());
    let set = &s["settings"];
    assert_eq!(set["privacy_mode"], json!("consent_aware"));
    assert_eq!(f32s(&set["holdout_fraction"]), 0.2);
    assert_eq!(set["paused"], json!(false));
    assert_eq!(set["night"], json!({ "start": 21, "end": 5 }));
    // Live slots only: the agent's draft slot is not there.
    let slots: Vec<&String> = s["slots"].as_object().unwrap().keys().collect();
    assert_eq!(slots, ["hero", "promo"]);
    let hero = &s["slots"]["hero"];
    // Approved arms only, with payloads and shares.
    assert_eq!(arm_keys(&s, "hero"), ["baseline", "gift", "sale"]);
    assert_eq!(hero["arms"][0], json!({ "id": hero["arms"][0]["id"], "key": "baseline", "payload": null, "baseline": true, "traffic_share": null }));
    assert_eq!(hero["arms"][1]["payload"], json!("<h1>Gift sets</h1>"));
    assert_eq!(f32s(&hero["arms"][2]["traffic_share"]), 0.4);
    assert_eq!(hero["gates"].as_array().unwrap().len(), 1);
    assert_eq!(hero["gates"][0]["variant"], json!("gift"));
    assert_eq!(hero["gates"][0]["predicate"], json!({ "returning": true }));
    assert_eq!(hero["declared_features"], json!({ "email_known": true, "vip": false }));
    assert_eq!(hero["experiment"], Value::Null);
    assert_eq!(hero["cold_start_exposures"], json!(50));
    let served: i64 = hero["cold_start"]["served"].as_object().unwrap().values().map(|v| v.as_i64().unwrap()).sum();
    assert_eq!(hero["cold_start"]["exposures"], json!(6 - count(&pool, "SELECT count(*) FROM decisions WHERE in_holdout").await));
    assert_eq!(served, count(&pool, "SELECT count(*) FROM decisions WHERE cold_start IS NOT NULL").await);
    assert!(served > 0, "no cold-start decision in the fixture");
    let promo = &s["slots"]["promo"];
    assert_eq!(promo["model"]["version"], json!(2));
    assert_eq!(promo["experiment"]["name"], json!("gift vs baseline"));
    assert_eq!(promo["experiment"]["arms"][1]["weight"], json!(2.0));
    // The manifest, as /manifest/demo serves it.
    let app = app(&pool);
    assert_eq!(s["manifest"], get(&app, "/manifest/demo").await.json());
}

/// What decide actually did agrees with the snapshot: the model version it used, the payload
/// of the variant it served, and the experiment it ran under.
#[sqlx::test(migrations = false)]
async fn decide_agrees_with_the_snapshot(pool: PgPool) {
    let (app, store) = rich(&pool).await;
    let s = store.last("demo").unwrap();
    for (slot, i) in [("hero", 0), ("promo", 1), ("hero", 2), ("promo", 3), ("hero", 4), ("promo", 5)] {
        let r = post(&app, "/decide", json!({ "site": "demo", "slot": slot, "subject": format!("agree-{i}"), "ctx": { "returning": true, "consent": true } })).await;
        let d = ok(r).await;
        let snap = &s["slots"][slot];
        assert_eq!(d["explanation"]["policy_version"], snap["model"]["version"], "{slot}: decide used another model version");
        let arm = snap["arms"].as_array().unwrap().iter().find(|a| a["key"] == d["variant"]).unwrap_or_else(|| panic!("{slot}: served {} which the snapshot lacks", d["variant"]));
        assert_eq!(arm["payload"], d["payload"]);
        assert_eq!(d["explanation"]["experiment"].get("id").cloned().unwrap_or(Value::Null), snap["experiment"].get("id").cloned().unwrap_or(Value::Null));
    }
}
