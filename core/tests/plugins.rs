//! C1/C3: the plugin registry. Plugins are compiled-in, named extensions of known kinds; a site
//! installs one with its config, and the install takes effect (`attribution.linear` switches the
//! site's attributor; `sink.identity_resolver` subscribes its URL to the site's events). A plugin
//! that declares itself a de-anonymiser sets a site warning while it is installed.

mod common;

use axum::{Router, body::Body, http::Request};
use chrono::Utc;
use common::*;
use pratibimba::worker;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

const SINK_URL: &str = "https://resolver.example.com/in";

fn file(slot_attribution: Option<&str>) -> pratibimba::site_file::SiteFile {
    let mut v = json!({
        "site": "demo", "origin": "http://localhost:5173",
        "slots": [{ "key": "hero", "selector": "#hero", "metric": "click",
                    "variants": [ { "key": "baseline", "baseline": true }, { "key": "gift", "html": "<h1>Gift sets</h1>" } ] }]
    });
    if let Some(a) = slot_attribution {
        v["slots"][0]["attribution"] = json!(a);
    }
    file_from(v)
}

async fn setup_with(pool: &PgPool, f: &pratibimba::site_file::SiteFile) -> Router {
    pratibimba::db::migrate(pool).await.unwrap();
    pratibimba::site_file::apply(pool, f).await.unwrap();
    set_holdout(pool, 0.0).await;
    admin_app(pool)
}

async fn setup(pool: &PgPool) -> Router {
    setup_with(pool, &file(None)).await
}

async fn install(app: &Router, name: &str, body: Value) -> Resp {
    api_post(app, &format!("/api/plugins/{name}/install"), body).await
}

async fn install_linear(app: &Router) -> Resp {
    install(app, "attribution.linear", json!({ "site": "demo", "actor": "owner" })).await
}

async fn install_sink(app: &Router) -> Resp {
    install(app, "sink.identity_resolver", json!({ "site": "demo", "actor": "owner", "config": { "url": SINK_URL } })).await
}

async fn uninstall(app: &Router, name: &str) -> Resp {
    api_post(app, &format!("/api/plugins/{name}/uninstall"), json!({ "site": "demo", "actor": "owner" })).await
}

async fn listing(app: &Router) -> Value {
    let r = api_get(app, "/api/sites/demo/plugins").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json()
}

fn plugin<'a>(l: &'a Value, name: &str) -> &'a Value {
    l["plugins"].as_array().unwrap().iter().find(|p| p["name"] == name).unwrap_or_else(|| panic!("no {name} in {l}"))
}

/// Two exposed touches for one subject a minute apart, then a click naming no decision; the
/// worker credits it. Returns each touch's credit, oldest first.
async fn two_touches_then_a_click(app: &Router, pool: &PgPool) -> Vec<Option<f64>> {
    let public = common::app(pool);
    let _ = app;
    let mut ds = vec![];
    for _ in 0..2 {
        sqlx::query("UPDATE decisions SET created_at = created_at - interval '1 minute'").execute(pool).await.unwrap();
        let d: Uuid = decide(&public, "s1", true).await.json()["decision_id"].as_str().unwrap().parse().unwrap();
        event(&public, "s1", "exposure", Some(&d.to_string())).await;
        worker::run_once(pool, Utc::now()).await.unwrap();
        ds.push(d);
    }
    event(&public, "s1", "click", None).await;
    worker::run_once(pool, Utc::now()).await.unwrap();
    let mut out = vec![];
    for d in ds {
        out.push(sqlx::query_scalar("SELECT credit FROM decisions WHERE id = $1").bind(d).fetch_one(pool).await.unwrap());
    }
    out
}

// ---- the catalogue ----

#[sqlx::test(migrations = false)]
async fn the_catalogue_lists_every_plugin_uninstalled_on_a_fresh_site(pool: PgPool) {
    let app = setup(&pool).await;
    let l = listing(&app).await;
    let names: Vec<&str> = l["plugins"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["attribution.linear", "sink.identity_resolver"]);
    assert!(l["plugins"].as_array().unwrap().iter().all(|p| p["installed"].is_null()), "{l}");
}

#[sqlx::test(migrations = false)]
async fn each_plugin_names_its_kind(pool: PgPool) {
    let app = setup(&pool).await;
    let l = listing(&app).await;
    assert_eq!(plugin(&l, "attribution.linear")["kind"], "attributor");
    assert_eq!(plugin(&l, "sink.identity_resolver")["kind"], "event_sink");
}

#[sqlx::test(migrations = false)]
async fn listing_an_unknown_sites_plugins_is_not_found(pool: PgPool) {
    let app = setup(&pool).await;
    assert_eq!(api_get(&app, "/api/sites/nope/plugins").await.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = false)]
