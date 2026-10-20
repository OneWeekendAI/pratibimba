//! S11: the asset store. Uploads are judged by their bytes, capped, content-addressed per site,
//! served immutable and unable to run script; variants on one site cannot name another site's
//! assets; the worker's garbage collection removes what nothing names after a grace period.

mod common;
use axum::{Router, body::Body, http::Request};
use chrono::{Duration, Utc};
use common::*;
use pratibimba::assets::{self, Assets};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;

/// A minimal PNG: signature, then an IHDR chunk with the given size. Enough for any sniffer.
fn png(w: u32, h: u32, pad: usize) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
    b.extend_from_slice(&13u32.to_be_bytes());
    b.extend_from_slice(b"IHDR");
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    b.resize(b.len() + pad, 0);
    b
}

const CLEAN_SVG: &str = r##"<?xml version="1.0"?><!-- banner --><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><defs><linearGradient id="g"><stop offset="0" stop-color="#f60"/></linearGradient></defs><style>.a{fill:url(#g)}</style><rect class="a" width="10" height="10"/><use href="#g"/></svg>"##;

fn app_with(pool: &PgPool, a: Assets) -> Router {
    pratibimba::http::router(
        pratibimba::http::AppState::configured_full(pool.clone(), Some(7), Some(TOKEN.into()), None, pratibimba::edge::Edge::off(), a),
        &[],
    )
}

async fn upload(app: &Router, site: &str, bytes: Vec<u8>, content_type: &str, name: &str) -> Resp {
    send(
        app,
        Request::post(format!("/api/sites/{site}/assets?name={name}"))
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("content-type", content_type)
            .body(Body::from(bytes))
            .unwrap(),
    )
    .await
}

async fn two_sites(pool: &PgPool) {
    setup_demo(pool).await;
    let mut f = demo_file();
    f.site = "other".into();
    pratibimba::site_file::apply(pool, &f).await.unwrap();
}

async fn mcp_token(pool: &PgPool, site: &str) -> String {
    pratibimba::mcp::issue_token(pool, site, "test agent").await.unwrap()
}

async fn mcp(app: &Router, token: &str, tool: &str, mut args: Value) -> Value {
    args["agent"] = json!("Claude Code");
    args["justification"] = json!("testing assets");
    let body = json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": { "name": tool, "arguments": args } });
    let r = send(
        app,
        Request::post("/mcp").header("content-type", "application/json").header("authorization", format!("Bearer {token}")).body(Body::from(body.to_string())).unwrap(),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json()["result"].clone()
}

fn tool_ok(r: &Value) -> Value {
    assert_ne!(r["isError"], json!(true), "tool failed: {r}");
    r["structuredContent"].clone()
}

fn tool_err(r: &Value) -> String {
    assert_eq!(r["isError"], json!(true), "expected a refusal: {r}");
    r["content"][0]["text"].as_str().unwrap().to_string()
}

// ---- upload and serve ----

#[sqlx::test(migrations = false)]
async fn a_png_upload_is_content_addressed_and_served_immutable(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app_with(&pool, Assets::memory());
    let bytes = png(1200, 400, 50);
    let r = upload(&app, "demo", bytes.clone(), "application/octet-stream", "hero.png").await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    let j = r.json();
    let id = assets::sha256_hex(&bytes);
    assert_eq!(j["id"], json!(id));
    assert_eq!(j["mime"], json!("image/png"));
    assert_eq!((j["width"].clone(), j["height"].clone()), (json!(1200), json!(400)));

    let got = get(&app, &format!("/assets/demo/{id}")).await;
    assert_eq!(got.status, StatusCode::OK);
    assert_eq!(got.headers["content-type"], "image/png");
    assert_eq!(got.headers["cache-control"], "public, max-age=31536000, immutable");
    assert_eq!(got.headers["x-content-type-options"], "nosniff");
    assert_eq!(got.headers["etag"], format!("\"{id}\"").as_str());
    let again = send(&app, Request::get(format!("/assets/demo/{id}")).header("if-none-match", format!("\"{id}\"")).body(Body::empty()).unwrap()).await;
    assert_eq!(again.status, StatusCode::NOT_MODIFIED);

    // The same bytes again: the same id, one row.
    assert_eq!(upload(&app, "demo", bytes, "image/png", "copy.png").await.json()["id"], json!(id));
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM assets").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
    let list = api_get(&app, "/api/sites/demo/assets").await.json();
    assert_eq!(list["assets"].as_array().unwrap().len(), 1);
    assert_eq!(list["assets"][0]["in_use"], json!(false));
}

#[sqlx::test(migrations = false)]
async fn every_accepted_type_is_recognised_by_its_bytes(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app_with(&pool, Assets::memory());
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11, 8, 0, 50, 0, 100, 3, 0, 0, 0];
    jpeg.extend_from_slice(&[0; 8]);
    let mut webp = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
    webp.extend_from_slice(&[0; 16]);
    let mut avif = vec![0, 0, 0, 28];
    avif.extend_from_slice(b"ftypmif1\0\0\0\0mif1avifmiaf");
    avif.extend_from_slice(&[0; 8]);
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (jpeg, "image/jpeg"),
        (b"GIF89a\x02\x00\x03\x00rest".to_vec(), "image/gif"),
        (webp, "image/webp"),
        (avif, "image/avif"),
        (CLEAN_SVG.as_bytes().to_vec(), "image/svg+xml"),
    ];
    for (bytes, mime) in cases {
        // Declared as a PNG every time: the declaration is ignored.
        let r = upload(&app, "demo", bytes, "image/png", "x.png").await;
        assert_eq!(r.status, StatusCode::CREATED, "{mime}: {}", r.text);
        assert_eq!(r.json()["mime"], json!(mime));
    }
    let jpeg_row: (Option<i32>, Option<i32>) = sqlx::query_as("SELECT width, height FROM assets WHERE mime = 'image/jpeg'").fetch_one(&pool).await.unwrap();
    assert_eq!(jpeg_row, (Some(100), Some(50)));
}

