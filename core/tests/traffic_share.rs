//! G2: a live variant's traffic share is editable, under the same rule as every other change.
//!
//! - A person edits it in the dashboard API (`POST /api/variants/{id}/traffic-share`): range
//!   checked as approval checks it (above 0, at most 50%), audited in `approvals` as `share`
//!   with the value it replaced, and read by the very next decide.
//! - An agent only PROPOSES one (MCP `update_slot` with `traffic_share`), which a person
//!   confirms or declines (`/api/share-requests/{id}/confirm|decline`).

mod common;
use axum::{Router, body::Body, http::Request};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn setup(pool: &PgPool) -> (Router, String) {
    setup_learning(pool).await;
    let t = pratibimba::mcp::issue_token(pool, "demo", "test agent").await.unwrap();
    (admin_app(pool), t)
}

async fn share_of(pool: &PgPool, key: &str) -> Option<f32> {
    sqlx::query_scalar("SELECT traffic_share FROM variants WHERE key = $1").bind(key).fetch_one(pool).await.unwrap()
}

async fn edit(app: &Router, id: i64, body: Value) -> Resp {
    api_post(app, &format!("/api/variants/{id}/traffic-share"), body).await
}

async fn call(app: &Router, token: &str, tool: &str, args: Value) -> Value {
    let body = json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": { "name": tool, "arguments": args } });
    let req = Request::post("/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .unwrap();
    let r = send(app, req).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json()["result"].clone()
}

fn data(result: &Value) -> Value {
    assert_ne!(result["isError"], json!(true), "tool failed: {result}");
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn err_text(result: &Value) -> String {
    assert_eq!(result["isError"], json!(true), "expected a tool error: {result}");
    result["content"][0]["text"].as_str().unwrap().to_string()
}

async fn propose(app: &Router, t: &str, extra: Value) -> Value {
    let mut a = json!({ "agent": "Claude Code", "justification": "gift is winning; give it more room", "slot": "hero" });
    a.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    call(app, t, "update_slot", a).await
}

async fn open_request(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT id FROM share_requests WHERE status = 'pending'").fetch_one(pool).await.unwrap()
}

// ---- the owner's edit ------------------------------------------------------------------------

#[sqlx::test(migrations = false)]
async fn the_owner_edits_a_live_variants_share(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    let gift = variant_id(&pool, "gift").await;
    let r = edit(&app, gift, json!({ "actor": "owner", "traffic_share": 0.2 })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.json()["traffic_share"].as_f64().map(|v| v as f32), Some(0.2));
    assert_eq!(share_of(&pool, "gift").await, Some(0.2));
}

#[sqlx::test(migrations = false)]
async fn a_share_of_half_is_allowed(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    let gift = variant_id(&pool, "gift").await;
    assert_eq!(edit(&app, gift, json!({ "actor": "owner", "traffic_share": 0.5 })).await.status, StatusCode::OK);
    assert_eq!(share_of(&pool, "gift").await, Some(0.5));
}

#[sqlx::test(migrations = false)]
async fn a_share_above_half_is_refused(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    let gift = variant_id(&pool, "gift").await;
    let r = edit(&app, gift, json!({ "actor": "owner", "traffic_share": 0.51 })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    assert_eq!(share_of(&pool, "gift").await, None);
}

#[sqlx::test(migrations = false)]
async fn a_zero_share_is_refused(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    let gift = variant_id(&pool, "gift").await;
    let r = edit(&app, gift, json!({ "actor": "owner", "traffic_share": 0.0 })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    assert_eq!(share_of(&pool, "gift").await, None);
}

#[sqlx::test(migrations = false)]
async fn a_share_edit_needs_a_name(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    let gift = variant_id(&pool, "gift").await;
    assert_eq!(edit(&app, gift, json!({ "actor": " ", "traffic_share": 0.2 })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(share_of(&pool, "gift").await, None);
}

#[sqlx::test(migrations = false)]
async fn the_baseline_has_no_share_to_edit(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    let base = variant_id(&pool, "baseline").await;
    let r = edit(&app, base, json!({ "actor": "owner", "traffic_share": 0.2 })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    assert_eq!(share_of(&pool, "baseline").await, None);
}

#[sqlx::test(migrations = false)]
async fn a_draft_gets_its_share_at_approval_not_here(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    insert_variant(&pool, "spring", "draft").await;
    let id = variant_id(&pool, "spring").await;
    let r = edit(&app, id, json!({ "actor": "owner", "traffic_share": 0.2 })).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text);
    assert_eq!(share_of(&pool, "spring").await, None);
}

#[sqlx::test(migrations = false)]
async fn a_retired_variant_cannot_be_given_a_share(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    insert_variant(&pool, "old", "retired").await;
    let id = variant_id(&pool, "old").await;
    assert_eq!(edit(&app, id, json!({ "actor": "owner", "traffic_share": 0.2 })).await.status, StatusCode::CONFLICT);
}

#[sqlx::test(migrations = false)]
async fn a_share_edit_is_audited_with_the_value_it_replaced(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    let gift = variant_id(&pool, "gift").await;
    edit(&app, gift, json!({ "actor": "Asha", "traffic_share": 0.3 })).await;
    edit(&app, gift, json!({ "actor": "Ritwik", "traffic_share": 0.1, "comment": "too much too soon" })).await;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, Option<f32>, Option<f32>, Option<String>)> = sqlx::query_as(
        "SELECT action, actor, traffic_share, previous_share, comment FROM approvals WHERE variant_id = $1 AND action = 'share' ORDER BY id",
    )
    .bind(gift)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            ("share".into(), "Asha".into(), Some(0.3), None, None),
            ("share".into(), "Ritwik".into(), Some(0.1), Some(0.3), Some("too much too soon".into())),
        ]
    );
}

#[sqlx::test(migrations = false)]
async fn a_share_edit_shows_in_recent_decisions(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    let gift = variant_id(&pool, "gift").await;
    edit(&app, gift, json!({ "actor": "Asha", "traffic_share": 0.3 })).await;
    let q = api_get(&app, "/api/sites/demo/approvals").await.json();
    let r = &q["recent"][0];
    assert_eq!((r["action"].as_str(), r["variant"].as_str(), r["actor"].as_str()), (Some("share"), Some("gift"), Some("Asha")), "{q}");
}

#[sqlx::test(migrations = false)]
async fn an_mcp_token_cannot_edit_a_share(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let gift = variant_id(&pool, "gift").await;
    let r = send(
        &app,
        Request::post(format!("/api/variants/{gift}/traffic-share"))
            .header("authorization", format!("Bearer {t}"))
            .header("content-type", "application/json")
            .body(Body::from(json!({ "actor": "agent", "traffic_share": 0.5 }).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{}", r.text);
    assert_eq!(share_of(&pool, "gift").await, None);
}

/// The new share gates the very next decide: a visitor outside it never gets gift.
#[sqlx::test(migrations = false)]
async fn decide_uses_the_edited_share(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    let gift = variant_id(&pool, "gift").await;
    let subjects: Vec<String> = (0..400).map(|i| format!("v{i}")).collect();
    let outside: Vec<&String> = subjects.iter().filter(|s| !pratibimba::hash::within_share(gift, s, 0.05)).take(40).collect();
    // Uncapped (the site file's NULL), gift is served to some of these visitors.
    let mut before = 0;
    for s in &outside {
        if post(&app, "/decide", json!({ "site": "demo", "slot": "hero", "subject": format!("pre-{s}") })).await.json()["variant"] == "gift" {
            before += 1;
        }
    }
    assert!(before > 0, "the fixture never serves gift, so this test could not fail");
    assert_eq!(edit(&app, gift, json!({ "actor": "owner", "traffic_share": 0.05 })).await.status, StatusCode::OK);
    for s in &outside {
        let d = post(&app, "/decide", json!({ "site": "demo", "slot": "hero", "subject": s })).await.json();
        assert_ne!(d["variant"], "gift", "a visitor outside the edited 5% share got gift: {d}");
    }
}

// ---- the agent's proposal --------------------------------------------------------------------

#[sqlx::test(migrations = false)]
async fn update_slot_only_proposes_a_share(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let r = data(&propose(&app, &t, json!({ "traffic_share": { "variant": "gift", "share": 0.3 } })).await);
    assert_eq!(r["traffic_share"]["status"], "proposed", "{r}");
    assert_eq!(share_of(&pool, "gift").await, None, "an agent changed a live share");
    let (share, agent, reason): (f32, String, String) =
        sqlx::query_as("SELECT traffic_share, agent, reason FROM share_requests").fetch_one(&pool).await.unwrap();
    assert_eq!((share, agent.as_str(), reason.as_str()), (0.3, "Claude Code", "gift is winning; give it more room"));
}

#[sqlx::test(migrations = false)]
async fn a_proposed_share_out_of_range_is_refused(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    for bad in [0.0, 0.6, -0.1] {
        let e = err_text(&propose(&app, &t, json!({ "traffic_share": { "variant": "gift", "share": bad } })).await);
        assert!(e.contains("50%"), "{e}");
    }
    assert_eq!(count(&pool, "SELECT count(*) FROM share_requests").await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_proposed_share_on_the_baseline_is_refused(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let e = err_text(&propose(&app, &t, json!({ "traffic_share": { "variant": "baseline", "share": 0.3 } })).await);
    assert!(e.contains("baseline"), "{e}");
    assert_eq!(count(&pool, "SELECT count(*) FROM share_requests").await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_proposed_share_on_a_draft_is_refused(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    insert_variant(&pool, "spring", "draft").await;
    let e = err_text(&propose(&app, &t, json!({ "traffic_share": { "variant": "spring", "share": 0.3 } })).await);
    assert!(e.contains("draft"), "{e}");
    assert_eq!(count(&pool, "SELECT count(*) FROM share_requests").await, 0);
}

/// A live slot's settings stay the owner's: a share proposal does not carry them along.
#[sqlx::test(migrations = false)]
async fn a_share_proposal_with_live_settings_is_refused_whole(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    err_text(&propose(&app, &t, json!({ "metric": "signup", "traffic_share": { "variant": "gift", "share": 0.3 } })).await);
    assert_eq!(count(&pool, "SELECT count(*) FROM share_requests").await, 0);
    let metric: String = sqlx::query_scalar("SELECT metric FROM slots WHERE key = 'hero'").fetch_one(&pool).await.unwrap();
    assert_ne!(metric, "signup");
}

#[sqlx::test(migrations = false)]
async fn one_open_share_proposal_per_variant(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&propose(&app, &t, json!({ "traffic_share": { "variant": "gift", "share": 0.3 } })).await);
    let e = err_text(&propose(&app, &t, json!({ "traffic_share": { "variant": "gift", "share": 0.4 } })).await);
    assert!(e.contains("open"), "{e}");
}

#[sqlx::test(migrations = false)]
async fn a_proposed_share_shows_in_the_approvals_queue(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&propose(&app, &t, json!({ "traffic_share": { "variant": "gift", "share": 0.3 } })).await);
    let q = api_get(&app, "/api/sites/demo/approvals").await.json();
    let r = &q["share_requests"][0];
    assert_eq!((r["slot"].as_str(), r["variant"].as_str(), r["agent"]["name"].as_str()), (Some("hero"), Some("gift"), Some("Claude Code")), "{q}");
    assert_eq!(r["traffic_share"].as_f64().map(|v| v as f32), Some(0.3));
    assert_eq!(r["current"], Value::Null);
}

#[sqlx::test(migrations = false)]
async fn list_variants_shows_the_open_share_proposal(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&propose(&app, &t, json!({ "traffic_share": { "variant": "gift", "share": 0.3 } })).await);
    let v = data(&call(&app, &t, "list_variants", json!({ "agent": "a", "justification": "j", "slot": "hero" })).await);
    let gift = v["variants"].as_array().unwrap().iter().find(|v| v["key"] == "gift").cloned().unwrap();
    assert_eq!(gift["share_request"]["status"], "pending", "{gift}");
}

#[sqlx::test(migrations = false)]
async fn confirming_a_share_proposal_applies_and_audits_it(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&propose(&app, &t, json!({ "traffic_share": { "variant": "gift", "share": 0.3 } })).await);
    let id = open_request(&pool).await;
    let r = api_post(&app, &format!("/api/share-requests/{id}/confirm"), json!({ "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(share_of(&pool, "gift").await, Some(0.3));
    let (status, by): (String, Option<String>) =
        sqlx::query_as("SELECT status, decided_by FROM share_requests WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!((status.as_str(), by.as_deref()), ("confirmed", Some("owner")));
    let (actor, share): (String, Option<f32>) =
        sqlx::query_as("SELECT actor, traffic_share FROM approvals WHERE action = 'share'").fetch_one(&pool).await.unwrap();
    assert_eq!((actor.as_str(), share), ("owner", Some(0.3)));
}

#[sqlx::test(migrations = false)]
async fn declining_a_share_proposal_keeps_the_share(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&propose(&app, &t, json!({ "traffic_share": { "variant": "gift", "share": 0.3 } })).await);
    let id = open_request(&pool).await;
    let r = api_post(&app, &format!("/api/share-requests/{id}/decline"), json!({ "actor": "owner", "comment": "not yet" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(share_of(&pool, "gift").await, None);
    let s: String = sqlx::query_scalar("SELECT status FROM share_requests WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(s, "declined");
    assert_eq!(count(&pool, "SELECT count(*) FROM approvals WHERE action = 'share'").await, 0);
    let c: String = sqlx::query_scalar("SELECT comment FROM approvals WHERE action = 'comment'").fetch_one(&pool).await.unwrap();
    assert_eq!(c, "not yet", "the reason goes back to the agent on the variant's thread");
}

#[sqlx::test(migrations = false)]
async fn a_decided_share_proposal_cannot_be_confirmed_again(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&propose(&app, &t, json!({ "traffic_share": { "variant": "gift", "share": 0.3 } })).await);
    let id = open_request(&pool).await;
    api_post(&app, &format!("/api/share-requests/{id}/decline"), json!({ "actor": "owner" })).await;
    let r = api_post(&app, &format!("/api/share-requests/{id}/confirm"), json!({ "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text);
    assert_eq!(share_of(&pool, "gift").await, None);
}

/// A proposal confirmed after its variant stopped being live changes nothing.
#[sqlx::test(migrations = false)]
async fn a_share_proposal_for_a_variant_retired_since_cannot_be_confirmed(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&propose(&app, &t, json!({ "traffic_share": { "variant": "gift", "share": 0.3 } })).await);
    sqlx::query("UPDATE variants SET status = 'retired' WHERE key = 'gift'").execute(&pool).await.unwrap();
    let id = open_request(&pool).await;
    let r = api_post(&app, &format!("/api/share-requests/{id}/confirm"), json!({ "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text);
    assert_eq!(share_of(&pool, "gift").await, None);
}

#[sqlx::test(migrations = false)]
async fn an_mcp_token_cannot_confirm_a_share_proposal(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&propose(&app, &t, json!({ "traffic_share": { "variant": "gift", "share": 0.3 } })).await);
    let id = open_request(&pool).await;
    let r = send(
        &app,
        Request::post(format!("/api/share-requests/{id}/confirm"))
            .header("authorization", format!("Bearer {t}"))
            .header("content-type", "application/json")
            .body(Body::from(json!({ "actor": "agent" }).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{}", r.text);
    assert_eq!(share_of(&pool, "gift").await, None);
}

// ---- re-applying the site file (bug fix 2026-10-16) ------------------------------------------
//
// The rule: a variant's `traffic_share` in the site file is applied only when it differs from
// the value the file stated at the last apply. Omitted, or unchanged since the last apply, the
// stored share stands, so an owner's edit survives a re-apply. The baseline never has one.

fn demo_with_share(share: Option<f32>) -> pratibimba::site_file::SiteFile {
    let mut gift = json!({ "key": "gift", "html": "<h1>Gift sets</h1>" });
    if let Some(s) = share {
        gift["traffic_share"] = json!(s);
    }
    file_from(json!({
        "site": "demo", "origin": "http://localhost:5173",
        "slots": [{ "key": "hero", "selector": "#hero", "metric": "click",
                    "variants": [ { "key": "baseline", "baseline": true }, gift ] }]
    }))
}

async fn apply(pool: &PgPool, file: &pratibimba::site_file::SiteFile) {
    pratibimba::site_file::apply(pool, file).await.unwrap();
}

async fn share_rows(pool: &PgPool, id: i64) -> Vec<(String, Option<f32>, Option<f32>)> {
    sqlx::query_as("SELECT actor, traffic_share, previous_share FROM approvals WHERE variant_id = $1 AND action = 'share' ORDER BY id")
        .bind(id)
        .fetch_all(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
async fn reapplying_the_site_file_keeps_an_edited_share(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    let gift = variant_id(&pool, "gift").await;
    edit(&app, gift, json!({ "actor": "owner", "traffic_share": 0.2 })).await;
    apply(&pool, &demo_file()).await;
    assert_eq!(share_of(&pool, "gift").await, Some(0.2));
}

#[sqlx::test(migrations = false)]
async fn a_share_the_file_states_is_applied_and_audited(pool: PgPool) {
    let (_app, _) = setup(&pool).await;
    let gift = variant_id(&pool, "gift").await;
    apply(&pool, &demo_with_share(Some(0.3))).await;
    assert_eq!(share_of(&pool, "gift").await, Some(0.3));
    assert_eq!(share_rows(&pool, gift).await, vec![("cli:apply".to_string(), Some(0.3), None)]);
}

#[sqlx::test(migrations = false)]
async fn a_new_variant_takes_the_files_share(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    apply(&pool, &demo_with_share(Some(0.25))).await;
    assert_eq!(share_of(&pool, "gift").await, Some(0.25));
}

#[sqlx::test(migrations = false)]
async fn an_unchanged_file_share_does_not_undo_an_edit(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    let gift = variant_id(&pool, "gift").await;
    apply(&pool, &demo_with_share(Some(0.3))).await;
    edit(&app, gift, json!({ "actor": "owner", "traffic_share": 0.2 })).await;
    apply(&pool, &demo_with_share(Some(0.3))).await;
    assert_eq!(share_of(&pool, "gift").await, Some(0.2));
    assert_eq!(share_rows(&pool, gift).await.len(), 2, "the unchanged re-apply writes no share row");
}

#[sqlx::test(migrations = false)]
async fn a_changed_file_share_wins_over_an_edit(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    let gift = variant_id(&pool, "gift").await;
    apply(&pool, &demo_with_share(Some(0.3))).await;
    edit(&app, gift, json!({ "actor": "owner", "traffic_share": 0.2 })).await;
    apply(&pool, &demo_with_share(Some(0.4))).await;
    assert_eq!(share_of(&pool, "gift").await, Some(0.4));
    assert_eq!(share_rows(&pool, gift).await.last().unwrap(), &("cli:apply".to_string(), Some(0.4), Some(0.2)));
}

#[sqlx::test(migrations = false)]
async fn dropping_the_files_share_keeps_the_current_one(pool: PgPool) {
    let (_app, _) = setup(&pool).await;
    apply(&pool, &demo_with_share(Some(0.3))).await;
    apply(&pool, &demo_file()).await;
    assert_eq!(share_of(&pool, "gift").await, Some(0.3));
    // Stated again after being dropped, it counts as a change and applies.
    sqlx::query("UPDATE variants SET traffic_share = 0.1 WHERE key = 'gift'").execute(&pool).await.unwrap();
    apply(&pool, &demo_with_share(Some(0.3))).await;
    assert_eq!(share_of(&pool, "gift").await, Some(0.3));
}

#[test]
fn the_site_file_refuses_a_share_out_of_range_or_on_the_baseline() {
    for bad in [0.0, 0.51, -0.1] {
        assert!(demo_with_share(Some(bad)).validate().is_err(), "{bad} accepted");
    }
    assert!(demo_with_share(Some(0.5)).validate().is_ok());
    let on_baseline: Result<pratibimba::site_file::SiteFile, _> = serde_json::from_value(json!({
        "site": "demo", "origin": "http://localhost:5173",
        "slots": [{ "key": "hero", "selector": "#hero",
                    "variants": [ { "key": "baseline", "baseline": true, "traffic_share": 0.2 },
                                  { "key": "gift", "html": "<h1>Gift sets</h1>" } ] }]
    }));
    assert!(on_baseline.unwrap().validate().is_err());
}

#[sqlx::test(migrations = false)]
async fn a_variant_made_the_baseline_loses_its_share(pool: PgPool) {
    let (_app, _) = setup(&pool).await;
    apply(&pool, &demo_with_share(Some(0.3))).await;
    apply(&pool, &file_from(json!({
        "site": "demo", "origin": "http://localhost:5173",
        "slots": [{ "key": "hero", "selector": "#hero", "metric": "click",
                    "variants": [ { "key": "baseline", "html": "<h1>Old</h1>" }, { "key": "gift", "baseline": true } ] }]
    }))).await;
    assert_eq!(share_of(&pool, "gift").await, None);
}
