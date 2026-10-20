//! S11: image fields on copy slots. A variant changes a picture by naming an uploaded asset (or
//! an https/root-relative URL) and alt text, never HTML; every door checks the shape, and an
//! asset must be one of the site's own uploads.

mod common;
use axum::{Router, body::Body, http::Request};
use common::*;
use pratibimba::assets::{self, Assets};
use serde_json::{Value, json};
use sqlx::PgPool;

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&[8, 6, 0, 0, 0, 1, 2, 3, 4]);
    b
}

fn banner_file(site: &str, variants: Value) -> Value {
    json!({ "site": site, "origin": "http://localhost:5173", "slots": [{
        "key": "hero", "selector": "#hero", "kind": "copy",
        "copy": { "fields": [ { "name": "headline", "max": 40 }, { "name": "banner", "type": "image", "required": true, "max": 60 } ] },
        "variants": variants }] })
}

fn with_variant(site: &str, v: Value) -> Value {
    banner_file(site, json!([{ "key": "own", "baseline": true }, v]))
}

fn app_with(pool: &PgPool, a: Assets) -> Router {
    pratibimba::http::router(
        pratibimba::http::AppState::configured_full(pool.clone(), Some(7), Some(TOKEN.into()), None, pratibimba::edge::Edge::off(), a),
        &[],
    )
}

