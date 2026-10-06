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
