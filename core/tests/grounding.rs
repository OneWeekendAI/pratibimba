//! The grounding check on agent-written drafts. Before this, every draft was labelled
//! "written by a person" and the check showed n/a, so an MCP draft was never checked against
//! the site's facts.

mod common;
use axum::Router;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn propose(app: &Router, token: &str, key: &str, html: &str) -> Value {
    let r = send(
        app,
        axum::http::Request::post("/mcp")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::from(
                json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "propose_variant",
                    "arguments": { "agent": "Claude Code", "justification": "grounding test", "slot": "hero", "key": key, "html": html } } })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    let j = r.json();
    serde_json::from_str(j["result"]["content"][0]["text"].as_str().expect("tool text")).unwrap()
}

fn grounding(checks: &Value) -> Value {
    checks.as_array().unwrap().iter().find(|c| c["name"] == "grounding").expect("a grounding row").clone()
}

async fn card(app: &Router, key: &str) -> Value {
    let q = api_get(app, "/api/sites/demo/approvals").await.json();
    q["drafts"].as_array().unwrap().iter().find(|c| c["key"] == key).expect("draft is queued").clone()
}

async fn setup(pool: &PgPool) -> (Router, String) {
    setup_demo(pool).await;
    let t = pratibimba::mcp::issue_token(pool, "demo", "test agent").await.unwrap();
    (admin_app(pool), t)
}

#[sqlx::test(migrations = false)]
async fn agent_draft_is_not_labelled_as_written_by_a_person(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    propose(&app, &t, "ship", "<h2>Ships in 2 days</h2>").await;
    let g = grounding(&card(&app, "ship").await["checks"]);
    assert_ne!(g["status"], "not_applicable", "grounding must run on an agent draft: {g}");
    assert!(!g["reason"].as_str().unwrap().contains("person"), "{g}");
}

#[sqlx::test(migrations = false)]
async fn without_facts_an_agent_draft_warns_that_its_claims_are_unchecked(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let r = propose(&app, &t, "ship", "<h2>Ships in 2 days</h2>").await;
    let g = grounding(&r["checks"]);
    assert_eq!(g["status"], "warn", "{g}");
    assert!(g["reason"].as_str().unwrap().contains("facts"), "{g}");
}

#[sqlx::test(migrations = false)]
async fn a_figure_missing_from_the_facts_is_flagged(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    sqlx::query("UPDATE sites SET facts = 'Orders ship in 2 days. Returns are free for 30 days.'").execute(&pool).await.unwrap();
    propose(&app, &t, "grounded", "<h2>Ships in 2 days, 30-day returns</h2>").await;
    propose(&app, &t, "invented", "<h2>Ships in 2 days, 20% off today</h2>").await;
    let ok = grounding(&card(&app, "grounded").await["checks"]);
    assert_eq!(ok["status"], "pass", "{ok}");
    let bad = grounding(&card(&app, "invented").await["checks"]);
    assert_eq!(bad["status"], "warn", "{bad}");
    assert!(bad["reason"].as_str().unwrap().contains("20%"), "{bad}");
}

#[sqlx::test(migrations = false)]
async fn a_human_draft_still_skips_grounding(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    let r = api_post(&app, "/api/sites/demo/slots/hero/variants", json!({ "key": "mine", "html": "<h2>20% off</h2>" })).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    assert_eq!(grounding(&card(&app, "mine").await["checks"])["status"], "not_applicable");
}

#[sqlx::test(migrations = false)]
async fn the_site_file_sets_the_facts(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    let mut f = serde_json::to_value(json!({
        "site": "demo", "origin": "http://localhost:5173", "facts": "Orders ship in 2 days.",
        "slots": [{ "key": "hero", "selector": "#hero", "variants": [{ "key": "baseline", "baseline": true }] }]
    }))
    .unwrap();
    pratibimba::site_file::apply(&pool, &file_from(f.clone())).await.unwrap();
    let facts: Option<String> = sqlx::query_scalar("SELECT facts FROM sites WHERE id = 'demo'").fetch_one(&pool).await.unwrap();
    assert_eq!(facts.as_deref(), Some("Orders ship in 2 days."));
    // Omitting `facts` keeps them.
    f.as_object_mut().unwrap().remove("facts");
    pratibimba::site_file::apply(&pool, &file_from(f)).await.unwrap();
    let kept: Option<String> = sqlx::query_scalar("SELECT facts FROM sites WHERE id = 'demo'").fetch_one(&pool).await.unwrap();
    assert_eq!(kept.as_deref(), Some("Orders ship in 2 days."));
}