async fn upload(app: &Router, site: &str, bytes: Vec<u8>) -> String {
    let r = send(
        app,
        Request::post(format!("/api/sites/{site}/assets")).header("authorization", format!("Bearer {TOKEN}")).body(Body::from(bytes)).unwrap(),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    r.json()["id"].as_str().unwrap().to_string()
}

async fn apply(pool: &PgPool, mut f: Value) -> Result<(), String> {
    f["privacy_mode"] = json!("full");
    f["slots"][0]["cold_start_exposures"] = json!(0);
    let file: pratibimba::site_file::SiteFile = serde_json::from_value(f).map_err(|e| e.to_string())?;
    pratibimba::site_file::apply(pool, &file).await.map_err(|e| e.to_string())
}

async fn rpc_call(app: &Router, token: &str, tool: &str, args: Value) -> Value {
    let mut a = json!({ "agent": "Claude Code", "justification": "testing image fields" });
    a.as_object_mut().unwrap().extend(args.as_object().unwrap().clone());
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": tool, "arguments": a } });
    let req = Request::post("/mcp").header("content-type", "application/json").header("authorization", format!("Bearer {token}"));
    send(app, req.body(Body::from(body.to_string())).unwrap()).await.json()["result"].clone()
}

fn tool_text(r: &Value) -> (bool, String) {
    (r["isError"] == json!(true), r["content"][0]["text"].as_str().unwrap().to_string())
}

// ---- the shape of an image value ----

#[sqlx::test(migrations = false)]
async fn a_url_image_applies_and_is_decided_as_fields(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    let img = json!({ "url": "https://cdn.example.com/autumn.webp", "alt": "Autumn coffee beans", "width": 1200, "height": 400 });
    apply(&pool, with_variant("demo", json!({ "key": "autumn", "copy": { "headline": "Autumn roast", "banner": img } }))).await.unwrap();
    set_holdout(&pool, 0.0).await;
    let app = app(&pool);
    let mut seen = false;
    for i in 0..40 {
        let d = post(&app, "/decide", json!({ "site": "demo", "slot": "hero", "subject": format!("s{i}") })).await.json();
        assert_eq!(d["payload"], Value::Null);
        if d["variant"] == "autumn" {
            assert_eq!(d["copy"]["banner"], img, "{d}");
            seen = true;
        }
    }
    assert!(seen);
}

#[sqlx::test(migrations = false)]
async fn a_bad_image_value_is_refused(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    let id = "a".repeat(64);
    let cases = [
        (json!({ "url": "https://x.example/a.png" }), "missing field `alt`"),
        (json!({ "url": "https://x.example/a.png", "alt": "  " }), "alt text is required"),
        (json!({ "url": "https://x.example/a.png", "asset": id, "alt": "x" }), "exactly one of asset or url"),
        (json!({ "alt": "x" }), "exactly one of asset or url"),
        (json!({ "url": "javascript:alert(1)", "alt": "x" }), "must be https"),
        (json!({ "url": "http://x.example/a.png", "alt": "x" }), "must be https"),
        (json!({ "url": "//evil.example/a.png", "alt": "x" }), "must be https"),
        (json!({ "url": "https://x.example/a.png\" onerror=\"alert(1)", "alt": "x" }), "must be https"),
        (json!({ "url": "data:image/svg+xml,<svg onload=alert(1)>", "alt": "x" }), "must be https"),
        (json!({ "url": "https://x.example/a.png", "alt": "<b>x</b>" }), "no HTML"),
        (json!({ "url": "https://x.example/a.png", "alt": "a".repeat(61) }), "max is 60"),
        (json!({ "url": "https://x.example/a.png", "alt": "x", "width": 0 }), "width and height"),
        (json!({ "url": "https://x.example/a.png", "alt": "x", "onerror": "alert(1)" }), "unknown field `onerror`"),
        (json!(42), "not 42"),
        (json!({ "asset": "../../etc/passwd", "alt": "x" }), "not an asset id"),
        (json!("https://x.example/a.png"), "is an image field"),
    ];
    for (img, want) in cases {
        let e = apply(&pool, with_variant("demo", json!({ "key": "bad", "copy": { "banner": img } }))).await.unwrap_err();
        assert!(e.contains(want), "{img}: wanted {want:?}, got {e}");
    }
    let e = apply(&pool, with_variant("demo", json!({ "key": "bad", "copy": { "headline": { "url": "https://x.example/a.png", "alt": "x" }, "banner": { "url": "https://x.example/a.png", "alt": "x" } } }))).await.unwrap_err();
    assert!(e.contains("is a text field"), "{e}");
    let e = apply(&pool, with_variant("demo", json!({ "key": "bad", "copy": { "headline": "Only text" } }))).await.unwrap_err();
    assert!(e.contains("\"banner\" is required"), "{e}");
    let mut f = with_variant("demo", json!({ "key": "v", "copy": { "headline": "x" } }));
    f["slots"][0]["copy"]["fields"][1]["type"] = json!("video");
    assert!(apply(&pool, f).await.unwrap_err().contains("is not text or image"));
    // A text field still needs its max.
    let mut f = with_variant("demo", json!({ "key": "v", "copy": { "banner": { "url": "/a.png", "alt": "x" } } }));
    f["slots"][0]["copy"]["fields"][0].as_object_mut().unwrap().remove("max");
    assert!(apply(&pool, f).await.unwrap_err().contains("max must be 1 to 500"));
    // A root-relative URL is fine.
    apply(&pool, with_variant("demo", json!({ "key": "rel", "copy": { "banner": { "url": "/img/a.png?v=2", "alt": "Beans" } } }))).await.unwrap();
}

// ---- assets belong to their site ----

#[sqlx::test(migrations = false)]
async fn a_site_file_may_name_only_its_own_uploads(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    apply(&pool, banner_file("demo", json!([{ "key": "own", "baseline": true }]))).await.unwrap();
    apply(&pool, banner_file("other", json!([{ "key": "own", "baseline": true }]))).await.unwrap();
    let app = app_with(&pool, Assets::memory());
    let id = upload(&app, "demo", png(10, 10)).await;
    let v = json!({ "key": "pic", "copy": { "banner": { "asset": id, "alt": "Beans" } } });
    let e = apply(&pool, with_variant("other", v.clone())).await.unwrap_err();
    assert!(e.contains("not one of site \"other\"'s uploads"), "{e}");
    let missing = json!({ "key": "pic", "copy": { "banner": { "asset": "b".repeat(64), "alt": "Beans" } } });
    assert!(apply(&pool, with_variant("demo", missing)).await.is_err());
    apply(&pool, with_variant("demo", v)).await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn an_agent_and_a_person_cannot_name_another_sites_asset(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    apply(&pool, banner_file("demo", json!([{ "key": "own", "baseline": true }]))).await.unwrap();
    apply(&pool, banner_file("other", json!([{ "key": "own", "baseline": true }]))).await.unwrap();
    let app = app_with(&pool, Assets::memory());
    let theirs = upload(&app, "demo", png(10, 10)).await;
    let mine = upload(&app, "other", png(20, 20)).await;
    let t = pratibimba::mcp::issue_token(&pool, "other", "t").await.unwrap();
    let (err, text) = tool_text(&rpc_call(&app, &t, "propose_copy", json!({ "slot": "hero", "key": "steal", "copy": { "banner": { "asset": theirs, "alt": "x" } } })).await);
    assert!(err && text.contains("not one of site \"other\"'s uploads"), "{text}");
    let r = api_post(&app, "/api/sites/other/slots/hero/variants", json!({ "key": "steal", "html": json!({ "banner": { "asset": theirs, "alt": "x" } }).to_string() })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM variants WHERE key = 'steal'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0);
    // Its own upload is fine through both doors.
    let (err, text) = tool_text(&rpc_call(&app, &t, "propose_copy", json!({ "slot": "hero", "key": "ok", "copy": { "banner": { "asset": mine, "alt": "Our beans" } } })).await);
    assert!(!err, "{text}");
    let r = api_post(&app, "/api/sites/other/slots/hero/variants", json!({ "key": "ok2", "html": json!({ "banner": { "asset": mine, "alt": "x" } }).to_string() })).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
}

// ---- the agent's whole path ----

#[sqlx::test(migrations = false)]
async fn an_agent_uploads_creates_an_image_slot_proposes_and_a_person_approves(pool: PgPool) {
    setup_demo(&pool).await;
    set_holdout(&pool, 0.0).await;
    let app = app_with(&pool, Assets::memory());
    let t = pratibimba::mcp::issue_token(&pool, "demo", "t").await.unwrap();
    use base64::Engine;
    let up = rpc_call(&app, &t, "upload_asset", json!({ "data": base64::engine::general_purpose::STANDARD.encode(png(1600, 500)), "name": "autumn.png" })).await;
    let id = up["structuredContent"]["id"].as_str().unwrap().to_string();
    let (err, text) = tool_text(&rpc_call(&app, &t, "create_slot", json!({ "key": "banner", "selector": "#banner", "kind": "copy",
        "copy": { "fields": [{ "name": "hero", "type": "image", "required": true }, { "name": "caption", "max": 40 }] } })).await);
    assert!(!err, "{text}");
    let (err, text) = tool_text(&rpc_call(&app, &t, "list_slots", json!({})).await);
    assert!(!err && text.contains("\"type\":\"image\""), "{text}");
    let (err, text) = tool_text(&rpc_call(&app, &t, "propose_copy", json!({ "slot": "banner", "key": "autumn",
        "copy": { "hero": { "asset": id, "alt": "A sack of autumn beans", "width": 1600, "height": 500 }, "caption": "New harvest" } })).await);
    assert!(!err, "{text}");
    // A person approves the slot's baseline, then the image variant.
    let approvals = api_get(&app, "/api/sites/demo/approvals").await.json();
    let card = approvals["drafts"].as_array().unwrap().iter().find(|d| d["key"] == "autumn").unwrap().clone();
    assert_eq!(card["copy"]["hero"]["asset"], json!(id), "{card}");
    assert_eq!(card["fields"][0]["type"], json!("image"));
    for d in approvals["drafts"].as_array().unwrap() {
        if d["slot"] == "banner" {
            let r = api_post(&app, &format!("/api/variants/{}/approve", d["id"]), json!({ "approver": "Ritwik", "traffic_share": 0.5 })).await;
            assert!(r.status.is_success(), "{}", r.text);
        }
    }
    let mut seen = false;
    for i in 0..40 {
        let d = post(&app, "/decide", json!({ "site": "demo", "slot": "banner", "subject": format!("v{i}") })).await.json();
        if d["variant"] == "autumn" {
            assert_eq!(d["copy"]["hero"]["asset"], json!(id));
            seen = true;
        }
    }
    assert!(seen);
    // The asset is now in use.
    assets::gc(&pool, &app_assets(), chrono::Utc::now()).await.unwrap();
    let in_use: bool = sqlx::query_scalar("SELECT unreferenced_since IS NULL FROM assets").fetch_one(&pool).await.unwrap();
    assert!(in_use);
}

fn app_assets() -> Assets {
    Assets::memory()
}

#[sqlx::test(migrations = false)]
async fn an_images_alt_text_is_held_to_the_sites_claims(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    apply(&pool, banner_file("demo", json!([{ "key": "own", "baseline": true }]))).await.unwrap();
    sqlx::query("UPDATE sites SET forbidden_claims = ARRAY['cures']").execute(&pool).await.unwrap();
    let app = app_with(&pool, Assets::memory());
    let r = api_post(&app, "/api/sites/demo/slots/hero/variants", json!({ "key": "claim", "html": json!({ "banner": { "url": "/a.png", "alt": "Coffee that cures colds" } }).to_string() })).await;
    assert_eq!(r.status, StatusCode::CREATED);
    let id = r.json()["id"].as_i64().unwrap();
    let r = api_post(&app, &format!("/api/variants/{id}/approve"), json!({ "approver": "Ritwik", "traffic_share": 0.1 })).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text);
}

#[test]
fn image_urls_are_https_or_root_relative() {
    use pratibimba::copy::check_url;
    for ok in ["https://cdn.example.com/a.png", "/a.png", "/img/a.png?v=1&w=2"] {
        assert!(check_url(ok).is_ok(), "{ok}");
    }
    for bad in ["http://a/b", "//a/b", "https:///a", "ftp://a", "javascript:x", "a.png", "/a b.png", "/a'.png", "https://a/<x>", ""] {
        assert!(check_url(bad).is_err(), "{bad}");
    }
}
