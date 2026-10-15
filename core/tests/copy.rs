//! F4: the `copy` slot kind. Variants are plain-text fields with declared max lengths; every door
//! (site file, agent, person, approval) checks them; a decision returns them as `copy`.

mod common;
use axum::{Router, body::Body, http::Request};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

fn copy_file(variants: Value) -> Value {
    json!({ "site": "demo", "origin": "http://localhost:5173", "slots": [{
        "key": "hero", "selector": "#hero", "kind": "copy",
        "copy": { "fields": [ { "name": "headline", "max": 20, "required": true }, { "name": "cta_label", "max": 10 } ] },
        "variants": variants }] })
}

fn base_and(extra: Value) -> Value {
    json!([{ "key": "own", "baseline": true }, extra])
}

async fn setup(pool: &PgPool) {
    pratibimba::db::migrate(pool).await.unwrap();
    let f = copy_file(base_and(json!({ "key": "short", "copy": { "headline": "Fast & free", "cta_label": "Shop" } })));
    pratibimba::site_file::apply(pool, &file_from(f)).await.unwrap();
    set_holdout(pool, 0.0).await;
}

async fn apply_err(pool: &PgPool, f: Value) -> String {
    pratibimba::db::migrate(pool).await.unwrap();
    pratibimba::site_file::apply(pool, &file_from(f)).await.unwrap_err().to_string()
}

// ---- the site file ----

#[sqlx::test(migrations = false)]
async fn a_headline_over_its_max_is_refused(pool: PgPool) {
    // 21 characters against a max of 20; 20 multibyte characters are within it (chars, not bytes).
    let e = apply_err(&pool, copy_file(base_and(json!({ "key": "long", "copy": { "headline": "a".repeat(21) } })))).await;
    assert!(e.contains("max is 20"), "{e}");
    pratibimba::site_file::apply(&pool, &file_from(copy_file(base_and(json!({ "key": "wide", "copy": { "headline": "é".repeat(20) } }))))).await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn html_in_a_field_is_refused(pool: PgPool) {
    let e = apply_err(&pool, copy_file(base_and(json!({ "key": "tag", "copy": { "headline": "<b>Sale</b>" } })))).await;
    assert!(e.contains("no HTML"), "{e}");
}

#[sqlx::test(migrations = false)]
async fn a_newline_in_a_field_is_refused(pool: PgPool) {
    let e = apply_err(&pool, copy_file(base_and(json!({ "key": "nl", "copy": { "headline": "two\nlines" } })))).await;
    assert!(e.contains("one line"), "{e}");
}

#[sqlx::test(migrations = false)]
async fn a_missing_or_blank_required_field_is_refused(pool: PgPool) {
    let e = apply_err(&pool, copy_file(base_and(json!({ "key": "nohead", "copy": { "cta_label": "Go" } })))).await;
    assert!(e.contains("\"headline\" is required"), "{e}");
    let e = apply_err(&pool, copy_file(base_and(json!({ "key": "blank", "copy": { "headline": "   " } })))).await;
    assert!(e.contains("\"headline\" is required"), "{e}");
}

#[sqlx::test(migrations = false)]
async fn an_undeclared_field_and_html_on_a_copy_slot_are_refused(pool: PgPool) {
    let e = apply_err(&pool, copy_file(base_and(json!({ "key": "x", "copy": { "headline": "Hi", "body": "no" } })))).await;
    assert!(e.contains("not a field"), "{e}");
    let e = apply_err(&pool, copy_file(base_and(json!({ "key": "x", "html": "<h2>Hi</h2>" })))).await;
    assert!(e.contains("not html"), "{e}");
}

#[sqlx::test(migrations = false)]
async fn copy_fields_on_a_swap_slot_are_refused(pool: PgPool) {
    let mut f = copy_file(base_and(json!({ "key": "x", "copy": { "headline": "Hi" } })));
    f["slots"][0]["kind"] = json!("swap");
    let e = apply_err(&pool, f).await;
    assert!(e.contains("only a kind \"copy\" slot"), "{e}");
}

// ---- deciding ----

#[sqlx::test(migrations = false)]
async fn a_copy_decision_returns_fields_and_never_a_payload(pool: PgPool) {
    setup(&pool).await;
    let app = app(&pool);
    let mut seen = std::collections::HashSet::new();
    for i in 0..40 {
        let r = post(&app, "/decide", json!({ "site": "demo", "slot": "hero", "subject": format!("s{i}") })).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text);
        let d = r.json();
        assert_eq!(d["payload"], Value::Null, "{d}");
        match d["variant"].as_str().unwrap() {
            "short" => assert_eq!(d["copy"], json!({ "headline": "Fast & free", "cta_label": "Shop" })),
            "own" => assert_eq!(d["copy"], Value::Null),
            v => panic!("{v}"),
        }
        seen.insert(d["variant"].as_str().unwrap().to_string());
        if i == 0 {
            let id = uuid::Uuid::parse_str(d["decision_id"].as_str().unwrap()).unwrap();
            let detail = pratibimba::explain::detail(&pool, id, chrono::Utc::now()).await.unwrap().unwrap();
            assert_eq!(detail.copy, d["copy"].as_object().map(|_| d["copy"].clone()), "explain shows the text");
        }
    }
    assert_eq!(seen.len(), 2, "both arms served in cold start: {seen:?}");
}

