//! The MCP slot tools and retirement, under the same rule as every MCP write: an agent's work
//! lands as a draft or a proposal, and a person makes it live.
//!
//! - `create_slot` makes a slot whose baseline is a DRAFT. It is in no manifest and decides
//!   nothing until a person approves that baseline, which activates the slot.
//! - `update_slot` edits a slot only while it is still a draft. A live slot's settings belong
//!   to the owner (site file or dashboard).
//! - `retire_variant` only PROPOSES (backend.md leaves the retire permission open; the floor is
//!   drafts-only). The variant keeps serving until a person confirms in the dashboard API.

mod common;
use axum::{Router, body::Body, http::Request};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn call(app: &Router, token: &str, tool: &str, args: Value) -> Value {
    let body = json!({ "jsonrpc": "2.0", "id": 77, "method": "tools/call", "params": { "name": tool, "arguments": args } });
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

fn why(extra: Value) -> Value {
    let mut a = json!({ "agent": "Claude Code", "justification": "the promo banner needs a slot" });
    a.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    a
}

async fn setup(pool: &PgPool) -> (Router, String) {
    setup_learning(pool).await;
    let t = pratibimba::mcp::issue_token(pool, "demo", "test agent").await.unwrap();
    (admin_app(pool), t)
}

/// A draft slot `promo` (#promo, metric `signup`), created over MCP.
async fn promo(app: &Router, t: &str) -> Value {
    data(&call(app, t, "create_slot", why(json!({ "key": "promo", "selector": "#promo", "metric": "signup" }))).await)
}

async fn baseline_of(pool: &PgPool, slot: &str) -> (i64, String) {
    sqlx::query_as("SELECT v.id, v.status FROM variants v JOIN slots s ON s.id = v.slot_id WHERE s.key = $1 AND v.is_baseline")
        .bind(slot)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn approve(app: &Router, id: i64) -> Resp {
    api_post(app, &format!("/api/variants/{id}/approve"), json!({ "approver": "owner", "traffic_share": 0.5 })).await
}

async fn status_of(pool: &PgPool, key: &str) -> String {
    sqlx::query_scalar("SELECT status FROM variants WHERE key = $1").bind(key).fetch_one(pool).await.unwrap()
}

// ---- create_slot ------------------------------------------------------------------------------

#[sqlx::test(migrations = false)]
async fn create_slot_lands_as_a_draft_slot(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let r = promo(&app, &t).await;
    assert_eq!(r["status"], "draft", "{r}");
    let (_, status) = baseline_of(&pool, "promo").await;
    assert_eq!(status, "draft");
    let (by, selector, metric, linked): (Option<String>, String, String, bool) =
        sqlx::query_as("SELECT created_by, selector, metric, mcp_call_id = $1 FROM slots WHERE key = 'promo'")
            .bind(r["audit"]["id"].as_i64().unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((by.as_deref(), selector.as_str(), metric.as_str(), linked), (Some("mcp:Claude Code"), "#promo", "signup", true));
}

#[sqlx::test(migrations = false)]
async fn a_draft_slot_is_in_no_manifest_and_decides_nothing(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    promo(&app, &t).await;
    let m = get(&app, "/manifest/demo").await.json();
    assert!(m["slots"].as_array().unwrap().iter().all(|s| s["key"] != "promo"), "{m}");
    let r = post(&app, "/decide", json!({ "site": "demo", "slot": "promo", "subject": "v1" })).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn approving_the_baseline_activates_the_slot(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    promo(&app, &t).await;
    let (id, _) = baseline_of(&pool, "promo").await;
    let r = approve(&app, id).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let m = get(&app, "/manifest/demo").await.json();
    assert!(m["slots"].as_array().unwrap().iter().any(|s| s["key"] == "promo"), "{m}");
    let d = post(&app, "/decide", json!({ "site": "demo", "slot": "promo", "subject": "v1" })).await;
    assert_eq!(d.status, StatusCode::OK, "{}", d.text);
    assert_eq!(d.json()["variant"], "baseline");
}

/// A baseline is the page's own content for everyone: approving it never caps its share.
#[sqlx::test(migrations = false)]
async fn an_activated_baseline_has_no_traffic_cap(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    promo(&app, &t).await;
    let (id, _) = baseline_of(&pool, "promo").await;
    approve(&app, id).await;
    let share: Option<f32> = sqlx::query_scalar("SELECT traffic_share FROM variants WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(share, None);
}

#[sqlx::test(migrations = false)]
async fn the_approval_card_says_approving_activates_the_slot(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    promo(&app, &t).await;
    let q = api_get(&app, "/api/sites/demo/approvals").await.json();
    let card = q["drafts"].as_array().unwrap().iter().find(|c| c["slot"] == "promo").cloned().unwrap_or_else(|| panic!("{q}"));
    assert_eq!(card["activates_slot"], json!(true), "{card}");
    assert_eq!(card["agent"]["tool"], "create_slot", "{card}");
}

#[sqlx::test(migrations = false)]
async fn an_ordinary_draft_card_does_not_activate_a_slot(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&call(&app, &t, "propose_variant", why(json!({ "slot": "hero", "key": "ship", "html": "<h2>Ship today</h2>" }))).await);
    let q = api_get(&app, "/api/sites/demo/approvals").await.json();
    let card = q["drafts"].as_array().unwrap().iter().find(|c| c["key"] == "ship").cloned().unwrap();
    assert_eq!(card["activates_slot"], json!(false), "{card}");
}

#[sqlx::test(migrations = false)]
async fn create_slot_refuses_an_existing_key(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let e = err_text(&call(&app, &t, "create_slot", why(json!({ "key": "hero", "selector": "#other" }))).await);
    assert!(e.contains("already"), "{e}");
    let sel: String = sqlx::query_scalar("SELECT selector FROM slots WHERE key = 'hero'").fetch_one(&pool).await.unwrap();
    assert_eq!(sel, "#hero");
}

#[sqlx::test(migrations = false)]
async fn create_slot_refuses_a_key_that_is_not_a_slug(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    err_text(&call(&app, &t, "create_slot", why(json!({ "key": "Promo Banner", "selector": "#promo" }))).await);
}

#[sqlx::test(migrations = false)]
async fn create_slot_refuses_a_metric_that_is_not_an_event_name(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    err_text(&call(&app, &t, "create_slot", why(json!({ "key": "promo", "selector": "#promo", "metric": "sign up!" }))).await);
}

#[sqlx::test(migrations = false)]
async fn create_slot_refuses_an_empty_selector(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    err_text(&call(&app, &t, "create_slot", why(json!({ "key": "promo", "selector": "  " }))).await);
}

// ---- update_slot ------------------------------------------------------------------------------

#[sqlx::test(migrations = false)]
async fn update_slot_changes_a_draft_slot(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    promo(&app, &t).await;
    let r = data(&call(&app, &t, "update_slot", why(json!({ "slot": "promo", "selector": "#promo-banner", "attribution_window_secs": 600 }))).await);
    assert_eq!(r["selector"], "#promo-banner", "{r}");
    let (sel, w, metric): (String, i32, String) =
        sqlx::query_as("SELECT selector, attribution_window_secs, metric FROM slots WHERE key = 'promo'").fetch_one(&pool).await.unwrap();
    assert_eq!((sel.as_str(), w, metric.as_str()), ("#promo-banner", 600, "signup"));
}

#[sqlx::test(migrations = false)]
async fn update_slot_refuses_a_live_slot(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let e = err_text(&call(&app, &t, "update_slot", why(json!({ "slot": "hero", "selector": "#elsewhere" }))).await);
    assert!(e.contains("live"), "{e}");
    let sel: String = sqlx::query_scalar("SELECT selector FROM slots WHERE key = 'hero'").fetch_one(&pool).await.unwrap();
    assert_eq!(sel, "#hero");
}

/// Once a person activates it, the agent's own slot is live too.
#[sqlx::test(migrations = false)]
async fn update_slot_refuses_an_activated_slot(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    promo(&app, &t).await;
    approve(&app, baseline_of(&pool, "promo").await.0).await;
    err_text(&call(&app, &t, "update_slot", why(json!({ "slot": "promo", "metric": "click" }))).await);
}

#[sqlx::test(migrations = false)]
async fn update_slot_refuses_a_field_it_does_not_own(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    promo(&app, &t).await;
    // A valid field rides along, so only the unknown one can make the call fail.
    err_text(&call(&app, &t, "update_slot", why(json!({ "slot": "promo", "selector": "#moved", "kind": "picker" }))).await);
    let sel: String = sqlx::query_scalar("SELECT selector FROM slots WHERE key = 'promo'").fetch_one(&pool).await.unwrap();
    assert_eq!(sel, "#promo");
}

#[sqlx::test(migrations = false)]
async fn update_slot_refuses_a_bad_window(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    promo(&app, &t).await;
    let e = err_text(&call(&app, &t, "update_slot", why(json!({ "slot": "promo", "attribution_window_secs": 0 }))).await);
    assert!(e.contains("attribution_window_secs"), "{e}");
}

/// A variant proposed into a draft slot cannot go live before the slot itself does.
#[sqlx::test(migrations = false)]
async fn a_variant_of_a_draft_slot_waits_for_its_baseline(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    promo(&app, &t).await;
    let id = data(&call(&app, &t, "propose_variant", why(json!({ "slot": "promo", "key": "spring", "html": "<p>Spring sale</p>" }))).await)["id"].as_i64().unwrap();
    let r = approve(&app, id).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text);
    assert_eq!(status_of(&pool, "spring").await, "draft");
}

/// Once the slot is active, its variants are approved as usual.
#[sqlx::test(migrations = false)]
async fn a_variant_of_an_activated_slot_can_be_approved(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    promo(&app, &t).await;
    approve(&app, baseline_of(&pool, "promo").await.0).await;
    let id = data(&call(&app, &t, "propose_variant", why(json!({ "slot": "promo", "key": "spring", "html": "<p>Spring sale</p>" }))).await)["id"].as_i64().unwrap();
    let r = approve(&app, id).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
}

// ---- retire_variant ---------------------------------------------------------------------------

async fn propose_retire(app: &Router, t: &str, variant: &str) -> Value {
    call(app, t, "retire_variant", json!({ "agent": "Claude Code", "justification": "gift has lost for three weeks", "slot": "hero", "variant": variant })).await
}

async fn open_request(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT id FROM retire_requests WHERE status = 'pending'").fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn retire_variant_only_proposes(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let r = data(&propose_retire(&app, &t, "gift").await);
    assert_eq!(r["status"], "proposed", "{r}");
    assert_eq!(status_of(&pool, "gift").await, "approved", "an agent retired a variant");
    let (reason, agent): (String, String) = sqlx::query_as("SELECT reason, agent FROM retire_requests").fetch_one(&pool).await.unwrap();
    assert_eq!((reason.as_str(), agent.as_str()), ("gift has lost for three weeks", "Claude Code"));
}

#[sqlx::test(migrations = false)]
async fn a_proposed_retirement_keeps_serving(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&propose_retire(&app, &t, "gift").await);
    let m = get(&app, "/manifest/demo").await.json();
    let hero = m["slots"].as_array().unwrap().iter().find(|s| s["key"] == "hero").cloned().unwrap();
    assert!(hero["variants"].as_array().unwrap().iter().any(|v| v["key"] == "gift" || v == "gift"), "{hero}");
}

#[sqlx::test(migrations = false)]
async fn a_proposed_retirement_shows_in_the_approvals_queue(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&propose_retire(&app, &t, "gift").await);
    let q = api_get(&app, "/api/sites/demo/approvals").await.json();
    let r = &q["retirements"][0];
    assert_eq!((r["slot"].as_str(), r["variant"].as_str(), r["agent"]["name"].as_str()), (Some("hero"), Some("gift"), Some("Claude Code")), "{q}");
}

#[sqlx::test(migrations = false)]
async fn list_variants_shows_the_open_proposal_to_the_agent(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&propose_retire(&app, &t, "gift").await);
    let v = data(&call(&app, &t, "list_variants", why(json!({ "slot": "hero" }))).await);
    let gift = v["variants"].as_array().unwrap().iter().find(|v| v["key"] == "gift").cloned().unwrap();
    assert_eq!(gift["retire_request"]["status"], "pending", "{gift}");
}

