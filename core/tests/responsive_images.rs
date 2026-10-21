//! S12b: responsive variants. An uploaded PNG, JPEG or WebP is also stored resized to each step
//! of the width ladder narrower than itself, served at `?w=<width>` as exactly that size, deleted
//! with the original, and named in copy decisions so renderers can write a srcset.

mod common;
use axum::{Router, body::Body, http::Request};
use common::*;
use http_body_util::BodyExt;
use pratibimba::assets::{self, Assets};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

/// A real, decodable image with some texture (so a smaller variant is also fewer bytes).
fn picture(w: u32, h: u32, format: image::ImageFormat) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        let n = (x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503)) >> 7;
        image::Rgb([(x * 255 / w) as u8 ^ (n as u8 & 15), (y * 255 / h) as u8, (n % 97) as u8])
    });
    let mut buf = std::io::Cursor::new(Vec::new());
    if format == image::ImageFormat::Jpeg {
        // A camera-like quality: Core's variants (quality 82) must come out smaller.
        img.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 95)).unwrap();
    } else {
        img.write_to(&mut buf, format).unwrap();
    }
    buf.into_inner()
}

fn app_with(pool: &PgPool, a: Assets) -> Router {
    pratibimba::http::router(
        pratibimba::http::AppState::configured_full(pool.clone(), Some(7), Some(TOKEN.into()), None, pratibimba::edge::Edge::off(), a),
        &[],
    )
}

async fn upload(app: &Router, bytes: Vec<u8>) -> Value {
    let r = send(app, Request::post("/api/sites/demo/assets").header("authorization", format!("Bearer {TOKEN}")).body(Body::from(bytes)).unwrap()).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    r.json()
}

/// GET returning the raw bytes (the shared helper keeps text only).
async fn fetch(app: &Router, path: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let r = app.clone().oneshot(Request::get(path).body(Body::empty()).unwrap()).await.unwrap();
    let (parts, body) = r.into_parts();
    (parts.status, parts.headers, body.collect().await.unwrap().to_bytes().to_vec())
}

fn size_of(bytes: &[u8]) -> (u32, u32) {
    let img = image::load_from_memory(bytes).expect("a decodable image");
    (img.width(), img.height())
}

#[sqlx::test(migrations = false)]
async fn a_jpeg_is_stored_at_each_narrower_width_in_its_own_format(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app_with(&pool, Assets::memory());
    let original = picture(2000, 1000, image::ImageFormat::Jpeg);
    let j = upload(&app, original.clone()).await;
    assert_eq!(j["widths"], json!([320, 640, 960, 1280, 1920]), "{j}");
    let id = j["id"].as_str().unwrap();
    for w in [320u32, 640, 960, 1280, 1920] {
        let (status, headers, bytes) = fetch(&app, &format!("/assets/demo/{id}?w={w}")).await;
        assert_eq!(status, StatusCode::OK, "w={w}");
        assert_eq!(headers["content-type"], "image/jpeg");
        assert_eq!(headers["cache-control"], assets::CACHE);
        assert_eq!(headers["content-security-policy"], assets::CSP);
        assert_eq!(headers["etag"], format!("\"{id}.w{w}\"").as_str());
        assert_eq!(size_of(&bytes), (w, w / 2), "the variant is the size its width says");
        assert!(bytes.len() < original.len());
    }
    // The original is still the original.
    let (_, _, bytes) = fetch(&app, &format!("/assets/demo/{id}")).await;
    assert_eq!(bytes, original);
    // And the listing names the widths.
    assert_eq!(api_get(&app, "/api/sites/demo/assets").await.json()["assets"][0]["widths"], json!([320, 640, 960, 1280, 1920]));
}

#[sqlx::test(migrations = false)]
async fn only_steps_narrower_than_the_image_exist_and_any_other_width_is_404(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app_with(&pool, Assets::memory());
    let j = upload(&app, picture(700, 300, image::ImageFormat::Png)).await;
    assert_eq!(j["widths"], json!([320, 640]), "{j}");
    let id = j["id"].as_str().unwrap();
    let (status, headers, bytes) = fetch(&app, &format!("/assets/demo/{id}?w=640")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "image/png");
    assert_eq!(size_of(&bytes), (640, 274));
    // Never the original under another width's name, and nothing off the ladder.
    for w in ["960", "1920", "500", "0", "abc", "640.0", "-320"] {
        assert_eq!(fetch(&app, &format!("/assets/demo/{id}?w={w}")).await.0, StatusCode::NOT_FOUND, "w={w}");
    }
    // A WebP gets its own-format variants too.
    let j = upload(&app, picture(400, 200, image::ImageFormat::WebP)).await;
    assert_eq!(j["widths"], json!([320]), "{j}");
    let (status, headers, bytes) = fetch(&app, &format!("/assets/demo/{}?w=320", j["id"].as_str().unwrap())).await;
    assert_eq!((status, headers["content-type"].to_str().unwrap()), (StatusCode::OK, "image/webp"));
    assert_eq!(size_of(&bytes), (320, 160));
}

#[sqlx::test(migrations = false)]
async fn what_core_cannot_resize_is_still_stored_without_variants(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app_with(&pool, Assets::memory());
    // A PNG header with no image data: sniffed as PNG, does not decode.
    let mut fake = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    fake.extend_from_slice(&3000u32.to_be_bytes());
    fake.extend_from_slice(&1000u32.to_be_bytes());
    fake.extend_from_slice(&[8, 6, 0, 0, 0, 1, 2, 3, 4]);
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="3000" height="900" viewBox="0 0 10 3"><rect width="10" height="3"/></svg>"#.to_vec();
    for bytes in [fake, svg] {
        let j = upload(&app, bytes).await;
        assert_eq!(j["widths"], json!([]), "{j}");
        assert_eq!(fetch(&app, &format!("/assets/demo/{}?w=320", j["id"].as_str().unwrap())).await.0, StatusCode::NOT_FOUND);
    }
}

