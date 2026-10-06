//! The MCP server at `/mcp`: per-site tokens, read tools, drafts-only writes, an audit row on
//! every tool call, and no path from an MCP token to an approved variant.

mod common;
use axum::{Router, body::Body, http::Request};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn rpc(app: &Router, token: Option<&str>, body: Value) -> Resp {
    let mut req = Request::post("/mcp").header("content-type", "application/json").header("accept", "application/json, text/event-stream");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    send(app, req.body(Body::from(body.to_string())).unwrap()).await
}

/// A `tools/call`; returns the JSON-RPC `result` (which carries `isError` for tool errors).
async fn call(app: &Router, token: &str, tool: &str, args: Value) -> Value {
    let r = rpc(app, Some(token), json!({ "jsonrpc": "2.0", "id": 41, "method": "tools/call", "params": { "name": tool, "arguments": args } })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let j = r.json();
    assert!(j.get("error").is_none(), "protocol error: {j}");
    j["result"].clone()
}

/// The JSON a successful tool returned.
fn data(result: &Value) -> Value {
    assert_ne!(result["isError"], json!(true), "tool failed: {result}");
    let text = result["content"][0]["text"].as_str().expect("text content");
    serde_json::from_str(text).unwrap()
}

fn err_text(result: &Value) -> String {
    assert_eq!(result["isError"], json!(true), "expected a tool error: {result}");
    result["content"][0]["text"].as_str().unwrap().to_string()
}

fn why(extra: Value) -> Value {
    let mut a = json!({ "agent": "Claude Code", "justification": "testing the MCP surface" });
    a.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    a
}

async fn token(pool: &PgPool, site: &str) -> String {
    pratibimba::mcp::issue_token(pool, site, "test agent").await.unwrap()
}

async fn setup(pool: &PgPool) -> (Router, String) {
    setup_demo(pool).await;
    let t = token(pool, "demo").await;
    (admin_app(pool), t)
}

async fn status_of(pool: &PgPool, key: &str) -> String {
    sqlx::query_scalar("SELECT status FROM variants WHERE key = $1").bind(key).fetch_one(pool).await.unwrap()
}

// ---- transport and auth ----

#[sqlx::test(migrations = false)]
async fn no_token_is_refused(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    let r = rpc(&app, None, json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = false)]
async fn wrong_token_is_refused(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let r = rpc(&app, Some(&format!("{t}x")), json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = false)]
async fn admin_token_is_not_an_mcp_token(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    let r = rpc(&app, Some(TOKEN), json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = false)]
async fn revoked_token_is_refused(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    sqlx::query("UPDATE mcp_tokens SET revoked_at = now()").execute(&pool).await.unwrap();
    let r = rpc(&app, Some(&t), json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = false)]
async fn token_is_stored_hashed(pool: PgPool) {
    let (_, t) = setup(&pool).await;
    assert_eq!(count(&pool, &format!("SELECT count(*) FROM mcp_tokens WHERE token_hash LIKE '%{t}%'")).await, 0);
}

#[sqlx::test(migrations = false)]
async fn initialize_names_the_server_and_tools_capability(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let r = rpc(&app, Some(&t), json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } } })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let j = r.json();
    assert_eq!(j["id"], 0);
    assert_eq!(j["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(j["result"]["serverInfo"]["name"], "pratibimba");
    assert!(j["result"]["capabilities"]["tools"].is_object());
}

#[sqlx::test(migrations = false)]
async fn notification_gets_202_and_no_body(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let r = rpc(&app, Some(&t), json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    assert!(r.text.is_empty());
}

#[sqlx::test(migrations = false)]
async fn get_is_405(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let r = send(&app, Request::get("/mcp").header("authorization", format!("Bearer {t}")).body(Body::empty()).unwrap()).await;
    assert_eq!(r.status, StatusCode::METHOD_NOT_ALLOWED);
}

#[sqlx::test(migrations = false)]
async fn unknown_method_is_a_json_rpc_error(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let r = rpc(&app, Some(&t), json!({ "jsonrpc": "2.0", "id": 9, "method": "resources/list" })).await;
    assert_eq!(r.json()["error"]["code"], -32601);
}

// ---- the catalogue, and the gate ----

#[sqlx::test(migrations = false)]
async fn tools_list_is_exactly_the_catalogue(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let r = rpc(&app, Some(&t), json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })).await;
    let tools = r.json()["result"]["tools"].as_array().unwrap().clone();
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort();
    assert_eq!(
        names,
        ["explain_decision", "get_results", "get_site", "list_decisions", "list_sites", "list_slots", "list_variants", "propose_variant"]
    );
    for tool in &tools {
        let req: Vec<&str> = tool["inputSchema"]["required"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert!(req.contains(&"justification") && req.contains(&"agent"), "{} must require agent and justification", tool["name"]);
    }
}

#[sqlx::test(migrations = false)]
async fn there_is_no_approve_tool(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let id = data(&call(&app, &t, "propose_variant", why(json!({ "slot": "hero", "key": "ship", "html": "<h2>Ship today</h2>" }))).await)["id"].as_i64().unwrap();
    for name in ["approve_variant", "approve"] {
        let r = rpc(&app, Some(&t), json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": name, "arguments": why(json!({ "id": id, "approver": "me", "traffic_share": 0.5 })) } })).await;
        let j = r.json();
        assert!(j.get("error").is_some() || j["result"]["isError"] == json!(true), "{name} did something: {j}");
    }
    assert_eq!(status_of(&pool, "ship").await, "draft");
}

#[sqlx::test(migrations = false)]
async fn mcp_token_cannot_approve_over_http(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let id = data(&call(&app, &t, "propose_variant", why(json!({ "slot": "hero", "key": "ship", "html": "<h2>Ship today</h2>" }))).await)["id"].as_i64().unwrap();
    let r = send(
        &app,
        Request::post(format!("/api/variants/{id}/approve"))
            .header("authorization", format!("Bearer {t}"))
            .header("content-type", "application/json")
            .body(Body::from(json!({ "approver": "agent", "traffic_share": 0.5 }).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{}", r.text);
    assert_eq!(status_of(&pool, "ship").await, "draft");
}

/// The guarantee, from every angle a write can take: an MCP write never yields an approved
/// variant, never touches one, and never reaches a visitor.
#[sqlx::test(migrations = false)]
async fn mcp_write_never_produces_an_approved_variant(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    set_holdout(&pool, 0.0).await;
    let approved_before = count(&pool, "SELECT count(*) FROM variants WHERE status = 'approved'").await;

    // A plain proposal lands as a draft, unapproved.
    let ok = data(&call(&app, &t, "propose_variant", why(json!({ "slot": "hero", "key": "ship", "html": "<h2>Ship today</h2>" }))).await);
    assert_eq!(ok["status"], "draft");
    let (status, by, share): (String, Option<String>, Option<f32>) =
        sqlx::query_as("SELECT status, approved_by, traffic_share FROM variants WHERE key = 'ship'").fetch_one(&pool).await.unwrap();
    assert_eq!((status.as_str(), by, share), ("draft", None, None));

    // Smuggled approval fields are refused outright, not ignored.
    for smuggle in [json!({ "status": "approved" }), json!({ "approved_by": "agent" }), json!({ "traffic_share": 0.5 }), json!({ "is_baseline": true })] {
        let mut args = why(json!({ "slot": "hero", "key": "sneak", "html": "<h2>Sneak</h2>" }));
        args.as_object_mut().unwrap().extend(smuggle.as_object().unwrap().clone());
        err_text(&call(&app, &t, "propose_variant", args).await);
    }
    assert_eq!(count(&pool, "SELECT count(*) FROM variants WHERE key = 'sneak'").await, 0);

    // Reusing an approved variant's key cannot overwrite it.
    err_text(&call(&app, &t, "propose_variant", why(json!({ "slot": "hero", "key": "gift", "html": "<h2>Hijack</h2>" }))).await);
    let gift: String = sqlx::query_scalar("SELECT payload FROM variants WHERE key = 'gift'").fetch_one(&pool).await.unwrap();
    assert_eq!(gift, "<h1>Gift sets</h1>");

    assert_eq!(count(&pool, "SELECT count(*) FROM variants WHERE status = 'approved'").await, approved_before);
    for i in 0..80 {
        assert_ne!(decide(&app, &format!("v{i}"), false).await.json()["variant"], "ship", "an MCP draft reached a visitor");
    }
}

#[sqlx::test(migrations = false)]
async fn proposal_is_an_agent_draft_with_its_checks_and_note(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let r = data(&call(&app, &t, "propose_variant", why(json!({ "slot": "hero", "key": "ship", "html": "<h2>Ship <script>x</script></h2>", "note": "aimed at returning buyers" }))).await);
    assert_eq!(r["status"], "draft");
    assert!(r["checks"].as_array().unwrap().iter().any(|c| c["name"] == "technical" && c["status"] == "fail"), "{r}");
    let author: String = sqlx::query_scalar("SELECT author FROM variants WHERE key = 'ship'").fetch_one(&pool).await.unwrap();
    assert_eq!(author, "agent");
    let (actor, comment): (String, String) =
        sqlx::query_as("SELECT actor, comment FROM approvals WHERE action = 'comment'").fetch_one(&pool).await.unwrap();
    assert_eq!((actor.as_str(), comment.as_str()), ("Claude Code", "aimed at returning buyers"));
}

#[sqlx::test(migrations = false)]
async fn draft_shows_in_the_approvals_queue(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&call(&app, &t, "propose_variant", why(json!({ "slot": "hero", "key": "ship", "html": "<h2>Ship today</h2>" }))).await);
    let q = api_get(&app, "/api/sites/demo/approvals").await.json();
    let card = q["drafts"].as_array().unwrap().iter().find(|c| c["key"] == "ship").expect("draft is queued").clone();
    assert_eq!(card["author"], "agent");
    assert_eq!(card["agent"]["name"], "Claude Code");
    assert_eq!(card["agent"]["justification"], "testing the MCP surface");
}

// ---- the audit stub ----

#[sqlx::test(migrations = false)]
async fn every_tool_call_is_audited(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&call(&app, &t, "list_slots", why(json!({ "on_behalf_of": "ritwik" }))).await);
    data(&call(&app, &t, "propose_variant", why(json!({ "slot": "hero", "key": "ship", "html": "<h2>Ship</h2>" }))).await);
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, Option<String>, String, Option<String>, String, Value, String)> = sqlx::query_as(
        "SELECT c.site_id, c.tool, c.on_behalf_of, c.agent, c.justification, c.outcome, c.call_id, k.name
           FROM mcp_calls c JOIN mcp_tokens k ON k.id = c.token_id ORDER BY c.id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "demo");
    assert_eq!(rows[0].1, "list_slots");
    assert_eq!(rows[0].2.as_deref(), Some("ritwik"));
    assert_eq!(rows[0].3, "Claude Code");
    assert_eq!(rows[0].4.as_deref(), Some("testing the MCP surface"));
    assert_eq!(rows[0].5, "ok");
    assert_eq!(rows[0].6, json!(41));
    assert_eq!(rows[0].7, "test agent");
    assert_eq!(rows[1].1, "propose_variant");
    let linked: i64 = count(&pool, "SELECT count(*) FROM variants v JOIN mcp_calls c ON c.id = v.mcp_call_id WHERE v.key = 'ship' AND c.tool = 'propose_variant'").await;
    assert_eq!(linked, 1, "the draft must point at the call that wrote it");
}

#[sqlx::test(migrations = false)]
async fn every_result_returns_its_audit_stub(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let r = data(&call(&app, &t, "list_slots", why(json!({ "on_behalf_of": "ritwik" }))).await);
    let row: i64 = count(&pool, "SELECT max(id) FROM mcp_calls").await;
    assert_eq!(r["audit"]["id"], json!(row));
    assert_eq!(r["audit"]["call_id"], json!(41));
    assert_eq!(r["audit"]["tool"], "list_slots");
    assert_eq!(r["audit"]["agent"], "Claude Code");
    assert_eq!(r["audit"]["on_behalf_of"], "ritwik");
    assert_eq!(r["audit"]["justification"], "testing the MCP surface");
}

#[sqlx::test(migrations = false)]
async fn non_object_arguments_are_refused_and_audited(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let r = rpc(&app, Some(&t), json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": { "name": "list_slots", "arguments": [1] } })).await;
    assert_eq!(r.json()["error"]["code"], json!(-32602));
    assert_eq!(count(&pool, "SELECT count(*) FROM mcp_calls WHERE outcome = 'refused' AND tool = 'list_slots'").await, 1);
}

#[sqlx::test(migrations = false)]
async fn a_call_without_justification_is_refused_and_still_audited(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    for args in [json!({ "agent": "x", "slot": "hero", "key": "ship", "html": "<h2>a</h2>" }), json!({ "agent": "x", "justification": "  ", "slot": "hero", "key": "ship", "html": "<h2>a</h2>" })] {
        err_text(&call(&app, &t, "propose_variant", args).await);
    }
    err_text(&call(&app, &t, "list_slots", json!({ "justification": "no agent named" })).await);
    assert_eq!(count(&pool, "SELECT count(*) FROM variants WHERE key = 'ship'").await, 0);
    assert_eq!(count(&pool, "SELECT count(*) FROM mcp_calls WHERE outcome = 'refused'").await, 3);
}

#[sqlx::test(migrations = false)]
async fn a_failed_tool_is_audited_as_an_error(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    err_text(&call(&app, &t, "list_variants", why(json!({ "slot": "nope" }))).await);
    assert_eq!(count(&pool, "SELECT count(*) FROM mcp_calls WHERE outcome = 'error' AND tool = 'list_variants' AND error LIKE '%nope%'").await, 1);
}

// ---- read tools ----

#[sqlx::test(migrations = false)]
async fn list_sites_and_get_site_see_only_the_token_site(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    sqlx::query("INSERT INTO sites (id, origin) VALUES ('other', 'http://other')").execute(&pool).await.unwrap();
    let sites = data(&call(&app, &t, "list_sites", why(json!({}))).await);
    let ids: Vec<&str> = sites["sites"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["demo"]);
    let site = data(&call(&app, &t, "get_site", why(json!({}))).await);
    assert_eq!(site["id"], "demo");
    assert_eq!(site["slots"][0]["key"], "hero");
    err_text(&call(&app, &t, "get_site", why(json!({ "site": "other" }))).await);
}

#[sqlx::test(migrations = false)]
async fn list_slots_names_the_champion(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let s = data(&call(&app, &t, "list_slots", why(json!({}))).await);
    let hero = &s["slots"][0];
    assert_eq!(hero["key"], "hero");
    assert_eq!(hero["selector"], "#hero");
    assert_eq!(hero["metric"], "click");
    assert_eq!(hero["champion"], "baseline");
}

#[sqlx::test(migrations = false)]
async fn list_variants_carries_reviewer_comments(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let id = data(&call(&app, &t, "propose_variant", why(json!({ "slot": "hero", "key": "ship", "html": "<h2>Ship</h2>" }))).await)["id"].as_i64().unwrap();
    api_post(&app, &format!("/api/variants/{id}/comment"), json!({ "actor": "Ritwik", "comment": "shorter headline please" })).await;
    let v = data(&call(&app, &t, "list_variants", why(json!({ "slot": "hero" }))).await);
    let ship = v["variants"].as_array().unwrap().iter().find(|x| x["key"] == "ship").unwrap().clone();
    assert_eq!(ship["status"], "draft");
    assert_eq!(ship["html"], "<h2>Ship</h2>");
    assert!(ship["comments"].as_array().unwrap().iter().any(|c| c["comment"] == "shorter headline please" && c["actor"] == "Ritwik"), "{ship}");
    assert!(v["variants"].as_array().unwrap().iter().any(|x| x["key"] == "gift" && x["status"] == "approved"));
}

#[sqlx::test(migrations = false)]
async fn get_results_reports_per_variant_counts(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    seed_decisions(&pool, "gift", false, 40, 10, 60).await;
    let r = data(&call(&app, &t, "get_results", why(json!({ "slot": "hero" }))).await);
    let gift = r["slots"][0]["variants"].as_array().unwrap().iter().find(|v| v["key"] == "gift").unwrap().clone();
    assert_eq!(gift["counts"]["n"], 40, "{gift}");
    assert_eq!(gift["counts"]["conversions"], 10, "{gift}");
}

#[sqlx::test(migrations = false)]
async fn list_decisions_and_explain_one(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let d = decide(&app, "visitor-1", true).await.json();
    let id = d["decision_id"].as_str().unwrap().to_string();
    let list = data(&call(&app, &t, "list_decisions", why(json!({ "slot": "hero" }))).await);
    assert!(list["decisions"].as_array().unwrap().iter().any(|x| x["id"] == id.as_str()), "{list}");
    let e = data(&call(&app, &t, "explain_decision", why(json!({ "id": id }))).await);
    assert_eq!(e["id"], id.as_str());
    assert!(!e["sentence"].as_str().unwrap().is_empty());
}

#[sqlx::test(migrations = false)]
async fn another_sites_decision_is_not_found(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    sqlx::query("INSERT INTO sites (id, origin) VALUES ('other', 'http://other')").execute(&pool).await.unwrap();
    let other = token(&pool, "other").await;
    let id = decide(&app, "visitor-1", true).await.json()["decision_id"].as_str().unwrap().to_string();
    let msg = err_text(&call(&app, &other, "explain_decision", why(json!({ "id": id }))).await);
    assert!(msg.contains("unknown decision"), "{msg}");
    err_text(&call(&app, &other, "list_variants", why(json!({ "slot": "hero" }))).await);
    err_text(&call(&app, &other, "propose_variant", why(json!({ "slot": "hero", "key": "x", "html": "<p>x</p>" }))).await);
    assert_eq!(count(&pool, "SELECT count(*) FROM variants WHERE key = 'x'").await, 0);
}

// ---- issuing tokens ----

#[sqlx::test(migrations = false)]
async fn admin_api_issues_a_working_token(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    let r = api_post(&app, "/api/sites/demo/mcp-tokens", json!({ "name": "cursor on laptop" })).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    let t = r.json()["token"].as_str().unwrap().to_string();
    assert!(t.starts_with("prb_mcp_"));
    data(&call(&app, &t, "list_sites", why(json!({}))).await);
    let r = api_post(&app, "/api/sites/nope/mcp-tokens", json!({ "name": "x" })).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}
