mod common;
use common::*;
use pratibimba::{db, site_file};
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
async fn apply_creates_site_slot_variants_model(pool: PgPool) {
    setup_demo(&pool).await;
    assert_eq!(count(&pool, "SELECT count(*) FROM sites WHERE id='demo'").await, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM slots WHERE key='hero' AND selector='#hero'").await, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM variants WHERE status='approved' AND approved_by='cli:apply'").await, 2);
    assert_eq!(count(&pool, "SELECT count(*) FROM variants WHERE is_baseline AND payload IS NULL").await, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM models").await, 1);
}

#[sqlx::test(migrations = false)]
async fn apply_twice_is_idempotent(pool: PgPool) {
    setup_demo(&pool).await;
    let mut f = demo_file();
    f.slots[0].variants[1].html = Some("<h1>Edited</h1>".into());
    site_file::apply(&pool, &f).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM variants").await, 2);
    assert_eq!(count(&pool, "SELECT count(*) FROM variants WHERE payload='<h1>Edited</h1>'").await, 1);
}

#[sqlx::test(migrations = false)]
async fn apply_retires_variants_removed_from_file(pool: PgPool) {
    setup_demo(&pool).await;
    let mut f = demo_file();
    f.slots[0].variants.truncate(1);
    site_file::apply(&pool, &f).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM variants WHERE key='gift' AND status='retired'").await, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM variants WHERE key='baseline' AND status='approved'").await, 1);
}

fn two_variants(a_baseline: bool, b_baseline: bool, a_html: Option<&str>, b_html: Option<&str>) -> site_file::SiteFile {
    file_from(json!({ "site": "demo", "origin": "http://x", "slots": [{ "key": "hero", "selector": "#hero",
        "variants": [ { "key": "a", "baseline": a_baseline, "html": a_html },
                      { "key": "b", "baseline": b_baseline, "html": b_html } ] }] }))
}

async fn assert_refused(pool: &PgPool, f: site_file::SiteFile, needle: &str) {
    db::migrate(pool).await.unwrap();
    let err = site_file::apply(pool, &f).await.expect_err("file should be refused").to_string();
    assert!(err.contains("slot \"hero\"") && err.contains(needle), "unhelpful error: {err}");
    assert_eq!(count(pool, "SELECT count(*) FROM sites").await, 0, "a refused file wrote rows");
}

#[sqlx::test(migrations = false)]
async fn apply_rejects_two_baselines(pool: PgPool) {
    assert_refused(&pool, two_variants(true, true, None, None), "found 2").await;
}

#[sqlx::test(migrations = false)]
async fn apply_rejects_slot_without_baseline(pool: PgPool) {
    assert_refused(&pool, two_variants(false, false, Some("<b>a</b>"), Some("<b>b</b>")), "found 0").await;
}

#[sqlx::test(migrations = false)]
async fn apply_rejects_baseline_with_html(pool: PgPool) {
    assert_refused(&pool, two_variants(true, false, Some("<b>a</b>"), Some("<b>b</b>")), "must not carry html").await;
}

#[sqlx::test(migrations = false)]
async fn apply_rejects_variant_without_html(pool: PgPool) {
    assert_refused(&pool, two_variants(true, false, None, None), "needs html").await;
}

#[sqlx::test(migrations = false)]
async fn apply_moves_baseline_between_variants(pool: PgPool) {
    db::migrate(&pool).await.unwrap();
    site_file::apply(&pool, &two_variants(true, false, None, Some("<b>b</b>"))).await.unwrap();
    // The new baseline is listed FIRST, so it is written while `a` still holds the flag.
    let moved = file_from(json!({ "site": "demo", "origin": "http://x", "slots": [{ "key": "hero", "selector": "#hero",
        "variants": [ { "key": "b", "baseline": true }, { "key": "a", "html": "<b>a</b>" } ] }] }));
    site_file::apply(&pool, &moved).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM variants WHERE key='b' AND is_baseline").await, 1);
}

// ---- P1 templates through the site file (B3) ----

fn copy_file(variants: serde_json::Value) -> serde_json::Value {
    json!({ "site": "demo", "origin": "http://localhost:5173", "slots": [{
        "key": "hero", "selector": "#hero", "kind": "copy",
        "copy": { "fields": [ { "name": "title", "max": 40, "required": true } ] },
        "variants": variants }] })
}

/// The site file applied, or its refusal as text (a bad template is refused when read).
async fn apply_json(pool: &PgPool, v: serde_json::Value) -> Result<(), String> {
    db::migrate(pool).await.unwrap();
    let f: site_file::SiteFile = match serde_json::from_value(v) {
        Ok(f) => f,
        Err(e) => return Err(e.to_string()),
    };
    site_file::apply(pool, &f).await.map_err(|e| e.to_string())
}

