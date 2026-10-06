//! Promoted audiences gate decide: a variant restricted to an audience is served only to
//! subjects whose decision context matches the audience's predicate, and the decision's
//! explanation says which audiences were evaluated and which variants they kept out.

mod common;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn segment(app: &axum::Router, predicate: Value) -> i64 {
    let r = api_post(app, "/api/sites/demo/segments", json!({ "predicate": predicate, "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    r.json()["id"].as_i64().unwrap()
}

async fn promote(app: &axum::Router, id: i64) {
    let r = api_post(app, &format!("/api/segments/{id}/promote"), json!({ "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
}

async fn restrict(app: &axum::Router, variant: i64, audience: Option<i64>) -> Resp {
    api_post(app, &format!("/api/variants/{variant}/audience"), json!({ "audience": audience, "actor": "owner" })).await
}

/// Demo site with gift restricted to the promoted audience "returning visitors".
async fn gated(pool: &PgPool) -> (axum::Router, i64) {
    setup_demo(pool).await;
    set_holdout(pool, 0.0).await;
    let admin = admin_app(pool);
    let aud = segment(&admin, json!({ "returning": true })).await;
    promote(&admin, aud).await;
    let r = restrict(&admin, variant_id(pool, "gift").await, Some(aud)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    (admin, aud)
}

async fn shown(app: &axum::Router, subject: &str, ctx: Value) -> Value {
    let r = post(app, "/decide", json!({ "site": "demo", "slot": "hero", "subject": subject, "ctx": ctx })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json()
}

#[sqlx::test(migrations = false)]
async fn a_gated_variant_is_never_served_outside_its_audience(pool: PgPool) {
    gated(&pool).await;
    let app = app(&pool);
    for i in 0..60 {
        let d = shown(&app, &format!("new{i}"), json!({ "returning": false })).await;
        assert_eq!(d["variant"], "baseline", "a first-time visitor was shown gift: {d}");
    }
    let gifts = futures_count(&app, |i| format!("back{i}"), json!({ "returning": true })).await;
    assert!(gifts > 0, "matching visitors never saw gift");
}

async fn futures_count(app: &axum::Router, subject: impl Fn(usize) -> String, ctx: Value) -> usize {
    let mut n = 0;
    for i in 0..60 {
        n += (shown(app, &subject(i), ctx.clone()).await["variant"] == "gift") as usize;
    }
    n
}

#[sqlx::test(migrations = false)]
async fn the_explanation_names_the_audience_and_what_it_kept_out(pool: PgPool) {
    let (admin, aud) = gated(&pool).await;
    let app = app(&pool);
    let out = shown(&app, "new", json!({ "returning": false })).await;
    let a = &out["explanation"]["audiences"];
    assert_eq!(a, &json!([{ "id": aud, "name": "Returning visitors", "matched": false, "variants": ["gift"] }]), "{out}");
    let inn = shown(&app, "back", json!({ "returning": true })).await;
    assert_eq!(inn["explanation"]["audiences"][0]["matched"], json!(true));
    // The decision record keeps it, so the detail page can say it later.
    let id = out["decision_id"].as_str().unwrap();
    let detail = api_get(&admin, &format!("/api/decisions/{id}")).await.json();
    assert_eq!(detail["audiences"][0]["matched"], json!(false), "{detail}");
}

#[sqlx::test(migrations = false)]
async fn only_a_promoted_audience_can_gate(pool: PgPool) {
    setup_demo(&pool).await;
    let admin = admin_app(&pool);
    let seg = segment(&admin, json!({ "mobile": true })).await;
    let r = restrict(&admin, variant_id(&pool, "gift").await, Some(seg)).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "an active, unpromoted segment gated a variant: {}", r.text);
}

#[sqlx::test(migrations = false)]
async fn the_baseline_cannot_be_gated(pool: PgPool) {
    setup_demo(&pool).await;
    let admin = admin_app(&pool);
    let seg = segment(&admin, json!({ "mobile": true })).await;
    promote(&admin, seg).await;
    let r = restrict(&admin, variant_id(&pool, "baseline").await, Some(seg)).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
}

/// An audience that loses its permission (archived) closes its gate: nobody gets the variant.
#[sqlx::test(migrations = false)]
async fn an_archived_audience_fails_closed(pool: PgPool) {
    let (admin, aud) = gated(&pool).await;
    let r = api_post(&admin, &format!("/api/segments/{aud}/archive"), json!({ "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::OK);
    let n = futures_count(&app(&pool), |i| format!("back{i}"), json!({ "returning": true })).await;
    assert_eq!(n, 0, "gift served through an archived audience");
}

/// A time predicate cannot match a visitor whose local clock is unknown.
#[sqlx::test(migrations = false)]
async fn a_time_audience_needs_a_known_clock(pool: PgPool) {
    setup_demo(&pool).await;
    set_holdout(&pool, 0.0).await;
    let admin = admin_app(&pool);
    // Every hour of the day, so a known clock always matches.
    let aud = segment(&admin, json!({ "hour_local": [0, 23] })).await;
    promote(&admin, aud).await;
    restrict(&admin, variant_id(&pool, "gift").await, Some(aud)).await;
    let app = app(&pool);
    let d = shown(&app, "unknown", json!({})).await;
    assert_eq!(d["explanation"]["audiences"][0]["matched"], json!(false), "{d}");
    let d = shown(&app, "known", json!({ "tz_offset": 330 })).await;
    assert_eq!(d["explanation"]["audiences"][0]["matched"], json!(true), "{d}");
}

#[sqlx::test(migrations = false)]
async fn lifting_the_gate_serves_everyone_again(pool: PgPool) {
    let (admin, _) = gated(&pool).await;
    let r = restrict(&admin, variant_id(&pool, "gift").await, None).await;
    assert_eq!(r.status, StatusCode::OK);
    let n = futures_count(&app(&pool), |i| format!("new{i}"), json!({ "returning": false })).await;
    assert!(n > 0);
    let d = shown(&app(&pool), "x", json!({})).await;
    assert_eq!(d["explanation"]["audiences"], json!([]));
}