#[sqlx::test(migrations = false)]
async fn the_edge_snapshot_leaves_copy_slots_out(pool: PgPool) {
    setup(&pool).await;
    let c = pratibimba::edge::content(&pool, "demo").await.unwrap().unwrap();
    assert!(c["slots"].get("hero").is_none(), "{}", c["slots"]);
}

#[sqlx::test(migrations = false)]
async fn the_manifest_marks_a_copy_slot(pool: PgPool) {
    setup(&pool).await;
    let m = get(&app(&pool), "/manifest/demo").await.json();
    assert_eq!(m["slots"][0]["kind"], "copy", "{m}");
}

// ---- agents ----

async fn rpc_call(app: &Router, token: &str, tool: &str, args: Value) -> Value {
    let mut a = json!({ "agent": "Claude Code", "justification": "testing copy" });
    a.as_object_mut().unwrap().extend(args.as_object().unwrap().clone());
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": tool, "arguments": a } });
    let req = Request::post("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("authorization", format!("Bearer {token}"));
    send(app, req.body(Body::from(body.to_string())).unwrap()).await.json()["result"].clone()
}

fn tool_text(r: &Value) -> (bool, String) {
    (r["isError"] == json!(true), r["content"][0]["text"].as_str().unwrap().to_string())
}

#[sqlx::test(migrations = false)]
async fn propose_copy_lands_a_checked_draft(pool: PgPool) {
    setup(&pool).await;
    let t = pratibimba::mcp::issue_token(&pool, "demo", "t").await.unwrap();
    let app = admin_app(&pool);
    let (err, text) = tool_text(&rpc_call(&app, &t, "propose_copy", json!({ "slot": "hero", "key": "bold", "copy": { "headline": "Ships tonight" } })).await);
    assert!(!err, "{text}");
    let (status, payload): (String, String) = sqlx::query_as("SELECT status, payload FROM variants WHERE key = 'bold'").fetch_one(&pool).await.unwrap();
    assert_eq!((status.as_str(), payload.as_str()), ("draft", r#"{"headline":"Ships tonight"}"#));
    // Each refusal leaves no row.
    for (key, copy, want) in [
        ("toolong", json!({ "headline": "x".repeat(21) }), "max is 20"),
        ("tagged", json!({ "headline": "<i>hi</i>" }), "no HTML"),
        ("nohead", json!({ "cta_label": "Go" }), "required"),
    ] {
        let (err, text) = tool_text(&rpc_call(&app, &t, "propose_copy", json!({ "slot": "hero", "key": key, "copy": copy })).await);
        assert!(err && text.contains(want), "{key}: {text}");
        assert_eq!(count(&pool, &format!("SELECT count(*) FROM variants WHERE key = '{key}'")).await, 0);
    }
}

#[sqlx::test(migrations = false)]
async fn propose_variant_on_a_copy_slot_is_refused_even_as_json(pool: PgPool) {
    setup(&pool).await;
    let t = pratibimba::mcp::issue_token(&pool, "demo", "t").await.unwrap();
    let app = admin_app(&pool);
    for html in ["<h2>Hi</h2>", r#"{"headline":"<script>x</script>"}"#] {
        let (err, text) = tool_text(&rpc_call(&app, &t, "propose_variant", json!({ "slot": "hero", "key": "sneak", "html": html })).await);
        assert!(err && text.contains("propose_copy"), "{text}");
    }
    assert_eq!(count(&pool, "SELECT count(*) FROM variants WHERE key = 'sneak'").await, 0);
}

#[sqlx::test(migrations = false)]
async fn propose_copy_on_a_swap_slot_is_refused(pool: PgPool) {
    setup_demo(&pool).await;
    let t = pratibimba::mcp::issue_token(&pool, "demo", "t").await.unwrap();
    let (err, text) = tool_text(&rpc_call(&admin_app(&pool), &t, "propose_copy", json!({ "slot": "hero", "key": "c", "copy": { "headline": "Hi" } })).await);
    assert!(err && text.contains("not a copy slot"), "{text}");
}

#[sqlx::test(migrations = false)]
async fn list_slots_shows_the_fields(pool: PgPool) {
    setup(&pool).await;
    let t = pratibimba::mcp::issue_token(&pool, "demo", "t").await.unwrap();
    let (_, text) = tool_text(&rpc_call(&admin_app(&pool), &t, "list_slots", json!({})).await);
    let j: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(j["slots"][0]["copy_fields"][0]["name"], "headline", "{j}");
}

// ---- people and approval ----

#[sqlx::test(migrations = false)]
async fn a_dashboard_draft_on_a_copy_slot_is_checked(pool: PgPool) {
    setup(&pool).await;
    let app = admin_app(&pool);
    let r = api_post(&app, "/api/sites/demo/slots/hero/variants", json!({ "key": "h", "html": "<h2>Hi</h2>" })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    let r = api_post(&app, "/api/sites/demo/slots/hero/variants", json!({ "key": "ok", "html": r#"{"headline":"Hello"}"# })).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn approval_rechecks_against_the_current_fields_and_cards_show_text(pool: PgPool) {
    setup(&pool).await;
    let app = admin_app(&pool);
    let r = api_post(&app, "/api/sites/demo/slots/hero/variants", json!({ "key": "mid", "html": r#"{"headline":"Fifteen chars!!"}"# })).await;
    let id = r.json()["id"].as_i64().unwrap();
    let cards = api_get(&app, "/api/sites/demo/approvals").await.json();
    let card = cards.as_array().or_else(|| cards["drafts"].as_array()).map(|a| a[0].clone()).unwrap_or_else(|| panic!("{cards}"));
    assert_eq!(card["copy"], json!({ "headline": "Fifteen chars!!" }), "{card}");
    assert_eq!(card["fields"][0]["name"], "headline");
    // No traffic yet: the champion is the baseline, the page's own copy, so it has no fields.
    assert_eq!((card["champion"]["key"].as_str(), &card["champion"]["copy"]), (Some("own"), &Value::Null), "{card}");
    // The site file tightens the headline to 10: the draft no longer fits and cannot go live.
    let mut f = copy_file(base_and(json!({ "key": "short", "copy": { "headline": "Fast", "cta_label": "Shop" } })));
    f["slots"][0]["copy"]["fields"][0]["max"] = json!(10);
    pratibimba::site_file::apply(&pool, &file_from(f)).await.unwrap();
    let r = api_post(&app, &format!("/api/variants/{id}/approve"), json!({ "approver": "Ritwik", "traffic_share": 0.2 })).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text);
    assert!(r.text.contains("max is 10"), "{}", r.text);
}
