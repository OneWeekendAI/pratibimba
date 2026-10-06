//! Install probes: each wizard step is done when Core observes it.

mod common;
use axum::{Router, body::Body, http::{HeaderMap, Request}, routing::get};
use common::*;
use pratibimba::probes::{self, State};
use sqlx::PgPool;

/// A fake owner site on a random local port. Returns its origin.
async fn site(app: Router) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

fn http() -> reqwest::Client {
    probes::client()
}

#[tokio::test]
async fn probe_sdk_detects_header() {
    let origin = site(Router::new().route("/", get(|| async {
        let mut h = HeaderMap::new();
        h.insert("x-prb-sdk", "next/0.1.0".parse().unwrap());
        (h, "<html></html>")
    })))
    .await;
    let s = probes::package(&http(), &origin).await;
    assert_eq!(s.state, State::Done, "{s:?}");
    assert!(s.detail.contains("next/0.1.0"));
}

#[tokio::test]
async fn probe_sdk_detects_script_tag() {
    let origin = site(Router::new().route("/", get(|| async { "<script src=\"/_prb/sdk.js\" data-site=\"demo\"></script>" }))).await;
    assert_eq!(probes::package(&http(), &origin).await.state, State::Done);
}

#[tokio::test]
async fn probe_sdk_missing_is_pending() {
    let origin = site(Router::new().route("/", get(|| async { "<html>no sdk here</html>" }))).await;
    let s = probes::package(&http(), &origin).await;
    assert_eq!(s.state, State::Pending, "{s:?}");
}

#[tokio::test]
async fn probe_unreachable_site_fails() {
    // Nothing listens on port 9 (discard) locally.
    let s = probes::package(&http(), "http://127.0.0.1:9").await;
    assert_eq!(s.state, State::Failed, "{s:?}");
}

#[tokio::test]
async fn probe_reach_ok() {
    let origin = site(Router::new().route("/_prb/manifest/demo", get(|| async {
        axum::Json(serde_json::json!({ "site": "demo", "slots": [{ "key": "hero" }] }))
    })))
    .await;
    let s = probes::reach(&http(), &origin, "demo").await;
    assert_eq!(s.state, State::Done, "{s:?}");
}

#[tokio::test]
async fn probe_reach_404_is_diagnosed() {
    let origin = site(Router::new()).await;
    let s = probes::reach(&http(), &origin, "demo").await;
    assert_eq!(s.state, State::Pending, "{s:?}");
    assert!(s.detail.contains("route.ts"), "{s:?}");
}

#[tokio::test]
async fn probe_reach_wrong_site_fails() {
    let origin = site(Router::new().route("/_prb/manifest/demo", get(|| async { axum::Json(serde_json::json!({ "site": "other" })) }))).await;
    assert_eq!(probes::reach(&http(), &origin, "demo").await.state, State::Failed);
}

#[sqlx::test(migrations = false)]
async fn probe_slot_waits_for_exposure(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app(&pool);
    // Other events (a goal from the script's track()) are not an exposure.
    event(&app, "v", "click", None).await;
    assert_eq!(probes::slot(&pool, "demo").await.unwrap().state, State::Pending);
    let d = decide(&app, "v", false).await.json();
    event(&app, "v", "exposure", d["decision_id"].as_str()).await;
    assert_eq!(probes::slot(&pool, "demo").await.unwrap().state, State::Done);
}

#[sqlx::test(migrations = false)]
async fn probe_live_needs_a_gate_approved_variant_served(pool: PgPool) {
    setup_demo(&pool).await;
    set_holdout(&pool, 0.0).await;
    // `gift` came from `apply`, which is not the dashboard gate.
    let app = admin_app(&pool);
    let d = decide(&app, "v", false).await.json();
    event(&app, "v", "exposure", d["decision_id"].as_str()).await;
    pratibimba::worker::run_once(&pool, chrono::Utc::now()).await.unwrap();
    assert_eq!(probes::live(&pool, "demo").await.unwrap().state, State::Pending, "an apply-approved variant counted as gate-approved");
    let id = api_post(&app, "/api/sites/demo/slots/hero/variants", serde_json::json!({ "key": "ship", "html": "<p>s</p>" })).await.json()["id"].as_i64().unwrap();
    api_post(&app, &format!("/api/variants/{id}/approve"), serde_json::json!({ "approver": "R", "traffic_share": 0.5 })).await;
    assert_eq!(probes::live(&pool, "demo").await.unwrap().state, State::Pending, "approved but never shown");
    for i in 0..200 {
        let s = format!("l{i}");
        let d = decide(&app, &s, false).await.json();
        if d["variant"] == "ship" {
            event(&app, &s, "exposure", d["decision_id"].as_str()).await;
            pratibimba::worker::run_once(&pool, chrono::Utc::now()).await.unwrap();
            assert_eq!(probes::live(&pool, "demo").await.unwrap().state, State::Done);
            return;
        }
    }
    panic!("ship was never served");
}

#[sqlx::test(migrations = false)]
async fn install_endpoint_lists_six_steps(pool: PgPool) {
    setup_demo(&pool).await;
    let origin = site(Router::new()).await;
    sqlx::query("UPDATE sites SET origin = $1").bind(&origin).execute(&pool).await.unwrap();
    let r = api_get(&admin_app(&pool), "/api/sites/demo/install").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let ids: Vec<String> = r.json()["steps"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect();
    assert_eq!(ids, ["framework", "package", "reach", "slot", "agent", "live"]);
    assert_eq!(r.json()["steps"][4]["state"], "pending");
}

#[sqlx::test(migrations = false)]
async fn agent_step_is_done_after_an_mcp_call_runs(pool: PgPool) {
    setup_demo(&pool).await;
    let token = pratibimba::mcp::issue_token(&pool, "demo", "laptop").await.unwrap();
    let refused = pratibimba::mcp::issue_token(&pool, "demo", "other").await.unwrap();
    let call = |t: &str, justification: &str| {
        Request::post("/mcp")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {t}"))
            .body(Body::from(serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "list_slots", "arguments": { "agent": "Claude Code", "justification": justification } } }).to_string()))
            .unwrap()
    };
    // A refused call (no justification) is not a connection.
    send(&app(&pool), call(&refused, "")).await;
    let step = pratibimba::probes::agent(&pool, "demo").await.unwrap();
    assert_eq!(serde_json::to_value(&step).unwrap()["state"], "pending");
    send(&app(&pool), call(&token, "checking the slots before writing a variant")).await;
    let step = serde_json::to_value(pratibimba::probes::agent(&pool, "demo").await.unwrap()).unwrap();
    assert_eq!(step["state"], "done");
    assert!(step["detail"].as_str().unwrap().contains("Claude Code called `list_slots`"), "{step}");
}
