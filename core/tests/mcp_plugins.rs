//! C2: the MCP plugin tools. `list_plugins` reads the catalogue as the token's site sees it;
//! `install_plugin` only REQUESTS an install, exactly as `retire_variant` only proposes: nothing
//! changes until a person confirms (`POST /api/plugin-requests/{id}/confirm`).

mod common;
use axum::{Router, body::Body, http::Request};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn call(app: &Router, token: &str, tool: &str, args: Value) -> Value {
    let body = json!({ "jsonrpc": "2.0", "id": 41, "method": "tools/call", "params": { "name": tool, "arguments": args } });
    let req = Request::post("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("authorization", format!("Bearer {token}"));
    let r = send(app, req.body(Body::from(body.to_string())).unwrap()).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json()["result"].clone()
}

fn data(result: &Value) -> Value {
    assert_ne!(result["isError"], json!(true), "tool failed: {result}");
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn is_error(result: &Value) -> bool {
    result["isError"] == json!(true)
}

fn why(extra: Value) -> Value {
    let mut a = json!({ "agent": "Claude Code", "justification": "the owner wants multi-touch credit" });
    a.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    a
}

async fn setup(pool: &PgPool) -> (Router, String) {
    setup_demo(pool).await;
    let t = pratibimba::mcp::issue_token(pool, "demo", "test agent").await.unwrap();
    (admin_app(pool), t)
}

async fn request_linear(app: &Router, t: &str) -> Value {
    call(app, t, "install_plugin", why(json!({ "plugin": "attribution.linear" }))).await
}

async fn open_request(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT id FROM plugin_requests WHERE status = 'pending'").fetch_one(pool).await.unwrap()
}

async fn installed(pool: &PgPool) -> i64 {
    count(pool, "SELECT count(*) FROM plugins").await
}

// ---- list_plugins ----

#[sqlx::test(migrations = false)]
async fn list_plugins_shows_the_catalogue_for_the_tokens_site(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let l = data(&call(&app, &t, "list_plugins", why(json!({}))).await);
    assert_eq!(l["site"], "demo");
    let names: Vec<&str> = l["plugins"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["attribution.linear", "sink.identity_resolver"], "{l}");
}

#[sqlx::test(migrations = false)]
async fn list_plugins_shows_an_open_request(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&request_linear(&app, &t).await);
    let l = data(&call(&app, &t, "list_plugins", why(json!({}))).await);
    let p = l["plugins"].as_array().unwrap().iter().find(|p| p["name"] == "attribution.linear").unwrap();
    assert_eq!(p["request"]["status"], "pending", "{l}");
}

#[sqlx::test(migrations = false)]
async fn list_plugins_is_audited(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&call(&app, &t, "list_plugins", why(json!({}))).await);
    assert_eq!(count(&pool, "SELECT count(*) FROM mcp_calls WHERE tool = 'list_plugins' AND outcome = 'ok'").await, 1);
}

// ---- install_plugin only requests ----

#[sqlx::test(migrations = false)]
async fn install_plugin_only_requests(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    let r = data(&request_linear(&app, &t).await);
    assert_eq!(r["status"], "requested", "{r}");
    assert_eq!(installed(&pool).await, 0, "an agent installed a plugin");
}

#[sqlx::test(migrations = false)]
async fn a_request_records_the_agent_its_reason_and_config(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&call(&app, &t, "install_plugin", why(json!({ "plugin": "sink.identity_resolver", "config": { "url": "https://r.example.com" } }))).await);
    let (agent, reason, config): (String, String, Value) =
        sqlx::query_as("SELECT agent, reason, config FROM plugin_requests").fetch_one(&pool).await.unwrap();
    assert_eq!((agent.as_str(), reason.as_str()), ("Claude Code", "the owner wants multi-touch credit"));
    assert_eq!(config, json!({ "url": "https://r.example.com" }));
}

#[sqlx::test(migrations = false)]
async fn install_plugin_is_audited(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&request_linear(&app, &t).await);
    let audited: i64 = sqlx::query_scalar("SELECT mcp_call_id FROM plugin_requests").fetch_one(&pool).await.unwrap();
    let tool: String = sqlx::query_scalar("SELECT tool FROM mcp_calls WHERE id = $1").bind(audited).fetch_one(&pool).await.unwrap();
    assert_eq!(tool, "install_plugin");
}