#[sqlx::test(migrations = false)]
async fn a_file_that_only_claims_to_be_an_image_is_refused(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app_with(&pool, Assets::memory());
    let spoofs: Vec<(&[u8], &str, &str)> = vec![
        (b"<html><script>alert(1)</script></html>", "image/png", "banner.png"),
        (b"<!DOCTYPE svg [<!ENTITY x \"y\">]><svg xmlns=\"http://www.w3.org/2000/svg\">&x;</svg>", "image/svg+xml", "a.svg"),
        (b"%PDF-1.7 not an image", "image/jpeg", "a.jpg"),
        (b"\x00\x00\x00\x18ftypmp42\x00\x00\x00\x00mp42isom", "image/avif", "a.avif"),
    ];
    for (bytes, ct, name) in spoofs {
        let r = upload(&app, "demo", bytes.to_vec(), ct, name).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{name}: {}", r.text);
        assert!(r.json()["error"].as_str().unwrap().contains("judged by the file's bytes"), "{}", r.text);
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM assets").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test(migrations = false)]
async fn an_svg_that_could_run_script_or_load_anything_is_refused(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app_with(&pool, Assets::memory());
    let svg = |inner: &str| format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">{inner}</svg>"#);
    let bad = [
        svg("<script>alert(1)</script>"),
        svg("<SCRIPT>alert(1)</SCRIPT>"),
        svg(r#"<rect onload="alert(1)" width="1"/>"#),
        svg(r#"<rect ONclick='alert(1)'/>"#),
        svg(r#"<a href="javascript:alert(1)"><rect/></a>"#),
        svg(r#"<a xlink:href="https://evil.example/x"><rect/></a>"#),
        svg(r#"<image href="https://tracker.example/p.gif"/>"#),
        svg(r#"<a href="&#106;avascript:alert(1)"><rect/></a>"#),
        svg(r##"<animate attributeName="href" values="#a"/>"##),
        svg(r#"<set attributeName="hre&#102;" to="x"/>"#),
        svg("<foreignObject><div>hi</div></foreignObject>"),
        svg(r#"<style>@import url(https://evil.example/a.css);</style>"#),
        svg(r#"<rect style="fill:url(https://evil.example/a)"/>"#),
        svg(r#"<rect style="fill:u\72l(https://evil.example/a)"/>"#),
        svg(r#"<image href="data:image/svg+xml;base64,PHN2Zz4="/>"#),
    ];
    for b in bad {
        let r = upload(&app, "demo", b.clone().into_bytes(), "image/svg+xml", "x.svg").await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "accepted: {b}");
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM assets").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test(migrations = false)]
async fn every_asset_is_served_under_a_policy_that_cannot_run_script(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app_with(&pool, Assets::memory());
    let id = upload(&app, "demo", CLEAN_SVG.as_bytes().to_vec(), "image/svg+xml", "logo.svg").await.json()["id"].as_str().unwrap().to_string();
    let got = get(&app, &format!("/assets/demo/{id}")).await;
    assert_eq!(got.status, StatusCode::OK);
    assert_eq!(got.headers["content-type"], "image/svg+xml");
    let csp = got.headers["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("default-src 'none'") && csp.contains("sandbox"), "{csp}");
    assert!(!csp.contains("script-src"), "{csp}");
    assert!(got.headers["content-disposition"].to_str().unwrap().starts_with("attachment"));
    assert_eq!(got.headers["x-content-type-options"], "nosniff");
    // A raster image gets the same policy.
    let pid = upload(&app, "demo", png(2, 2, 0), "image/png", "p.png").await.json()["id"].as_str().unwrap().to_string();
    let p = get(&app, &format!("/assets/demo/{pid}")).await;
    assert!(p.headers["content-security-policy"].to_str().unwrap().contains("sandbox"));
}

#[sqlx::test(migrations = false)]
async fn an_upload_over_the_cap_is_refused_with_413(pool: PgPool) {
    setup_demo(&pool).await;
    let mut a = Assets::memory();
    a.max_bytes = 1000;
    let app = app_with(&pool, a);
    let at_cap = png(1, 1, 1000 - png(1, 1, 0).len());
    assert_eq!(at_cap.len(), 1000);
    assert_eq!(upload(&app, "demo", at_cap, "image/png", "ok.png").await.status, StatusCode::CREATED);
    let over = png(1, 1, 1001 - png(1, 1, 0).len());
    let r = upload(&app, "demo", over, "image/png", "big.png").await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE, "{}", r.text);
    let far_over = png(1, 1, 50_000);
    assert_eq!(upload(&app, "demo", far_over, "image/png", "huge.png").await.status, StatusCode::PAYLOAD_TOO_LARGE);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM assets").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test(migrations = false)]
async fn an_empty_upload_and_an_unknown_site_are_refused(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app_with(&pool, Assets::memory());
    assert_eq!(upload(&app, "demo", vec![], "image/png", "e.png").await.status, StatusCode::BAD_REQUEST);
    assert_eq!(upload(&app, "nope", png(1, 1, 0), "image/png", "e.png").await.status, StatusCode::NOT_FOUND);
    // Uploading needs the admin token.
    let r = send(&app, Request::post("/api/sites/demo/assets").body(Body::from(png(1, 1, 0))).unwrap()).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

// ---- paths and sites ----

#[sqlx::test(migrations = false)]
async fn no_path_reaches_outside_the_store(pool: PgPool) {
    setup_demo(&pool).await;
    let dir = std::env::temp_dir().join(format!("prb-assets-{}", uuid::Uuid::new_v4().simple()));
    let secret = dir.join("secret");
    std::fs::create_dir_all(dir.join("store")).unwrap();
    std::fs::write(&secret, b"top secret").unwrap();
    let app = app_with(&pool, Assets::local(dir.join("store")));
    for p in [
        "/assets/demo/..%2Fsecret".to_string(),
        "/assets/..%2F..%2Fsecret/x".into(),
        "/assets/demo/../../secret".into(),
        format!("/assets/..%2Fsecret/{}", "a".repeat(64)),
        format!("/assets/demo/{}%2F..%2F..%2Fsecret", "a".repeat(64)),
    ] {
        let r = get(&app, &p).await;
        assert_ne!(r.status, StatusCode::OK, "{p}");
        assert!(!r.text.contains("top secret"), "{p}");
    }
    // The store refuses a key that is not <site>/<sha256>, whatever the caller.
    let store = assets::Local { dir: dir.join("store") };
    use assets::Store;
    for k in ["../secret", "demo/../../secret", "demo/x", "/etc/passwd", "demo/../demo", "../x/aaaa", "demo\\..\\secret"] {
        assert!(store.get(k).await.is_err(), "{k}");
        assert!(store.put(k, b"x", "image/png").await.is_err(), "{k}");
    }
    let hash = "a".repeat(64);
    assert!(store.get(&format!("../{hash}")).await.is_err());
    assert!(store.get(&format!("../../{hash}")).await.is_err());
    // A good upload lands where it should.
    let bytes = png(3, 3, 0);
    let id = upload(&app, "demo", bytes.clone(), "image/png", "../../evil.png").await.json()["id"].as_str().unwrap().to_string();
    assert_eq!(std::fs::read(dir.join("store").join("demo").join(&id)).unwrap(), bytes);
    std::fs::remove_dir_all(dir).unwrap();
}

#[sqlx::test(migrations = false)]
async fn one_sites_asset_is_not_another_sites(pool: PgPool) {
    two_sites(&pool).await;
    let a = Assets::memory();
    let app = app_with(&pool, a.clone());
    let id = upload(&app, "demo", png(4, 4, 0), "image/png", "a.png").await.json()["id"].as_str().unwrap().to_string();
    // Served under its own site only, though the bytes exist.
    assert_eq!(get(&app, &format!("/assets/demo/{id}")).await.status, StatusCode::OK);
    assert_eq!(get(&app, &format!("/assets/other/{id}")).await.status, StatusCode::NOT_FOUND);
    // Even bytes in the store under the other site's key are not served without its own row.
    use assets::Store;
    a.store.put(&assets::key("other", &id), &png(4, 4, 0), "image/png").await.unwrap();
    assert_eq!(get(&app, &format!("/assets/other/{id}")).await.status, StatusCode::NOT_FOUND);
    // Listed for its site only, by the dashboard and over MCP.
    assert_eq!(api_get(&app, "/api/sites/other/assets").await.json()["assets"], json!([]));
    let t = mcp_token(&pool, "other").await;
    assert_eq!(tool_ok(&mcp(&app, &t, "list_assets", json!({})).await)["assets"], json!([]));
    let mine = mcp_token(&pool, "demo").await;
    assert_eq!(tool_ok(&mcp(&app, &mine, "list_assets", json!({})).await)["assets"][0]["id"], json!(id));
}

// ---- MCP ----

#[sqlx::test(migrations = false)]
async fn an_agent_uploads_over_mcp_and_the_audit_keeps_no_bytes(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app_with(&pool, Assets::memory());
    let t = mcp_token(&pool, "demo").await;
    use base64::Engine;
    let bytes = png(640, 360, 2000);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let r = tool_ok(&mcp(&app, &t, "upload_asset", json!({ "data": b64, "name": "hero.png" })).await);
    assert_eq!(r["id"], json!(assets::sha256_hex(&bytes)));
    assert_eq!(r["use"]["image_field"]["asset"], r["id"]);
    let (by, args): (String, Value) = sqlx::query_as("SELECT a.uploaded_by, c.arguments FROM assets a JOIN mcp_calls c ON c.id = a.mcp_call_id").fetch_one(&pool).await.unwrap();
    assert_eq!(by, "Claude Code");
    assert!(!args.to_string().contains(&b64[..200]), "the audit row holds the file");
    assert!(args["data"].as_str().unwrap().contains("base64 characters"));
    // Not base64, and a spoof, are refused as tool errors.
    assert!(tool_err(&mcp(&app, &t, "upload_asset", json!({ "data": "%%%not base64" })).await).contains("base64"));
    let html = base64::engine::general_purpose::STANDARD.encode(b"<html><body>hi</body></html>");
    assert!(tool_err(&mcp(&app, &t, "upload_asset", json!({ "data": html, "name": "x.png" })).await).contains("bytes"));
}

#[sqlx::test(migrations = false)]
async fn an_mcp_upload_of_a_few_megabytes_fits_its_body_limit(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app_with(&pool, Assets::memory());
    let t = mcp_token(&pool, "demo").await;
    use base64::Engine;
    // 3 MiB of image is 4 MiB of base64: over axum's 2 MiB default, under the 5 MiB cap.
    let bytes = png(1, 1, 3 * 1024 * 1024);
    let r = tool_ok(&mcp(&app, &t, "upload_asset", json!({ "data": base64::engine::general_purpose::STANDARD.encode(&bytes) })).await);
    assert_eq!(r["bytes"], json!(bytes.len()));
}

#[sqlx::test(migrations = false)]
async fn an_mcp_upload_over_the_cap_is_refused(pool: PgPool) {
    setup_demo(&pool).await;
    let mut a = Assets::memory();
    a.max_bytes = 1000;
    let app = app_with(&pool, a);
    let t = mcp_token(&pool, "demo").await;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png(1, 1, 1500));
    assert!(tool_err(&mcp(&app, &t, "upload_asset", json!({ "data": b64 })).await).contains("limit is 1000"));
}

// ---- garbage collection ----

async fn hero_slot(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT id FROM slots WHERE site_id = 'demo' AND key = 'hero'").fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn gc_keeps_what_a_variant_names_and_deletes_the_rest_after_the_grace(pool: PgPool) {
    setup_demo(&pool).await;
    let a = Assets::memory();
    let app = app_with(&pool, a.clone());
    let used = upload(&app, "demo", png(5, 5, 0), "image/png", "used.png").await.json()["id"].as_str().unwrap().to_string();
    let loose = upload(&app, "demo", png(6, 6, 0), "image/png", "loose.png").await.json()["id"].as_str().unwrap().to_string();
    // A draft swap variant names `used` by its URL.
    let slot = hero_slot(&pool).await;
    sqlx::query("INSERT INTO variants (slot_id, key, payload, status) VALUES ($1, 'banner', $2, 'draft')")
        .bind(slot)
        .bind(format!(r#"<img src="/_prb/assets/demo/{used}" alt="Autumn sale">"#))
        .execute(&pool)
        .await
        .unwrap();
    let now = Utc::now();
    let g = assets::gc(&pool, &a, now).await.unwrap();
    assert_eq!(g.revived, 1, "the named upload is in use");
    assert!(g.deleted.is_empty());
    // Inside the grace: nothing goes, even unreferenced.
    let g = assets::gc(&pool, &a, now + Duration::days(6)).await.unwrap();
    assert!(g.deleted.is_empty(), "{g:?}");
    // Past it: the loose one goes, bytes and all; the used one stays.
    let g = assets::gc(&pool, &a, now + Duration::days(8)).await.unwrap();
    assert_eq!(g.deleted, vec![("demo".to_string(), loose.clone())]);
    assert_eq!(get(&app, &format!("/assets/demo/{loose}")).await.status, StatusCode::NOT_FOUND);
    assert!(a.store.get(&assets::key("demo", &loose)).await.unwrap().is_none());
    assert_eq!(get(&app, &format!("/assets/demo/{used}")).await.status, StatusCode::OK);

    // Rejecting the only variant that names it starts its grace from that pass.
    sqlx::query("UPDATE variants SET status = 'rejected' WHERE key = 'banner'").execute(&pool).await.unwrap();
    let later = now + Duration::days(10);
    assert_eq!(assets::gc(&pool, &a, later).await.unwrap().marked, 1);
    assert!(assets::gc(&pool, &a, later + Duration::days(6)).await.unwrap().deleted.is_empty());
    // Named again before the grace ends (a new draft): kept and unmarked.
    sqlx::query("INSERT INTO variants (slot_id, key, payload, status) VALUES ($1, 'banner2', $2, 'draft')")
        .bind(slot)
        .bind(format!(r#"{{"hero":{{"asset":"{used}","alt":"x"}}}}"#))
        .execute(&pool)
        .await
        .unwrap();
    let g = assets::gc(&pool, &a, later + Duration::days(20)).await.unwrap();
    assert_eq!((g.revived, g.deleted.len()), (1, 0));
}

#[sqlx::test(migrations = false)]
async fn another_sites_variant_does_not_keep_an_asset_alive(pool: PgPool) {
    two_sites(&pool).await;
    let a = Assets::memory();
    let app = app_with(&pool, a.clone());
    let id = upload(&app, "demo", png(7, 7, 0), "image/png", "a.png").await.json()["id"].as_str().unwrap().to_string();
    sqlx::query("INSERT INTO variants (slot_id, key, payload, status) SELECT id, 'steal', $1, 'draft' FROM slots WHERE site_id = 'other' AND key = 'hero'")
        .bind(format!(r#"<img src="/_prb/assets/demo/{id}" alt="x">"#))
        .execute(&pool)
        .await
        .unwrap();
    let g = assets::gc(&pool, &a, Utc::now() + Duration::days(8)).await.unwrap();
    assert_eq!(g.deleted.len(), 1);
}

// ---- storage backends ----

#[tokio::test]
async fn the_memory_and_local_stores_round_trip() {
    let dir = std::env::temp_dir().join(format!("prb-assets-{}", uuid::Uuid::new_v4().simple()));
    let stores: Vec<Arc<dyn assets::Store>> = vec![Arc::new(assets::Memory::default()), Arc::new(assets::Local { dir: dir.clone() })];
    let k = assets::key("demo~sim", &"b".repeat(64));
    for s in stores {
        assert!(s.get(&k).await.unwrap().is_none());
        s.put(&k, b"bytes", "image/png").await.unwrap();
        assert_eq!(s.get(&k).await.unwrap().unwrap(), b"bytes");
        s.delete(&k).await.unwrap();
        assert!(s.get(&k).await.unwrap().is_none());
        s.delete(&k).await.unwrap();
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Runs against a real S3-compatible server when `PRB_TEST_S3_ENDPOINT` (and `_BUCKET`,
/// `_ACCESS_KEY`, `_SECRET_KEY`) are set, e.g. MinIO in Docker; skipped otherwise.
#[sqlx::test(migrations = false)]
async fn the_s3_store_round_trips_through_core(pool: PgPool) {
    let Ok(endpoint) = std::env::var("PRB_TEST_S3_ENDPOINT") else {
        eprintln!("skipped: PRB_TEST_S3_ENDPOINT is not set");
        return;
    };
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k}"));
    let s3 = assets::S3 {
        endpoint,
        bucket: env("PRB_TEST_S3_BUCKET"),
        access_key: env("PRB_TEST_S3_ACCESS_KEY"),
        secret_key: env("PRB_TEST_S3_SECRET_KEY"),
        region: "us-east-1".into(),
        http: reqwest::Client::new(),
    };
    let a = Assets::with_store(Arc::new(s3));
    setup_demo(&pool).await;
    let app = app_with(&pool, a.clone());
    let bytes = png(9, 9, 300);
    let id = upload(&app, "demo", bytes.clone(), "image/png", "s3.png").await.json()["id"].as_str().unwrap().to_string();
    let got = send(&app, Request::get(format!("/assets/demo/{id}")).body(Body::empty()).unwrap()).await;
    assert_eq!(got.status, StatusCode::OK);
    assert_eq!(got.headers["content-length"], bytes.len().to_string().as_str());
    let g = assets::gc(&pool, &a, Utc::now() + Duration::days(8)).await.unwrap();
    assert_eq!(g.deleted.len(), 1);
    assert!(a.store.get(&assets::key("demo", &id)).await.unwrap().is_none());
    // Wrong credentials are refused by the server, not silently accepted.
    let bad = assets::S3 {
        endpoint: env("PRB_TEST_S3_ENDPOINT"),
        bucket: env("PRB_TEST_S3_BUCKET"),
        access_key: env("PRB_TEST_S3_ACCESS_KEY"),
        secret_key: "wrong".into(),
        region: "us-east-1".into(),
        http: reqwest::Client::new(),
    };
    use assets::Store;
    assert!(bad.put(&assets::key("demo", &id), &bytes, "image/png").await.is_err());
}

#[test]
fn sniffing_ignores_what_is_not_an_image() {
    assert_eq!(assets::sniff(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), Some("image/svg+xml"));
    assert_eq!(assets::sniff(b"\xef\xbb\xbf  <!-- c --><svg>"), Some("image/svg+xml"));
    assert_eq!(assets::sniff(b"<svgx>"), None);
    assert_eq!(assets::sniff(b"<html><svg></svg></html>"), None);
    assert_eq!(assets::sniff(b"RIFF\0\0\0\0WAVEfmt "), None);
    assert!(assets::check_svg(CLEAN_SVG.as_bytes()).is_ok(), "{:?}", assets::check_svg(CLEAN_SVG.as_bytes()));
}
