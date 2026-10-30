//! S13: the Preview room's backend. `/preview` shows every version a visitor is eligible for
//! and writes nothing; its token is signed and short-lived; production decide is unchanged.

mod common;
use common::*;
use pratibimba::http::preview::{self, Claims};
use serde_json::{Value, json};
use sqlx::PgPool;

/// The demo site, no holdout, `gift` gated to returning visitors, plus a draft `spring`.
async fn fixture(pool: &PgPool) -> axum::Router {
    setup_demo(pool).await;
    set_holdout(pool, 0.0).await;
    let admin = admin_app(pool);
    let r = api_post(&admin, "/api/sites/demo/segments", json!({ "predicate": { "returning": true }, "actor": "owner", "name": "Returning visitors" })).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    let aud = r.json()["id"].as_i64().unwrap();
    assert_eq!(api_post(&admin, &format!("/api/segments/{aud}/promote"), json!({ "actor": "owner" })).await.status, StatusCode::OK);
    let gift = variant_id(pool, "gift").await;
    let r = api_post(&admin, &format!("/api/variants/{gift}/audience"), json!({ "audience": aud, "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    insert_variant(pool, "spring", "draft").await;
    admin
}

fn token_for(site: &str, exp_in: i64) -> String {
    preview::mint(TOKEN, &Claims { site: site.into(), exp: chrono::Utc::now().timestamp() + exp_in, parent: Some("http://dash.test".into()) })
}

async fn look(app: &axum::Router, ctx: Value) -> Value {
    let r = post(app, "/preview", json!({ "token": token_for("demo", 600), "ctx": ctx })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json()
}

fn keys(p: &Value, slot: &str) -> Vec<String> {
    let s = p["slots"].as_array().unwrap().iter().find(|s| s["slot"] == slot).unwrap_or_else(|| panic!("no slot {slot}: {p}"));
    s["versions"].as_array().unwrap().iter().map(|v| v["key"].as_str().unwrap().to_string()).collect()
}

/// Every table a decide, an event or a learner touches, as text, so any write shows.
async fn footprint(pool: &PgPool) -> Vec<String> {
    let mut out = vec![];
    for sql in [
        "SELECT count(*)::text FROM decisions",
        "SELECT count(*)::text FROM events",
        "SELECT count(*)::text FROM pending_events",
        "SELECT count(*)::text FROM webhook_deliveries",
        "SELECT count(*)::text FROM approvals",
        "SELECT COALESCE(string_agg(slot_id || ':' || version || ':' || md5(state::text), ',' ORDER BY slot_id), '') FROM models",
        "SELECT string_agg(id || ':' || status, ',' ORDER BY id) FROM variants",
        "SELECT count(*)::text FROM privacy_salts",
    ] {
        out.push(sqlx::query_scalar::<_, String>(sql).fetch_one(pool).await.unwrap());
    }
    out
}

#[sqlx::test(migrations = false)]
async fn preview_writes_no_decision_exposure_or_model_row(pool: PgPool) {
    let admin = fixture(&pool).await;
    // A webhook that listens for decisions: a preview that decided would also queue a delivery.
    let r = api_post(&admin, "/api/sites/demo/webhooks", json!({ "url": "https://hooks.test/x", "events": ["decision"] })).await;
    assert!(r.status.is_success(), "{}", r.text);
    // Some real traffic first, so the counts are not all zero.
    let app = admin_app(&pool);
    for i in 0..5 {
        assert_eq!(decide(&app, &format!("real{i}"), i % 2 == 0).await.status, StatusCode::OK);
    }
    let before = footprint(&pool).await;
    assert_ne!(before[0], "0", "the fixture made real decisions");
    for ctx in [json!({}), json!({ "returning": true }), json!({ "mobile": true, "from_ad": true, "tz_offset": 330 })] {
        for _ in 0..5 {
            look(&app, ctx.clone()).await;
        }
    }
    assert_eq!(footprint(&pool).await, before, "a preview wrote something");
}

#[sqlx::test(migrations = false)]
async fn a_gated_variant_is_left_out_for_a_visitor_outside_its_audience(pool: PgPool) {
    let _ = fixture(&pool).await;
    let app = admin_app(&pool);
    let first = look(&app, json!({ "returning": false })).await;
    assert_eq!(keys(&first, "hero"), ["baseline", "spring"], "{first}");
    let hero = &first["slots"][0];
    assert_eq!(hero["kept_out"], json!([{ "key": "gift", "why": "only for Returning visitors" }]), "{hero}");
    let back = look(&app, json!({ "returning": true })).await;
    // The version made for this visitor comes first, then the page's own, then drafts.
    assert_eq!(keys(&back, "hero"), ["gift", "baseline", "spring"], "{back}");
    assert_eq!(back["slots"][0]["versions"][0]["why"], "made for Returning visitors");
    assert_eq!(back["slots"][0]["versions"][2]["status"], "draft");
    assert_eq!(back["parent"], "http://dash.test");
}

#[sqlx::test(migrations = false)]
async fn a_bad_expired_or_foreign_token_is_refused(pool: PgPool) {
    let _ = fixture(&pool).await;
    let app = admin_app(&pool);
    let expired = token_for("demo", -1);
    let forged = preview::mint("some-other-server", &Claims { site: "demo".into(), exp: chrono::Utc::now().timestamp() + 600, parent: None });
    // A valid token's body swapped for another site's, signature kept.
    let good = token_for("demo", 600);
    let other_body = token_for("shop", 600).split_once('.').unwrap().0.to_string();
    let spliced = format!("{other_body}.{}", good.split_once('.').unwrap().1);
    for t in [expired, forged, spliced, "nonsense".into(), String::new()] {
        let r = post(&app, "/preview", json!({ "token": t, "ctx": {} })).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "token {t:?} was accepted: {}", r.text);
    }
    // And with the admin API off there is no preview at all.
    let off = common::app(&pool);
    assert_eq!(post(&off, "/preview", json!({ "token": good, "ctx": {} })).await.status, StatusCode::UNAUTHORIZED);
}

/// Production decide draws the same seeds whether or not previews ran in between: a preview
/// takes nothing from the decision stream and leaves no sticky decision behind.
#[sqlx::test(migrations = false)]
async fn production_decide_is_the_same_with_previews_in_between(pool: PgPool) {
    let _ = fixture(&pool).await;
    let run = |with_preview: bool, prefix: &'static str| {
        let pool = pool.clone();
        async move {
            let app = admin_app(&pool);
            let mut seen = vec![];
            for i in 0..30 {
                if with_preview {
                    look(&app, json!({ "returning": true })).await;
                }
                let d = decide(&app, &format!("{prefix}{i}"), true).await;
                assert_eq!(d.status, StatusCode::OK, "{}", d.text);
                let j = d.json();
                seen.push((j["variant"].as_str().unwrap().to_string(), j["explanation"]["seed"].as_str().unwrap().to_string()));
            }
            seen
        }
    };
    let plain = run(false, "a").await;
    let previewed = run(true, "b").await;
    assert_eq!(plain, previewed);
    assert!(plain.iter().any(|v| v.0 == "gift") && plain.iter().any(|v| v.0 == "baseline"), "both arms drawn: {plain:?}");
}

#[sqlx::test(migrations = false)]
async fn the_room_mints_a_token_for_the_sites_preview_page(pool: PgPool) {
    let _ = fixture(&pool).await;
    let admin = admin_app(&pool);
    // Without the admin token: nothing.
    let anon = post(&admin, "/api/sites/demo/preview-token", json!({})).await;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED);
    let r = api_post(&admin, "/api/sites/demo/preview-token", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let j = r.json();
    assert_eq!(j["page"], "http://localhost:5173/");
    let url = j["url"].as_str().unwrap();
    assert!(url.starts_with("http://localhost:5173/?prb_preview="), "{url}");
    assert!(preview::verify(TOKEN, j["token"].as_str().unwrap(), chrono::Utc::now().timestamp()).is_some_and(|c| c.site == "demo"));
    // A site file's preview_url is the page the room opens.
    let mut f = demo_file();
    f.preview_url = Some("http://localhost:5173/shop/?x=1".into());
    pratibimba::site_file::apply(&pool, &f).await.unwrap();
    let j = api_post(&admin, "/api/sites/demo/preview-token", json!({})).await.json();
    assert!(j["url"].as_str().unwrap().starts_with("http://localhost:5173/shop/?x=1&prb_preview="), "{j}");
    f.preview_url = Some("javascript:alert(1)".into());
    assert!(pratibimba::site_file::apply(&pool, &f).await.is_err());
}

#[sqlx::test(migrations = false)]
async fn reject_in_the_room_retires_a_live_variant_and_logs_who(pool: PgPool) {
    let admin = fixture(&pool).await;
    let gift = variant_id(&pool, "gift").await;
    let base = variant_id(&pool, "baseline").await;
    let r = api_post(&admin, &format!("/api/variants/{base}/retire"), json!({ "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "the page's own version stays: {}", r.text);
    let r = api_post(&admin, &format!("/api/variants/{gift}/retire"), json!({ "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(count(&pool, &format!("SELECT count(*) FROM variants WHERE id = {gift} AND status = 'retired'")).await, 1);
    assert_eq!(count(&pool, &format!("SELECT count(*) FROM approvals WHERE variant_id = {gift} AND action = 'retire' AND actor = 'owner'")).await, 1);
    let app = admin_app(&pool);
    assert_eq!(keys(&look(&app, json!({ "returning": true })).await, "hero"), ["baseline", "spring"]);
}

// --- B4: personas -------------------------------------------------------------------------

const USUAL: &str = "Your usual {visitor.top_item.name | truncate: 30} is ready";

/// A kitchen copy slot: the page's own title and an approved `your_usual` that needs a top item.
async fn kitchen(pool: &PgPool) -> axum::Router {
    pratibimba::db::migrate(pool).await.unwrap();
    let f = json!({ "site": "demo", "origin": "http://localhost:5173", "privacy_mode": "full", "slots": [{
        "key": "hero_copy", "selector": "#hero", "kind": "copy",
        "copy": { "fields": [ { "name": "title", "max": 60, "required": true } ] },
        "variants": [
            { "key": "own", "baseline": true },
            { "key": "your_usual", "requires": ["visitor.top_item"], "copy": { "title": USUAL } }
        ] }] });
    pratibimba::site_file::apply(pool, &file_from(f)).await.unwrap();
    admin_app(pool)
}

async fn look_as(app: &axum::Router, persona: &str) -> Resp {
    post(app, "/preview", json!({ "token": token_for("demo", 600), "ctx": {}, "persona": persona })).await
}

fn version<'a>(p: &'a Value, key: &str) -> &'a Value {
    p["slots"][0]["versions"].as_array().unwrap().iter().find(|v| v["key"] == key).unwrap_or_else(|| panic!("no {key}: {p}"))
}

// Mutation: pass `None` facts in `versions` -> red.
#[sqlx::test(migrations = false)]
async fn preview_persona_fills_template(pool: PgPool) {
    let app = kitchen(&pool).await;
    let r = look_as(&app, "usual_rajma").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let v = version(&r.json(), "your_usual").clone();
    assert_eq!(v["copy"], json!({ "title": "Your usual rajma-chawal is ready" }), "{v}");
    assert_eq!(v["filled"], json!(true), "{v}");
    assert!(!r.text.contains("tpl"), "a template left Core: {}", r.text);
}

// Mutation: skip close_by_requires in `versions` -> red.
#[sqlx::test(migrations = false)]
async fn preview_first_visit_shows_fall_through(pool: PgPool) {
    let app = kitchen(&pool).await;
    let r = look_as(&app, "first_visit").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let v = version(&r.json(), "your_usual").clone();
    assert_eq!(v["closed"], json!(true), "{v}");
    assert_eq!(v["why"], "needs visitor.top_item; this visitor falls to own", "{v}");
    assert!(v["copy"].is_null(), "a closed version shows no copy: {v}");
}

// Mutation: accept an unknown persona as a first visit -> red.
#[sqlx::test(migrations = false)]
async fn preview_unknown_persona_refused(pool: PgPool) {
    let app = kitchen(&pool).await;
    let r = look_as(&app, "asha@example.com").await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    assert!(r.text.contains("unknown persona") && r.text.contains("usual_rajma"), "{}", r.text);
}

// Mutation: record a decision for a persona preview -> red (the footprint changes).
#[sqlx::test(migrations = false)]
async fn preview_writes_nothing_with_persona(pool: PgPool) {
    let app = kitchen(&pool).await;
    for i in 0..3 {
        let r = post(&app, "/decide", json!({ "site": "demo", "slot": "hero_copy", "subject": format!("real{i}") })).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    }
    let before = footprint(&pool).await;
    assert_ne!(before[0], "0");
    for p in ["first_visit", "three_orders_40d", "usual_rajma", "cart_yesterday"] {
        assert_eq!(look_as(&app, p).await.status, StatusCode::OK);
    }
    assert_eq!(footprint(&pool).await, before, "a persona preview wrote something");
}

// Mutation: answer the personas route with an empty list -> red.
#[sqlx::test(migrations = false)]
async fn personas_route_lists_keys_and_labels(pool: PgPool) {
    let app = kitchen(&pool).await;
    let r = api_get(&app, "/api/personas").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let keys: Vec<String> = r.json()["personas"].as_array().unwrap().iter().map(|p| p["key"].as_str().unwrap().to_string()).collect();
    assert_eq!(keys, ["first_visit", "three_orders_40d", "usual_rajma", "cart_yesterday"]);
    assert!(r.json()["personas"][2]["label"].as_str().is_some_and(|l| !l.is_empty()));
}