#[sqlx::test(migrations = false)]
async fn install_plugin_refuses_an_unknown_plugin(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    assert!(is_error(&call(&app, &t, "install_plugin", why(json!({ "plugin": "attribution.shapley" }))).await));
}

#[sqlx::test(migrations = false)]
async fn install_plugin_refuses_config_the_plugin_does_not_take(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    assert!(is_error(&call(&app, &t, "install_plugin", why(json!({ "plugin": "sink.identity_resolver", "config": { "url": "ftp://x" } }))).await));
}

#[sqlx::test(migrations = false)]
async fn a_second_open_request_for_the_same_plugin_is_refused(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&request_linear(&app, &t).await);
    assert!(is_error(&request_linear(&app, &t).await));
}

#[sqlx::test(migrations = false)]
async fn install_plugin_refuses_a_plugin_already_installed(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    assert_eq!(api_post(&app, "/api/plugins/attribution.linear/install", json!({ "site": "demo", "actor": "owner" })).await.status, StatusCode::CREATED);
    assert!(is_error(&request_linear(&app, &t).await));
}

// ---- a person decides ----

#[sqlx::test(migrations = false)]
async fn the_approvals_api_lists_open_plugin_requests(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&request_linear(&app, &t).await);
    let q = api_get(&app, "/api/sites/demo/approvals").await.json();
    let r = &q["plugin_requests"][0];
    assert_eq!((r["plugin"].as_str(), r["agent"]["name"].as_str()), (Some("attribution.linear"), Some("Claude Code")), "{q}");
}

#[sqlx::test(migrations = false)]
async fn confirming_a_request_installs_the_plugin(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&request_linear(&app, &t).await);
    let id = open_request(&pool).await;
    let r = api_post(&app, &format!("/api/plugin-requests/{id}/confirm"), json!({ "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let (by, attributor): (String, String) = sqlx::query_as("SELECT installed_by, attributor FROM plugins").fetch_one(&pool).await.unwrap();
    assert_eq!((by.as_str(), attributor.as_str()), ("owner", "linear"));
}

#[sqlx::test(migrations = false)]
async fn confirming_installs_the_requested_config(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&call(&app, &t, "install_plugin", why(json!({ "plugin": "sink.identity_resolver", "config": { "url": "https://r.example.com" } }))).await);
    let id = open_request(&pool).await;
    assert_eq!(api_post(&app, &format!("/api/plugin-requests/{id}/confirm"), json!({ "actor": "owner" })).await.status, StatusCode::OK);
    let url: String = sqlx::query_scalar("SELECT url FROM webhook_subscriptions").fetch_one(&pool).await.unwrap();
    assert_eq!(url, "https://r.example.com");
}

#[sqlx::test(migrations = false)]
async fn a_confirmed_request_cannot_then_be_declined(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&request_linear(&app, &t).await);
    let id = open_request(&pool).await;
    assert_eq!(api_post(&app, &format!("/api/plugin-requests/{id}/confirm"), json!({ "actor": "owner" })).await.status, StatusCode::OK);
    let r = api_post(&app, &format!("/api/plugin-requests/{id}/decline"), json!({ "actor": "owner", "comment": "changed my mind" })).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn declining_leaves_the_plugin_uninstalled(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&request_linear(&app, &t).await);
    let id = open_request(&pool).await;
    let r = api_post(&app, &format!("/api/plugin-requests/{id}/decline"), json!({ "actor": "owner", "comment": "keep last touch" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(installed(&pool).await, 0);
    assert_eq!(count(&pool, "SELECT count(*) FROM plugin_requests WHERE status = 'declined'").await, 1);
}

#[sqlx::test(migrations = false)]
async fn declining_needs_a_reason(pool: PgPool) {
    let (app, t) = setup(&pool).await;
    data(&request_linear(&app, &t).await);
    let id = open_request(&pool).await;
    let r = api_post(&app, &format!("/api/plugin-requests/{id}/decline"), json!({ "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn confirming_an_unknown_request_is_not_found(pool: PgPool) {
    let (app, _) = setup(&pool).await;
    assert_eq!(api_post(&app, "/api/plugin-requests/999/confirm", json!({ "actor": "owner" })).await.status, StatusCode::NOT_FOUND);
}
