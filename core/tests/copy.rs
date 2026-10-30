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

// ---- templates (P1 B1) ----

fn stored_payload_sql(key: &str) -> String {
    format!("SELECT payload FROM variants WHERE key = '{key}'")
}

#[sqlx::test(migrations = false)]
async fn template_within_max_accepted(pool: PgPool) {
    // "Hi " + a visit count of at most 5 digits = 8, within 20.
    pratibimba::db::migrate(&pool).await.unwrap();
    let f = copy_file(base_and(json!({ "key": "visits", "requires": ["visitor.visit_count"], "copy": { "headline": "Hi {visitor.visit_count}" } })));
    pratibimba::site_file::apply(&pool, &file_from(f)).await.unwrap();
    let p: String = sqlx::query_scalar(&stored_payload_sql("visits")).fetch_one(&pool).await.unwrap();
    assert!(p.contains("\"tpl\""), "stored parsed, not as the string: {p}");
}

// Mutation: make `longest` count a ref as 0 characters -> red.
#[sqlx::test(migrations = false)]
async fn template_over_max_refused_names_path(pool: PgPool) {
    let e = apply_err(&pool, copy_file(base_and(json!({ "key": "usual", "copy": { "headline": "Usual {visitor.top_item}" } })))).await;
    assert!(e.contains("visitor.top_item") && e.contains("its max is 20") && e.contains("truncate"), "{e}");
}

