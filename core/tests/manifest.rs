mod common;
use common::*;
use sqlx::PgPool;

fn variant_keys(m: &serde_json::Value) -> Vec<String> {
    m["slots"][0]["variants"].as_array().unwrap().iter().map(|v| v["key"].as_str().unwrap().to_string()).collect()
}

#[sqlx::test(migrations = false)]
async fn manifest_lists_approved_variants(pool: PgPool) {
    setup_demo(&pool).await;
    let r = get(&app(&pool), "/manifest/demo").await;
    assert_eq!(r.status, StatusCode::OK);
    let m = r.json();
    assert_eq!(m["slots"][0]["selector"], "#hero");
    assert_eq!(variant_keys(&m), ["baseline", "gift"]);
}

#[sqlx::test(migrations = false)]
async fn manifest_excludes_draft_variants(pool: PgPool) {
    setup_demo(&pool).await;
    insert_variant(&pool, "draft_one", "draft").await;
    let keys = variant_keys(&get(&app(&pool), "/manifest/demo").await.json());
    assert!(!keys.contains(&"draft_one".to_string()), "{keys:?}");
}

#[sqlx::test(migrations = false)]
async fn manifest_excludes_retired_variants(pool: PgPool) {
    setup_demo(&pool).await;
    insert_variant(&pool, "old_one", "retired").await;
    let keys = variant_keys(&get(&app(&pool), "/manifest/demo").await.json());
    assert!(!keys.contains(&"old_one".to_string()), "{keys:?}");
}

#[sqlx::test(migrations = false)]
async fn manifest_unknown_site_is_404(pool: PgPool) {
    setup_demo(&pool).await;
    assert_eq!(get(&app(&pool), "/manifest/nope").await.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = false)]
async fn manifest_js_assigns_global(pool: PgPool) {
    setup_demo(&pool).await;
    let r = get(&app(&pool), "/manifest/demo.js").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.text.starts_with("window.__prb_manifest={"), "{}", r.text);
    assert!(r.headers["content-type"].to_str().unwrap().starts_with("application/javascript"));
}

#[sqlx::test(migrations = false)]
async fn manifest_js_cannot_close_a_script_tag(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    let f = file_from(serde_json::json!({ "site": "demo", "origin": "http://x", "slots": [{ "key": "hero",
        "selector": "#a</script><script>alert(1)//", "variants": [{ "key": "b", "baseline": true }] }] }));
    pratibimba::site_file::apply(&pool, &f).await.unwrap();
    let r = get(&app(&pool), "/manifest/demo.js").await;
    assert!(!r.text.contains("</script>"), "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn manifest_is_edge_cacheable(pool: PgPool) {
    setup_demo(&pool).await;
    let r = get(&app(&pool), "/manifest/demo").await;
    let cc = r.headers.get("cache-control").map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
    assert!(cc.contains("max-age=60") && cc.contains("stale-while-revalidate"), "{cc:?}");
}