async fn the_plugin_api_needs_the_admin_token(pool: PgPool) {
    let app = setup(&pool).await;
    let r = send(&app, Request::get("/api/sites/demo/plugins").body(Body::empty()).unwrap()).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

// ---- install ----

#[sqlx::test(migrations = false)]
async fn installing_records_the_plugin_its_config_and_who(pool: PgPool) {
    let app = setup(&pool).await;
    let r = install_sink(&app).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    let l = listing(&app).await;
    let p = &plugin(&l, "sink.identity_resolver")["installed"];
    assert_eq!(p["config"], json!({ "url": SINK_URL }), "{l}");
    assert_eq!(p["by"], "owner");
}

#[sqlx::test(migrations = false)]
async fn installing_is_per_site(pool: PgPool) {
    let app = setup(&pool).await;
    sqlx::query("INSERT INTO sites (id, origin) VALUES ('other', 'http://other.test')").execute(&pool).await.unwrap();
    assert_eq!(install_linear(&app).await.status, StatusCode::CREATED);
    let other = api_get(&app, "/api/sites/other/plugins").await.json();
    assert!(plugin(&other, "attribution.linear")["installed"].is_null(), "{other}");
}

#[sqlx::test(migrations = false)]
async fn installing_an_unknown_plugin_is_not_found(pool: PgPool) {
    let app = setup(&pool).await;
    let r = install(&app, "attribution.shapley", json!({ "site": "demo", "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn installing_on_an_unknown_site_is_not_found(pool: PgPool) {
    let app = setup(&pool).await;
    let r = install(&app, "attribution.linear", json!({ "site": "nope", "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn installing_needs_an_actor(pool: PgPool) {
    let app = setup(&pool).await;
    let r = install(&app, "attribution.linear", json!({ "site": "demo", "actor": "  " })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn installing_twice_is_a_conflict(pool: PgPool) {
    let app = setup(&pool).await;
    assert_eq!(install_linear(&app).await.status, StatusCode::CREATED);
    assert_eq!(install_linear(&app).await.status, StatusCode::CONFLICT);
}

#[sqlx::test(migrations = false)]
async fn config_a_plugin_does_not_take_is_refused(pool: PgPool) {
    let app = setup(&pool).await;
    let r = install(&app, "attribution.linear", json!({ "site": "demo", "actor": "owner", "config": { "window": 3 } })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn the_sink_refuses_a_url_that_is_not_http(pool: PgPool) {
    let app = setup(&pool).await;
    let r = install(&app, "sink.identity_resolver", json!({ "site": "demo", "actor": "owner", "config": { "url": "ftp://x.test/in" } })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn the_sink_refuses_a_missing_url(pool: PgPool) {
    let app = setup(&pool).await;
    let r = install(&app, "sink.identity_resolver", json!({ "site": "demo", "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
}

// ---- install takes effect ----

#[sqlx::test(migrations = false)]
async fn without_the_plugin_the_site_credits_the_last_touch(pool: PgPool) {
    let app = setup(&pool).await;
    assert_eq!(two_touches_then_a_click(&app, &pool).await, vec![None, Some(1.0)]);
}

#[sqlx::test(migrations = false)]
async fn installing_linear_switches_the_sites_attributor(pool: PgPool) {
    let app = setup(&pool).await;
    assert_eq!(install_linear(&app).await.status, StatusCode::CREATED);
    assert_eq!(two_touches_then_a_click(&app, &pool).await, vec![Some(0.5), Some(0.5)]);
}

#[sqlx::test(migrations = false)]
async fn a_slots_own_attributor_still_beats_the_installed_one(pool: PgPool) {
    let app = setup_with(&pool, &file(Some("last_touch"))).await;
    assert_eq!(install_linear(&app).await.status, StatusCode::CREATED);
    assert_eq!(two_touches_then_a_click(&app, &pool).await, vec![None, Some(1.0)]);
}

#[sqlx::test(migrations = false)]
async fn uninstalling_linear_restores_the_sites_own_attributor(pool: PgPool) {
    let app = setup(&pool).await;
    assert_eq!(install_linear(&app).await.status, StatusCode::CREATED);
    assert_eq!(uninstall(&app, "attribution.linear").await.status, StatusCode::OK);
    assert_eq!(two_touches_then_a_click(&app, &pool).await, vec![None, Some(1.0)]);
}

#[sqlx::test(migrations = false)]
async fn the_sink_subscribes_its_url_to_exposures_and_conversions(pool: PgPool) {
    let app = setup(&pool).await;
    let r = install_sink(&app).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    let (url, events): (String, Vec<String>) =
        sqlx::query_as("SELECT url, events FROM webhook_subscriptions WHERE site_id = 'demo'").fetch_one(&pool).await.unwrap();
    assert_eq!((url.as_str(), events), (SINK_URL, vec!["exposure".to_string(), "conversion".to_string()]));
    assert!(r.json()["secret"].as_str().is_some_and(|s| !s.is_empty()), "the signing secret is shown once: {}", r.text);
}

#[sqlx::test(migrations = false)]
async fn uninstalling_the_sink_removes_its_subscription(pool: PgPool) {
    let app = setup(&pool).await;
    assert_eq!(install_sink(&app).await.status, StatusCode::CREATED);
    assert_eq!(uninstall(&app, "sink.identity_resolver").await.status, StatusCode::OK);
    assert_eq!(count(&pool, "SELECT count(*) FROM webhook_subscriptions").await, 0);
}

#[sqlx::test(migrations = false)]
async fn uninstalling_forgets_the_install(pool: PgPool) {
    let app = setup(&pool).await;
    assert_eq!(install_linear(&app).await.status, StatusCode::CREATED);
    assert_eq!(uninstall(&app, "attribution.linear").await.status, StatusCode::OK);
    assert!(plugin(&listing(&app).await, "attribution.linear")["installed"].is_null());
}

#[sqlx::test(migrations = false)]
async fn uninstalling_a_plugin_that_is_not_installed_is_not_found(pool: PgPool) {
    let app = setup(&pool).await;
    assert_eq!(uninstall(&app, "attribution.linear").await.status, StatusCode::NOT_FOUND);
}

// ---- C3: the de-anonymiser flag and the site warning ----

#[sqlx::test(migrations = false)]
async fn every_plugin_declares_whether_it_deanonymises(pool: PgPool) {
    let app = setup(&pool).await;
    let l = listing(&app).await;
    assert_eq!(plugin(&l, "attribution.linear")["deanonymiser"], json!(false));
    assert_eq!(plugin(&l, "sink.identity_resolver")["deanonymiser"], json!(true));
}

#[sqlx::test(migrations = false)]
async fn a_fresh_site_has_no_warning(pool: PgPool) {
    let app = setup(&pool).await;
    assert_eq!(listing(&app).await["warnings"], json!([]));
}

#[sqlx::test(migrations = false)]
async fn installing_a_deanonymiser_sets_a_site_warning(pool: PgPool) {
    let app = setup(&pool).await;
    assert_eq!(install_sink(&app).await.status, StatusCode::CREATED);
    let w = listing(&app).await["warnings"].clone();
    assert_eq!(w.as_array().unwrap().len(), 1, "{w}");
    assert!(w[0].as_str().unwrap().contains("sink.identity_resolver"), "{w}");
}

#[sqlx::test(migrations = false)]
async fn installing_a_plugin_that_is_not_a_deanonymiser_sets_no_warning(pool: PgPool) {
    let app = setup(&pool).await;
    assert_eq!(install_linear(&app).await.status, StatusCode::CREATED);
    assert_eq!(listing(&app).await["warnings"], json!([]));
}

#[sqlx::test(migrations = false)]
async fn the_install_answer_carries_the_warning(pool: PgPool) {
    let app = setup(&pool).await;
    let r = install_sink(&app).await.json();
    assert!(r["warning"].as_str().is_some_and(|w| w.contains("de-anonymiser")), "{r}");
}

#[sqlx::test(migrations = false)]
async fn the_overview_carries_the_site_warnings(pool: PgPool) {
    let app = setup(&pool).await;
    assert_eq!(install_sink(&app).await.status, StatusCode::CREATED);
    let o = api_get(&app, "/api/sites/demo/overview").await.json();
    assert_eq!(o["warnings"], listing(&app).await["warnings"], "{o}");
    assert_eq!(o["warnings"].as_array().unwrap().len(), 1, "{o}");
}

#[sqlx::test(migrations = false)]
async fn the_overview_has_no_warning_without_a_deanonymiser(pool: PgPool) {
    let app = setup(&pool).await;
    let o = api_get(&app, "/api/sites/demo/overview").await.json();
    assert_eq!(o["warnings"], json!([]), "{o}");
}

#[sqlx::test(migrations = false)]
async fn uninstalling_the_deanonymiser_clears_the_warning(pool: PgPool) {
    let app = setup(&pool).await;
    assert_eq!(install_sink(&app).await.status, StatusCode::CREATED);
    assert_eq!(uninstall(&app, "sink.identity_resolver").await.status, StatusCode::OK);
    assert_eq!(listing(&app).await["warnings"], json!([]));
}