#[sqlx::test(migrations = false)]
async fn truncate_brings_template_under_max(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    let f = copy_file(base_and(json!({ "key": "usual", "requires": ["visitor.top_item"], "copy": { "headline": "Usual {visitor.top_item | truncate: 14}" } })));
    pratibimba::site_file::apply(&pool, &file_from(f)).await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn template_in_image_field_refused(pool: PgPool) {
    let mut f = copy_file(base_and(json!({ "key": "pic", "copy": { "headline": "Hi", "photo": "{visitor.top_item | truncate: 5}" } })));
    f["slots"][0]["copy"]["fields"].as_array_mut().unwrap().push(json!({ "name": "photo", "type": "image" }));
    let e = apply_err(&pool, f).await;
    assert!(e.contains("\"photo\" is an image field"), "{e}");
}

#[sqlx::test(migrations = false)]
async fn stored_template_round_trips(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    let src = "Your {visitor.top_item | truncate: 9, fallback: 'thali'}";
    let f = copy_file(base_and(json!({ "key": "usual", "copy": { "headline": src } })));
    pratibimba::site_file::apply(&pool, &file_from(f)).await.unwrap();
    let p: String = sqlx::query_scalar(&stored_payload_sql("usual")).fetch_one(&pool).await.unwrap();
    let text = pratibimba::copy::read_text(Some(&p)).unwrap();
    let want = pratibimba::copy::FieldValue::Template(pratibimba::template::parse(src).unwrap());
    assert_eq!(text.get("headline"), Some(&want), "{p}");
    // And it reads back the same again: the stored form is stable.
    assert_eq!(pratibimba::copy::payload(&text), p);
}

#[sqlx::test(migrations = false)]
async fn a_bad_template_names_its_fault_when_read(pool: PgPool) {
    // A template is parsed as the site file is read, so the refusal comes from reading it.
    pratibimba::db::migrate(&pool).await.unwrap();
    let f = copy_file(base_and(json!({ "key": "rev", "copy": { "headline": "{visitor.top_item | reverse}" } })));
    let e = serde_json::from_value::<pratibimba::site_file::SiteFile>(f).unwrap_err().to_string();
    assert!(e.contains("reverse"), "{e}");
}

// ---- requires (P1 B1) ----

// Mutation: drop the CHECK from 0300_templates.sql -> red.
#[sqlx::test(migrations = false)]
async fn baseline_with_requires_refused_by_db(pool: PgPool) {
    setup(&pool).await;
    let r = sqlx::query("UPDATE variants SET requires = '{visitor.top_item}' WHERE is_baseline").execute(&pool).await;
    assert!(r.is_err(), "a baseline stored a requires list");
    // The accepting case: the same on a non-baseline variant.
    sqlx::query("UPDATE variants SET requires = '{visitor.top_item}' WHERE key = 'short'").execute(&pool).await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn requires_is_stored_on_apply(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    let f = copy_file(base_and(json!({ "key": "usual", "requires": ["visitor.top_item"],
        "copy": { "headline": "Usual {visitor.top_item | truncate: 14}" } })));
    pratibimba::site_file::apply(&pool, &file_from(f)).await.unwrap();
    let r: Vec<String> = sqlx::query_scalar("SELECT requires FROM variants WHERE key = 'usual'").fetch_one(&pool).await.unwrap();
    assert_eq!(r, vec!["visitor.top_item".to_string()]);
}

#[sqlx::test(migrations = false)]
async fn requires_unknown_path_refused_on_apply(pool: PgPool) {
    let e = apply_err(&pool, copy_file(base_and(json!({ "key": "x", "requires": ["visitor.ltv"], "copy": { "headline": "Hi" } })))).await;
    assert!(e.contains("visitor.ltv") && e.contains("requires"), "{e}");
}

#[sqlx::test(migrations = false)]
async fn requires_on_swap_slot_refused(pool: PgPool) {
    let f = json!({ "site": "demo", "origin": "http://localhost:5173", "slots": [{ "key": "hero", "selector": "#hero",
        "variants": [ { "key": "own", "baseline": true }, { "key": "gift", "html": "<h1>Gift</h1>", "requires": ["visitor.top_item"] } ] }] });
    let e = apply_err(&pool, f).await;
    assert!(e.contains("requires") && e.contains("copy slot"), "{e}");
}

#[sqlx::test(migrations = false)]
async fn requires_on_the_baseline_refused_on_apply(pool: PgPool) {
    let f = copy_file(json!([{ "key": "own", "baseline": true, "requires": ["visitor.top_item"] }, { "key": "x", "copy": { "headline": "Hi" } }]));
    let e = apply_err(&pool, f).await;
    assert!(e.contains("baseline") && e.contains("requires"), "{e}");
}

#[sqlx::test(migrations = false)]
async fn a_dashboard_draft_carries_requires(pool: PgPool) {
    setup(&pool).await;
    let app = admin_app(&pool);
    let body = json!({ "key": "usual", "html": r#"{"headline":"Hi {visitor.top_item | truncate: 10}"}"#, "requires": ["visitor.top_item"] });
    let r = api_post(&app, "/api/sites/demo/slots/hero/variants", body).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    let (req, payload): (Vec<String>, String) = sqlx::query_as("SELECT requires, payload FROM variants WHERE key = 'usual'").fetch_one(&pool).await.unwrap();
    assert_eq!(req, vec!["visitor.top_item".to_string()]);
    assert!(payload.contains("\"tpl\""), "a dashboard draft is stored parsed: {payload}");
    let r = api_post(&app, "/api/sites/demo/slots/hero/variants", json!({ "key": "bad", "html": r#"{"headline":"Hi"}"#, "requires": ["visitor.ltv"] })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
}

// ---- fill at decide (P1 B2) ----
//
// The kitchen's hero: the baseline is the page's own "Home food, cooked today"; `your_usual`
// greets a repeat buyer by their most-ordered dish and requires one. An experiment that puts all
// traffic on the named arms makes each case deterministic: an arm closed to the visitor drops out
// of it and the baseline is served.

const USUAL: &str = "Your usual {visitor.top_item.name | truncate: 30} is ready";

fn kitchen(mode: &str, extra_variants: Value) -> Value {
    let mut variants = json!([
        { "key": "own", "baseline": true },
        { "key": "your_usual", "requires": ["visitor.top_item"], "copy": { "title": USUAL } }
    ]);
    variants.as_array_mut().unwrap().extend(extra_variants.as_array().unwrap().clone());
    json!({ "site": "demo", "origin": "http://localhost:5173", "privacy_mode": mode, "slots": [{
        "key": "hero_copy", "selector": "#hero", "kind": "copy",
        "copy": { "fields": [ { "name": "title", "max": 60, "required": true } ] },
        "variants": variants }] })
}

async fn kitchen_setup(pool: &PgPool, f: Value, forced: &[&str]) {
    pratibimba::db::migrate(pool).await.unwrap();
    pratibimba::site_file::apply(pool, &file_from(f)).await.unwrap();
    set_holdout(pool, 0.0).await;
    let mut arms = vec![];
    for k in forced {
        arms.push(json!({ "variant_id": variant_id(pool, k).await, "weight": 1.0 / forced.len() as f64 }));
    }
    sqlx::query(
        "INSERT INTO experiments (site_id, slot_id, name, metric, arms, starts_at, stops_at, created_by)
         SELECT 'demo', id, 'force', metric, $1, now() - interval '1 hour', now() + interval '1 day', 'test' FROM slots WHERE key = 'hero_copy'",
    )
    .bind(json!(arms))
    .execute(pool)
    .await
    .unwrap();
}

async fn ordered(pool: &PgPool, subject: &str, dish: &str, n: usize, days: f64) {
    for i in 0..n {
        sqlx::query(
            "INSERT INTO events (id, site_id, subject, name, received_at, order_id, value, currency, items)
             VALUES ($1, 'demo', $2, 'purchase', now() - make_interval(secs => $3), $4, 10, 'USD', $5)",
        )
        .bind(uuid::Uuid::new_v4())
        .bind(subject)
        .bind(days * 86400.0 + i as f64)
        .bind(format!("{subject}-{dish}-{i}"))
        .bind(json!([{ "id": dish }]))
        .execute(pool)
        .await
        .unwrap();
    }
}

/// Asha: 10 orders, 9 of them rajma-chawal.
async fn asha(pool: &PgPool) {
    ordered(pool, "asha", "rajma-chawal", 9, 2.0).await;
    ordered(pool, "asha", "thali", 1, 1.0).await;
}

async fn hero(pool: &PgPool, subject: &str) -> Resp {
    let r = post(&app(pool), "/decide", json!({ "site": "demo", "slot": "hero_copy", "subject": subject, "ctx": { "consent": true } })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r
}

#[sqlx::test(migrations = false)]
async fn usual_arm_filled_for_repeat_buyer(pool: PgPool) {
    kitchen_setup(&pool, kitchen("full", json!([])), &["your_usual"]).await;
    asha(&pool).await;
    let d = hero(&pool, "asha").await.json();
    assert_eq!(d["copy"], json!({ "title": "Your usual rajma-chawal is ready" }), "{d}");
}

// Mutation: skip close_by_requires -> red.
#[sqlx::test(migrations = false)]
async fn first_visit_never_sees_empty_hero(pool: PgPool) {
    kitchen_setup(&pool, kitchen("full", json!([])), &["your_usual"]).await;
    for i in 0..100 {
        hero(&pool, &format!("ravi{i}")).await;
    }
    let usual = variant_id(&pool, "your_usual").await;
    assert_eq!(count(&pool, &format!("SELECT count(*) FROM decisions WHERE variant_id = {usual}")).await, 0);
}

#[sqlx::test(migrations = false)]
async fn cookieless_closes_requiring_arm(pool: PgPool) {
    kitchen_setup(&pool, kitchen("cookieless", json!([])), &["your_usual"]).await;
    asha(&pool).await;
    let d = hero(&pool, "asha").await.json();
    assert_eq!((d["variant"].as_str(), &d["copy"]), (Some("own"), &Value::Null), "{d}");
}

#[sqlx::test(migrations = false)]
async fn all_arms_closed_serves_baseline(pool: PgPool) {
    let lapsed = json!([{ "key": "lapsed", "requires": ["visitor.days_since_last_purchase"],
                          "copy": { "title": "Back after {visitor.days_since_last_purchase} days" } }]);
    kitchen_setup(&pool, kitchen("full", lapsed), &["your_usual", "lapsed"]).await;
    let d = hero(&pool, "ravi").await.json();
    assert_eq!(d["variant"], "own", "{d}");
}

// A template without `requires` reaches a visitor with no history: it fills with its fallback.
// Mutation: with_copy returns the stored fields unfilled -> red.
#[sqlx::test(migrations = false)]
async fn response_never_contains_tpl(pool: PgPool) {
    let open = json!([{ "key": "try", "copy": { "title": "Try the {visitor.top_item | truncate: 20, fallback: 'thali'}" } }]);
    kitchen_setup(&pool, kitchen("full", open), &["try"]).await;
    let r = hero(&pool, "ravi").await;
    assert!(!r.text.contains("tpl"), "{}", r.text);
    assert_eq!(r.json()["copy"], json!({ "title": "Try the thali" }), "{}", r.text);
}

// Mutation: bind NULL for `filled` -> red.
#[sqlx::test(migrations = false)]
async fn decision_row_stores_filled(pool: PgPool) {
    kitchen_setup(&pool, kitchen("full", json!([])), &["your_usual"]).await;
    asha(&pool).await;
    let d = hero(&pool, "asha").await.json();
    let (id, filled): (uuid::Uuid, Option<Value>) = sqlx::query_as("SELECT id, filled FROM decisions").fetch_one(&pool).await.unwrap();
    assert_eq!(filled.as_ref(), Some(&d["copy"]), "{d}");
    let detail = pratibimba::explain::detail(&pool, id, chrono::Utc::now()).await.unwrap().unwrap();
    assert_eq!(detail.copy, filled, "the decision page shows what the visitor saw");
}

#[sqlx::test(migrations = false)]
async fn a_plain_decision_stores_no_filled(pool: PgPool) {
    setup(&pool).await;
    post(&app(&pool), "/decide", json!({ "site": "demo", "slot": "hero", "subject": "s1" })).await;
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions WHERE filled IS NOT NULL").await, 0);
}

// Review focus 1. Asha's history stops being readable (the owner drops `history` from the
// fields collected) inside her sticky window: the old filled name must not come back.
// Mutation: close arms by `requires` after the sticky reuse instead of before -> red.
#[sqlx::test(migrations = false)]
async fn sticky_drops_when_requires_no_longer_met(pool: PgPool) {
    kitchen_setup(&pool, kitchen("full", json!([])), &["your_usual"]).await;
    asha(&pool).await;
    let first = hero(&pool, "asha").await.json();
    assert_eq!(first["variant"], "your_usual", "{first}");
    let r = api_post(&admin_app(&pool), "/api/sites/demo/privacy", json!({ "fields_allowed": ["mobile", "returning", "from_ad"] })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let again = hero(&pool, "asha").await;
    assert!(!again.text.contains("rajma"), "{}", again.text);
    assert_eq!(again.json()["variant"], "own", "{}", again.text);
}

// A reload inside the TTL shows the same filled text even after the facts move.
// Mutation: fill the sticky arm afresh instead of reusing `filled` -> red.
#[sqlx::test(migrations = false)]
async fn sticky_reuses_filled_text(pool: PgPool) {
    kitchen_setup(&pool, kitchen("full", json!([])), &["your_usual"]).await;
    asha(&pool).await;
    let first = hero(&pool, "asha").await.json();
    ordered(&pool, "asha", "thali", 20, 0.5).await;
    let again = hero(&pool, "asha").await.json();
    assert_eq!((&again["decision_id"], &again["copy"]), (&first["decision_id"], &first["copy"]), "{again}");
}

// Mutation: drop the `filled = NULL` from the erase -> red.
#[sqlx::test(migrations = false)]
async fn erase_clears_filled(pool: PgPool) {
    kitchen_setup(&pool, kitchen("full", json!([])), &["your_usual"]).await;
    asha(&pool).await;
    hero(&pool, "asha").await;
    let r = api_post(&admin_app(&pool), "/api/sites/demo/erase", json!({ "subject": "asha", "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions WHERE filled IS NOT NULL").await, 0);
}

#[sqlx::test(migrations = false)]
async fn usual_arm_closed_when_orders_are_older_than_the_site_lookback(pool: PgPool) {
    let mut f = kitchen("full", json!([]));
    f["history_settings"] = json!({ "top_item_days": 30 });
    kitchen_setup(&pool, f, &["your_usual"]).await;
    ordered(&pool, "asha", "rajma-chawal", 5, 60.0).await;
    assert_eq!(hero(&pool, "asha").await.json()["variant"], "own");
}

// Mutation: remove `kind NOT IN ('sorted','copy')` from the edge snapshot's slot list -> red.
#[sqlx::test(migrations = false)]
async fn copy_slot_absent_from_edge_snapshot(pool: PgPool) {
    kitchen_setup(&pool, kitchen("full", json!([])), &["your_usual"]).await;
    let c = pratibimba::edge::content(&pool, "demo").await.unwrap().unwrap();
    assert!(c["slots"].get("hero_copy").is_none(), "{}", c["slots"]);
}

// ---- braces: the escape, and copy stored before templates (P1 B2) ----

#[sqlx::test(migrations = false)]
async fn an_escaped_brace_in_a_site_file_reaches_the_page_as_one_brace(pool: PgPool) {
    let f = kitchen("full", json!([{ "key": "code", "copy": { "title": "Use code {{SAVE10}} today" } }]));
    kitchen_setup(&pool, f, &["code"]).await;
    assert_eq!(hero(&pool, "ravi").await.json()["copy"], json!({ "title": "Use code {SAVE10} today" }));
}

#[sqlx::test(migrations = false)]
async fn a_lone_brace_in_a_site_file_is_refused_with_the_escape(pool: PgPool) {
    let f = kitchen("full", json!([{ "key": "code", "copy": { "title": "Use code {SAVE10} today" } }]));
    let e = serde_json::from_value::<pratibimba::site_file::SiteFile>(f).unwrap_err().to_string();
    assert!(e.contains("{{"), "{e}");
}

/// A copy variant as stored before migration 0300: a plain string holding a lone brace.
async fn old_brace_copy(pool: &PgPool, status: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO variants (slot_id, key, payload, author, status) SELECT id, 'old', '{\"title\":\"Use code {SAVE10} today\"}', 'human', $1
           FROM slots WHERE key = 'hero_copy' RETURNING id",
    )
    .bind(status)
    .fetch_one(pool)
    .await
    .unwrap()
}

// Mutation: read stored strings as template language (the door's parser) -> red.
#[sqlx::test(migrations = false)]
async fn old_brace_copy_decides_verbatim(pool: PgPool) {
    kitchen_setup(&pool, kitchen("full", json!([])), &[]).await;
    let old = old_brace_copy(&pool, "draft").await;
    sqlx::query("UPDATE variants SET status = 'approved', approved_by = 'owner', approved_at = now() WHERE id = $1").bind(old).execute(&pool).await.unwrap();
    sqlx::query("UPDATE experiments SET arms = $1").bind(json!([{ "variant_id": old, "weight": 1.0 }])).execute(&pool).await.unwrap();
    assert_eq!(hero(&pool, "ravi").await.json()["copy"], json!({ "title": "Use code {SAVE10} today" }));
}

// Mutation: read stored strings as template language -> red (approval re-checks the stored copy).
#[sqlx::test(migrations = false)]
async fn old_brace_draft_can_still_be_approved(pool: PgPool) {
    kitchen_setup(&pool, kitchen("full", json!([])), &[]).await;
    let old = old_brace_copy(&pool, "draft").await;
    let r = api_post(&admin_app(&pool), &format!("/api/variants/{old}/approve"), json!({ "approver": "Ritwik", "traffic_share": 0.2 })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn a_dashboard_draft_stores_an_escaped_brace_as_one(pool: PgPool) {
    kitchen_setup(&pool, kitchen("full", json!([])), &[]).await;
    let r = api_post(&admin_app(&pool), "/api/sites/demo/slots/hero_copy/variants", json!({ "key": "esc", "html": r#"{"title":"Use code {{SAVE10}}"}"# })).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    let p: String = sqlx::query_scalar(&stored_payload_sql("esc")).fetch_one(&pool).await.unwrap();
    assert_eq!(p, r#"{"title":"Use code {SAVE10}"}"#);
}

// Every site file in the repo still reads under the brace rule: one test per file.
macro_rules! site_file_reads {
    ($($name:ident => $path:expr),* $(,)?) => {$(
        #[test]
        fn $name() {
            let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../", $path);
            let v: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
            let f: pratibimba::site_file::SiteFile = serde_json::from_value(v).unwrap_or_else(|e| panic!("{p}: {e}"));
            f.validate().unwrap_or_else(|e| panic!("{p}: {e}"));
        }
    )*};
}
site_file_reads! {
    site_file_ci_reads => "ci/site.json",
    site_file_e2e_reads => "dashboard/e2e/site.json",
    site_file_e2e_kiosk_reads => "dashboard/e2e/kiosk.json",
    site_file_e2e_sorted_reads => "dashboard/e2e/sorted.json",
    site_file_example_plain_reads => "examples/plain/site.json",
    site_file_example_next_reads => "examples/next/site.json",
    site_file_example_edge_reads => "examples/edge/site.json",
}
