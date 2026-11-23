//! The bring-your-own-agent panel's backend: the tail filtered to one slot, and the per-agent
//! instruction-pack status that agents report through the optional `pack` argument.

mod common;
use axum::{Router, body::Body, http::Request};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn call(app: &Router, token: &str, tool: &str, args: Value) -> Value {
    let body = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": { "name": tool, "arguments": args } });
    let req = Request::post("/mcp").header("content-type", "application/json").header("accept", "application/json, text/event-stream").header("authorization", format!("Bearer {token}"));
    let r = send(app, req.body(Body::from(body.to_string())).unwrap()).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json()["result"].clone()
}

fn who(agent: &str, pack: Option<&str>, extra: Value) -> Value {
    let mut a = json!({ "agent": agent, "justification": "panel test" });
    if let Some(p) = pack {
        a["pack"] = json!(p);
    }
    a.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    a
}

async fn setup(pool: &PgPool) -> (Router, String) {
    setup_demo(pool).await;
    (admin_app(pool), pratibimba::mcp::issue_token(pool, "demo", "t").await.unwrap())
}

#[sqlx::test(migrations = false)]
async fn the_tail_can_be_scoped_to_one_slot(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let slots = call(&app, &t, "list_slots", who("Claude Code", None, json!({}))).await;
    let key = serde_json::from_str::<Value>(slots["content"][0]["text"].as_str().unwrap()).unwrap()["slots"][0]["key"].as_str().unwrap().to_string();
    call(&app, &t, "list_variants", who("Claude Code", None, json!({ "slot": key }))).await;
    call(&app, &t, "list_variants", who("Claude Code", None, json!({ "slot": "elsewhere" }))).await;
    let all = api_get(&app, "/api/sites/demo/mcp-calls").await.json();
    assert_eq!(all["calls"].as_array().unwrap().len(), 3);
    let one = api_get(&app, &format!("/api/sites/demo/mcp-calls?slot={key}")).await.json();
    let calls = one["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 1, "{one}");
    assert_eq!(calls[0]["tool"], "list_variants");
}

#[sqlx::test(migrations = false)]
async fn pack_status_follows_what_each_agent_reports(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let status = |j: &Value, who: &str| j["packs"].as_array().unwrap().iter().find(|p| p["agent"] == who).unwrap()["status"].clone();
    let j = api_get(&app, "/api/sites/demo/mcp-calls").await.json();
    assert_eq!(j["packs"].as_array().unwrap().len(), 3);
    assert_eq!(status(&j, "Claude Code"), "not_installed");
    call(&app, &t, "list_slots", who("Claude Code", Some(pratibimba::mcp::PACK_VERSION), json!({}))).await;
    call(&app, &t, "list_slots", who("Cursor", Some("0.0.1"), json!({}))).await;
    call(&app, &t, "list_slots", who("my-bot", None, json!({}))).await;
    let j = api_get(&app, "/api/sites/demo/mcp-calls").await.json();
    assert_eq!(j["pack_version"], pratibimba::mcp::PACK_VERSION);
    assert_eq!(status(&j, "Claude Code"), "installed");
    assert_eq!(status(&j, "Cursor"), "out_of_date");
    assert_eq!(status(&j, "Generic MCP"), "not_installed");
    // The newest report wins: Cursor upgrades.
    call(&app, &t, "list_slots", who("Cursor", Some(pratibimba::mcp::PACK_VERSION), json!({}))).await;
    let j = api_get(&app, "/api/sites/demo/mcp-calls").await.json();
    assert_eq!(status(&j, "Cursor"), "installed");
}

#[test]
fn the_core_pack_version_is_the_agent_package_version() {
    let p: Value = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../packages/agent/package.json")).unwrap()).unwrap();
    assert_eq!(p["version"], pratibimba::mcp::PACK_VERSION);
}