#[sqlx::test(migrations = false)]
async fn confirming_a_retirement_retires_the_variant(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&propose_retire(&app, &t, "gift").await);
    let id = open_request(&pool).await;
    let r = api_post(&app, &format!("/api/retire-requests/{id}/confirm"), json!({ "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(status_of(&pool, "gift").await, "retired");
    let by: Option<String> = sqlx::query_scalar("SELECT decided_by FROM retire_requests WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(by.as_deref(), Some("owner"));
    for i in 0..30 {
        let d = post(&app, "/decide", json!({ "site": "demo", "slot": "hero", "subject": format!("after{i}") })).await.json();
        assert_eq!(d["variant"], "baseline", "a retired variant was served: {d}");
    }
}

#[sqlx::test(migrations = false)]
async fn declining_a_retirement_keeps_the_variant(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&propose_retire(&app, &t, "gift").await);
    let id = open_request(&pool).await;
    let r = api_post(&app, &format!("/api/retire-requests/{id}/decline"), json!({ "actor": "owner", "comment": "keep it through the holidays" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(status_of(&pool, "gift").await, "approved");
    let s: String = sqlx::query_scalar("SELECT status FROM retire_requests WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(s, "declined");
}

#[sqlx::test(migrations = false)]
async fn a_decided_proposal_cannot_be_confirmed_again(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&propose_retire(&app, &t, "gift").await);
    let id = open_request(&pool).await;
    api_post(&app, &format!("/api/retire-requests/{id}/decline"), json!({ "actor": "owner", "comment": "no" })).await;
    let r = api_post(&app, &format!("/api/retire-requests/{id}/confirm"), json!({ "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text);
    assert_eq!(status_of(&pool, "gift").await, "approved");
}

#[sqlx::test(migrations = false)]
async fn an_mcp_token_cannot_confirm_a_retirement(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&propose_retire(&app, &t, "gift").await);
    let id = open_request(&pool).await;
    let r = send(
        &app,
        Request::post(format!("/api/retire-requests/{id}/confirm"))
            .header("authorization", format!("Bearer {t}"))
            .header("content-type", "application/json")
            .body(Body::from(json!({ "actor": "agent" }).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{}", r.text);
    assert_eq!(status_of(&pool, "gift").await, "approved");
}

#[sqlx::test(migrations = false)]
async fn retire_variant_refuses_the_baseline(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let e = err_text(&propose_retire(&app, &t, "baseline").await);
    assert!(e.contains("baseline"), "{e}");
}

#[sqlx::test(migrations = false)]
async fn retire_variant_refuses_a_variant_that_is_not_live(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&call(&app, &t, "propose_variant", why(json!({ "slot": "hero", "key": "ship", "html": "<h2>Ship today</h2>" }))).await);
    err_text(&propose_retire(&app, &t, "ship").await);
}

#[sqlx::test(migrations = false)]
async fn a_second_open_proposal_is_refused(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&propose_retire(&app, &t, "gift").await);
    let e = err_text(&propose_retire(&app, &t, "gift").await);
    assert!(e.contains("already"), "{e}");
}

#[sqlx::test(migrations = false)]
async fn retire_variant_is_audited_with_its_justification(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let r = data(&propose_retire(&app, &t, "gift").await);
    let linked: bool = sqlx::query_scalar("SELECT mcp_call_id = $1 FROM retire_requests")
        .bind(r["audit"]["id"].as_i64().unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(linked);
    let (tool, outcome): (String, String) =
        sqlx::query_as("SELECT tool, outcome FROM mcp_calls WHERE id = $1").bind(r["audit"]["id"].as_i64().unwrap()).fetch_one(&pool).await.unwrap();
    assert_eq!((tool.as_str(), outcome.as_str()), ("retire_variant", "ok"));
}

// ---- create_slot kinds: copy (F4) and sorted (F2) -------------------------------------------

fn headline_fields() -> Value {
    json!({ "fields": [{ "name": "headline", "max": 40, "required": true }, { "name": "cta_label", "max": 20 }] })
}

async fn create(app: &Router, t: &str, args: Value) -> Value {
    call(app, t, "create_slot", why(args)).await
}

async fn kind_cols(pool: &PgPool, key: &str) -> (String, Option<Value>, Option<Value>) {
    sqlx::query_as("SELECT kind, copy, sorted FROM slots WHERE key = $1").bind(key).fetch_one(pool).await.unwrap()
}

async fn in_manifest(app: &Router, key: &str) -> bool {
    let m = get(app, "/manifest/demo").await.json();
    m["slots"].as_array().unwrap().iter().any(|s| s["key"] == key)
}

#[sqlx::test(migrations = false)]
async fn create_slot_makes_a_draft_copy_slot_with_its_fields(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let r = data(&create(&app, &t, json!({ "key": "banner", "selector": "#banner", "kind": "copy", "copy": headline_fields() })).await);
    assert_eq!((r["kind"].as_str(), r["status"].as_str()), (Some("copy"), Some("draft")), "{r}");
    let (kind, copy, sorted) = kind_cols(&pool, "banner").await;
    assert_eq!((kind.as_str(), sorted), ("copy", None));
    assert_eq!(copy.unwrap(), json!({ "fields": [{ "name": "headline", "max": 40, "required": true }, { "name": "cta_label", "max": 20, "required": false }] }));
    assert_eq!(baseline_of(&pool, "banner").await.1, "draft");
    assert!(!in_manifest(&app, "banner").await);
    // The declared fields are the ones propose_copy is held to.
    let long = "x".repeat(41);
    err_text(&call(&app, &t, "propose_copy", why(json!({ "slot": "banner", "key": "long", "copy": { "headline": long } }))).await);
    data(&call(&app, &t, "propose_copy", why(json!({ "slot": "banner", "key": "short", "copy": { "headline": "Ship today" } }))).await);
}

#[sqlx::test(migrations = false)]
async fn approving_a_copy_slots_baseline_activates_it(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&create(&app, &t, json!({ "key": "banner", "selector": "#banner", "kind": "copy", "copy": headline_fields() })).await);
    let (id, _) = baseline_of(&pool, "banner").await;
    assert_eq!(approve(&app, id).await.status, StatusCode::OK);
    assert!(in_manifest(&app, "banner").await);
}

#[sqlx::test(migrations = false)]
async fn create_slot_makes_a_draft_sorted_slot_in_the_default_order(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let r = data(&create(&app, &t, json!({ "key": "grid", "selector": "#grid", "kind": "sorted", "items": ["tea", "mug", "pot"] })).await);
    assert_eq!((r["kind"].as_str(), r["status"].as_str()), (Some("sorted"), Some("draft")), "{r}");
    let (kind, copy, sorted) = kind_cols(&pool, "grid").await;
    assert_eq!((kind.as_str(), copy), ("sorted", None));
    assert_eq!(sorted.unwrap(), json!({ "visible": 3, "order": ["tea", "mug", "pot"] }));
    let items: Vec<(String, bool, String)> = sqlx::query_as(
        "SELECT v.key, v.is_baseline, v.status FROM variants v JOIN slots s ON s.id = v.slot_id WHERE s.key = 'grid' ORDER BY v.id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let want: Vec<(String, bool, String)> =
        [("tea", true), ("mug", false), ("pot", false)].iter().map(|(k, b)| (k.to_string(), *b, "draft".to_string())).collect();
    assert_eq!(items, want, "the head of the default order is the baseline; every item is a draft");
    assert!(!in_manifest(&app, "grid").await);
}

#[sqlx::test(migrations = false)]
async fn create_slot_keeps_a_sorted_slots_visible(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&create(&app, &t, json!({ "key": "grid", "selector": "#grid", "kind": "sorted", "items": ["a", "b", "c"], "sorted": { "visible": 2 } })).await);
    assert_eq!(kind_cols(&pool, "grid").await.2.unwrap()["visible"], 2);
}

#[sqlx::test(migrations = false)]
async fn approving_a_sorted_slots_head_activates_it(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&create(&app, &t, json!({ "key": "grid", "selector": "#grid", "kind": "sorted", "items": ["tea", "mug"] })).await);
    let (id, _) = baseline_of(&pool, "grid").await;
    let r = approve(&app, id).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert!(in_manifest(&app, "grid").await);
}

/// One refusal per rule, each shaped so only that rule can explain the error.
async fn refused(args: Value, needle: &str, pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let mut a = json!({ "key": "promo", "selector": "#promo" });
    a.as_object_mut().unwrap().extend(args.as_object().unwrap().clone());
    let e = err_text(&create(&app, &t, a).await);
    assert!(e.contains(needle), "wanted {needle:?} in {e:?}");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM slots WHERE key = 'promo'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0, "a refused slot is not stored");
}

#[sqlx::test(migrations = false)]
async fn create_slot_refuses_a_copy_slot_without_fields(pool: PgPool) {
    refused(json!({ "kind": "copy" }), "needs copy.fields", pool).await;
}
#[sqlx::test(migrations = false)]
async fn create_slot_refuses_a_copy_slot_with_no_fields(pool: PgPool) {
    refused(json!({ "kind": "copy", "copy": { "fields": [] } }), "1-8 fields", pool).await;
}
#[sqlx::test(migrations = false)]
async fn create_slot_refuses_a_copy_field_over_the_max_length(pool: PgPool) {
    refused(json!({ "kind": "copy", "copy": { "fields": [{ "name": "headline", "max": 501 }] } }), "max must be 1 to 500", pool).await;
}
#[sqlx::test(migrations = false)]
async fn create_slot_refuses_a_repeated_copy_field(pool: PgPool) {
    let f = json!({ "name": "headline", "max": 40 });
    refused(json!({ "kind": "copy", "copy": { "fields": [f, f] } }), "invalid or repeated", pool).await;
}
#[sqlx::test(migrations = false)]
async fn create_slot_refuses_copy_fields_on_a_swap_slot(pool: PgPool) {
    refused(json!({ "copy": headline_fields() }), "slot takes copy fields", pool).await;
}
#[sqlx::test(migrations = false)]
async fn create_slot_refuses_a_sorted_slot_without_items(pool: PgPool) {
    refused(json!({ "kind": "sorted" }), "needs 2-50 items", pool).await;
}
#[sqlx::test(migrations = false)]
async fn create_slot_refuses_a_sorted_slot_with_one_item(pool: PgPool) {
    refused(json!({ "kind": "sorted", "items": ["tea"] }), "needs 2-50 items", pool).await;
}
#[sqlx::test(migrations = false)]
async fn create_slot_refuses_a_repeated_item(pool: PgPool) {
    refused(json!({ "kind": "sorted", "items": ["tea", "tea"] }), "invalid or repeated", pool).await;
}
#[sqlx::test(migrations = false)]
async fn create_slot_refuses_an_item_that_is_not_a_slug(pool: PgPool) {
    refused(json!({ "kind": "sorted", "items": ["tea", "Big Mug"] }), "invalid or repeated", pool).await;
}
#[sqlx::test(migrations = false)]
async fn create_slot_refuses_a_visible_beyond_the_items(pool: PgPool) {
    refused(json!({ "kind": "sorted", "items": ["a", "b"], "sorted": { "visible": 3 } }), "sorted.visible", pool).await;
}
#[sqlx::test(migrations = false)]
async fn create_slot_refuses_items_on_a_copy_slot(pool: PgPool) {
    refused(json!({ "kind": "copy", "copy": headline_fields(), "items": ["a", "b"] }), "slot takes items", pool).await;
}
#[sqlx::test(migrations = false)]
async fn create_slot_refuses_sorted_options_on_a_swap_slot(pool: PgPool) {
    refused(json!({ "sorted": { "visible": 2 } }), "slot takes sorted options", pool).await;
}
#[sqlx::test(migrations = false)]
async fn create_slot_refuses_a_picker_slot(pool: PgPool) {
    refused(json!({ "kind": "picker" }), "kind must be swap, copy or sorted", pool).await;
}