#[sqlx::test(migrations = false)]
async fn apply_accepts_template_variant(pool: PgPool) {
    let f = copy_file(json!([{ "key": "own", "baseline": true },
        { "key": "usual", "requires": ["visitor.top_item"], "copy": { "title": "Your usual {visitor.top_item.name | truncate: 20}" } }]));
    apply_json(&pool, f).await.unwrap();
    let (payload, req): (String, Vec<String>) =
        sqlx::query_as("SELECT payload, requires FROM variants WHERE key = 'usual'").fetch_one(&pool).await.unwrap();
    let p: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(p["title"]["src"], "Your usual {visitor.top_item.name | truncate: 20}", "{payload}");
    assert!(p["title"]["tpl"].is_array(), "stored parsed: {payload}");
    assert_eq!(req, vec!["visitor.top_item".to_string()]);
}

// Mutation: drop the copy::check_covered call in copy::validate_slot -> red.
#[sqlx::test(migrations = false)]
async fn apply_refuses_uncovered_required_placeholder(pool: PgPool) {
    // B5's gap: no requires, no fallback, so a new visitor would get an empty required title.
    let f = copy_file(json!([{ "key": "own", "baseline": true },
        { "key": "usual", "copy": { "title": "{visitor.top_item.name | truncate: 20}" } }]));
    let e = apply_json(&pool, f).await.unwrap_err();
    assert!(e.contains("\"usual\"") && e.contains("\"title\" is required") && e.contains("Add \"visitor.top_item.name\" to requires") && e.contains("fallback"), "{e}");
    assert_eq!(count(&pool, "SELECT count(*) FROM variants").await, 0, "nothing applied");
}

// Mutation: skip `check` on templated values in apply (the Template arm of copy::check) -> red.
#[sqlx::test(migrations = false)]
async fn apply_refuses_over_long_template(pool: PgPool) {
    // An item name can be 256 characters; "Your usual " plus that cannot fit a 40-character title.
    let f = copy_file(json!([{ "key": "own", "baseline": true },
        { "key": "usual", "copy": { "title": "Your usual {visitor.top_item.name}" } }]));
    let e = apply_json(&pool, f).await.unwrap_err();
    assert!(e.contains("\"title\"") && e.contains("visitor.top_item.name") && e.contains("max is 40") && e.contains("truncate"), "{e}");
    assert_eq!(count(&pool, "SELECT count(*) FROM variants").await, 0, "nothing applied");
}

// Mutation: pass `false` for `baseline` to validate_requires in site_file::validate_slot -> red
// (the database CHECK still refuses, but without saying why).
#[sqlx::test(migrations = false)]
async fn apply_refuses_requires_on_baseline(pool: PgPool) {
    let f = copy_file(json!([{ "key": "own", "baseline": true, "requires": ["visitor.top_item"] },
        { "key": "x", "copy": { "title": "Hello" } }]));
    let e = apply_json(&pool, f).await.unwrap_err();
    assert!(e.contains("\"own\"") && e.contains("baseline") && e.contains("has no requires"), "{e}");
}

fn page_file(page: serde_json::Value) -> site_file::SiteFile {
    file_from(json!({ "site": "demo", "origin": "http://x", "slots": [{ "key": "hero", "selector": "#hero", "page": page,
        "variants": [ { "key": "baseline", "baseline": true }, { "key": "gift", "html": "<b>x</b>" } ] }] }))
}

#[sqlx::test(migrations = false)]
async fn apply_round_trips_a_slots_page(pool: PgPool) {
    db::migrate(&pool).await.unwrap();
    site_file::apply(&pool, &page_file(json!("/product/x"))).await.unwrap();
    let p: Option<String> = sqlx::query_scalar("SELECT page FROM slots WHERE key = 'hero'").fetch_one(&pool).await.unwrap();
    assert_eq!(p.as_deref(), Some("/product/x"));
    let app = admin_app(&pool);
    let d = api_get(&app, "/api/sites/demo/slots/hero").await.json();
    assert_eq!(d["page"], "/product/x", "{d}");
    // Re-applying without the key clears it; the file is the truth.
    site_file::apply(&pool, &demo_file()).await.unwrap();
    let p: Option<String> = sqlx::query_scalar("SELECT page FROM slots WHERE key = 'hero'").fetch_one(&pool).await.unwrap();
    assert_eq!(p, None);
    let d = api_get(&app, "/api/sites/demo/slots/hero").await.json();
    assert!(d["page"].is_null(), "{d}");
}

#[sqlx::test(migrations = false)]
async fn apply_refuses_a_page_without_a_leading_slash(pool: PgPool) {
    db::migrate(&pool).await.unwrap();
    let err = site_file::apply(&pool, &page_file(json!("product/x"))).await.expect_err("refused").to_string();
    assert!(err.contains("page") && err.contains("\"/\""), "{err}");
    assert_eq!(count(&pool, "SELECT count(*) FROM sites").await, 0);
}