#[sqlx::test(migrations = false)]
async fn a_reupload_fills_in_variants_for_an_asset_stored_without_them(pool: PgPool) {
    setup_demo(&pool).await;
    let a = Assets::memory();
    let app = app_with(&pool, a.clone());
    let bytes = picture(1000, 500, image::ImageFormat::Jpeg);
    let id = upload(&app, bytes.clone()).await["id"].as_str().unwrap().to_string();
    // As if stored before S12b: no widths, no variant bytes.
    sqlx::query("UPDATE assets SET widths = '{}'").execute(&pool).await.unwrap();
    for w in [320, 640, 960] {
        a.store.delete(&assets::variant_key("demo", &id, w)).await.unwrap();
    }
    assert_eq!(fetch(&app, &format!("/assets/demo/{id}?w=640")).await.0, StatusCode::NOT_FOUND);
    assert_eq!(upload(&app, bytes).await["widths"], json!([320, 640, 960]));
    assert_eq!(fetch(&app, &format!("/assets/demo/{id}?w=640")).await.0, StatusCode::OK);
}

#[sqlx::test(migrations = false)]
async fn gc_deletes_the_variants_bytes_with_the_original(pool: PgPool) {
    setup_demo(&pool).await;
    let a = Assets::memory();
    let app = app_with(&pool, a.clone());
    let id = upload(&app, picture(1000, 500, image::ImageFormat::Jpeg)).await["id"].as_str().unwrap().to_string();
    for w in [320, 640, 960] {
        assert!(a.store.get(&assets::variant_key("demo", &id, w)).await.unwrap().is_some());
    }
    let g = assets::gc(&pool, &a, chrono::Utc::now() + chrono::Duration::days(8)).await.unwrap();
    assert_eq!(g.deleted, vec![("demo".to_string(), id.clone())]);
    assert!(a.store.get(&assets::key("demo", &id)).await.unwrap().is_none());
    for w in [320, 640, 960] {
        assert!(a.store.get(&assets::variant_key("demo", &id, w)).await.unwrap().is_none(), "w{w} left behind");
    }
}

#[tokio::test]
async fn a_local_store_keeps_variants_beside_the_original() {
    let dir = std::env::temp_dir().join(format!("prb-s12b-{}", uuid::Uuid::new_v4().simple()));
    let a = Assets::local(&dir);
    let id = assets::sha256_hex(b"x");
    a.store.put(&assets::variant_key("demo", &id, 640), b"small", "image/png").await.unwrap();
    assert!(dir.join("demo").join(format!("{id}.w640")).is_file());
    for bad in [format!("demo/{id}.w"), format!("demo/{id}.w064"), format!("demo/{id}.wx"), format!("demo/{id}.w640/../x")] {
        assert!(a.store.put(&bad, b"x", "image/png").await.is_err(), "{bad}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

// ---- decisions name the widths ----

#[sqlx::test(migrations = false)]
async fn a_copy_decision_carries_the_widths_its_image_was_stored_at(pool: PgPool) {
    setup_demo(&pool).await;
    set_holdout(&pool, 0.0).await;
    let app = app_with(&pool, Assets::memory());
    let wide = upload(&app, picture(1600, 500, image::ImageFormat::Jpeg)).await["id"].as_str().unwrap().to_string();
    let tiny = upload(&app, picture(200, 100, image::ImageFormat::Jpeg)).await["id"].as_str().unwrap().to_string();
    let file = json!({ "site": "demo", "origin": "http://localhost:5173", "privacy_mode": "full", "slots": [{
        "key": "banner", "selector": "#banner", "kind": "copy", "cold_start_exposures": 0,
        "copy": { "fields": [ { "name": "photo", "type": "image", "required": true }, { "name": "thumb", "type": "image" }, { "name": "logo", "type": "image" } ] },
        "variants": [ { "key": "own", "baseline": true },
                      { "key": "autumn", "copy": { "photo": { "asset": wide, "alt": "Autumn beans", "width": 800 },
                                                   "thumb": { "asset": tiny, "alt": "A bean" },
                                                   "logo": { "url": "https://cdn.example.com/logo.png", "alt": "Logo" } } } ] }] });
    let file: pratibimba::site_file::SiteFile = serde_json::from_value(file).unwrap();
    pratibimba::site_file::apply(&pool, &file).await.unwrap();
    let mut seen = 0;
    for i in 0..40 {
        let d = post(&app, "/decide", json!({ "site": "demo", "slot": "banner", "subject": format!("v{i}") })).await.json();
        if d["variant"] != "autumn" {
            continue;
        }
        seen += 1;
        assert_eq!(
            d["copy"]["photo"],
            json!({ "asset": wide, "alt": "Autumn beans", "width": 800, "widths": [320, 640, 960, 1280], "natural_width": 1600 }),
            "{d}"
        );
        assert_eq!(d["copy"]["thumb"], json!({ "asset": tiny, "alt": "A bean" }), "an image narrower than every step has no widths");
        assert_eq!(d["copy"]["logo"], json!({ "url": "https://cdn.example.com/logo.png", "alt": "Logo" }), "a URL image has no widths");
    }
    assert!(seen >= 2, "autumn decided {seen} times");
}
