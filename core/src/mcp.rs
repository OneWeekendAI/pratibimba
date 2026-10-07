//! `/mcp`: an open MCP server over the streamable HTTP transport, in the same binary as Core.
//!
//! - **Tokens are per site.** `Authorization: Bearer prb_mcp_…`, issued by
//!   `pratibimba mcp-token <site> <name>` or `POST /api/sites/{site}/mcp-tokens`. Only the
//!   token's SHA-256 is stored. A token sees one site and nothing else; the admin token is not
//!   an MCP token, and an MCP token is not an admin token.
//! - **Reads:** `list_sites`, `get_site`, `list_slots`, `list_variants`, `get_results`,
//!   `list_decisions`, `explain_decision`.
//! - **One write:** `propose_variant`, which always lands as a draft. There is no approve tool,
//!   and the `/api` approve route refuses an MCP token, so an agent cannot approve its own work.
//! - **Slots, as drafts:** `create_slot` makes a slot whose baseline is a DRAFT, so it is in
//!   no manifest and decides nothing until a person approves that baseline in the dashboard
//!   (which activates it). `update_slot` edits a slot only while it is such a draft; a live
//!   slot's settings are the owner's.
//! - **Retirement, proposed:** `retire_variant` records a proposal; the variant keeps serving
//!   until a person confirms it (`POST /api/retire-requests/{id}/confirm`). backend.md leaves the
//!   retire permission open, so this takes the drafts-only floor.
//! - **Plugins, requested:** `list_plugins` reads the compiled-in catalogue as this site has it;
//!   `install_plugin` records a request that a person confirms
//!   (`POST /api/plugin-requests/{id}/confirm`) before anything is installed.
//! - **Audiences:** `list_audiences` reads the site's segments and what each gates;
//!   `create_audience` writes a DRAFT segment. Promotion (the permission to gate a decision) is
//!   the owner's, in the dashboard, exactly like approval.
//! - **The simulator, sandbox only:** `simulate` sends synthetic cohorts through the site's
//!   sandbox (`<site>~sim`, see [`crate::sim`]) and `fast_forward_visitor` moves the sandbox's
//!   clock and shows what one subject would be served then. Neither takes a site argument: the
//!   sandbox id is derived from the token's site, so no call can reach the real site's
//!   decisions, events or models.
//! - **Every tool call is audited** in `mcp_calls`: the token, the JSON-RPC id, the tool, the
//!   agent's self-reported name, who it works for, and its justification. `agent` and
//!   `justification` are required arguments of every tool; a call without them is refused and
//!   the refusal is still recorded.
//!
//! The transport is stateless: each POST carries one JSON-RPC message and gets one JSON
//! response (or 202 for a notification). There is no server-to-client stream, so GET is 405.

use crate::http::{ApiError, AppState, admin};
use crate::{checks, explain, results};
use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{Duration, Utc};
use rand::RngCore;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

pub const TOKEN_PREFIX: &str = "prb_mcp_";
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

pub fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Make a token for `site`, store its hash, and return the token. It is never shown again.
pub async fn issue_token(pool: &PgPool, site: &str, name: &str) -> anyhow::Result<String> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty() && name.chars().count() <= 128, "name the token (1-128 characters), e.g. the agent it is for");
    let mut bytes = [0u8; 24];
    rand::rng().fill_bytes(&mut bytes);
    let token = format!("{TOKEN_PREFIX}{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>());
    let n = sqlx::query("INSERT INTO mcp_tokens (site_id, name, token_hash) SELECT id, $2, $3 FROM sites WHERE id = $1")
        .bind(site)
        .bind(name)
        .bind(hash(&token))
        .execute(pool)
        .await?
        .rows_affected();
    anyhow::ensure!(n == 1, "unknown site {site:?}");
    Ok(token)
}

struct Caller {
    token_id: i64,
    site: String,
}

async fn authenticate(pool: &PgPool, headers: &HeaderMap) -> Result<Option<Caller>, sqlx::Error> {
    let Some(t) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(str::trim) else {
        return Ok(None);
    };
    if !t.starts_with(TOKEN_PREFIX) {
        return Ok(None);
    }
    let row: Option<(i64, String)> = sqlx::query_as("SELECT id, site_id FROM mcp_tokens WHERE token_hash = $1 AND revoked_at IS NULL")
        .bind(hash(t))
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(token_id, site)| Caller { token_id, site }))
}

fn rpc_result(id: &Value, result: Value) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })).into_response()
}

pub async fn handler(State(state): State<Arc<AppState>>, headers: HeaderMap, body: Bytes) -> Response {
    let caller = match authenticate(&state.pool, &headers).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
                Json(json!({ "error": "an MCP token for this site is required (Authorization: Bearer prb_mcp_…)" })),
            )
                .into_response();
        }
        Err(e) => return ApiError::from(e).into_response(),
    };
    let Ok(msg) = serde_json::from_slice::<Value>(&body) else {
        return rpc_error(&Value::Null, -32700, "parse error");
    };
    let Some(method) = msg.get("method").and_then(Value::as_str) else {
        return rpc_error(msg.get("id").unwrap_or(&Value::Null), -32600, "expected one JSON-RPC request or notification");
    };
    // A notification (no id) gets no response body.
    let Some(id) = msg.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(PROTOCOL_VERSIONS[0]);
            rpc_result(
                &id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "pratibimba", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": INSTRUCTIONS,
                }),
            )
        }
        "ping" => rpc_result(&id, json!({})),
        "tools/list" => rpc_result(&id, json!({ "tools": catalogue() })),
        "tools/call" => tools_call(&state, &caller, &id, &params).await,
        _ => rpc_error(&id, -32601, &format!("method not found: {method}")),
    }
}

const INSTRUCTIONS: &str = "Pratibimba personalises page slots with a bandit over approved variants. \
This token sees one site. Read slots, variants, results and decision explanations freely. \
propose_variant submits a DRAFT: a person approves it in the dashboard; you cannot. \
create_slot makes a DRAFT slot (live only once a person approves its baseline); update_slot edits only draft slots. \
retire_variant PROPOSES a retirement; the variant keeps serving until a person confirms. \
install_plugin REQUESTS a plugin; nothing is installed until a person confirms. \
simulate and fast_forward_visitor run only in the site's sandbox, never on real traffic. \
Every tool needs `agent` (your name, e.g. \"Claude Code\") and a one-line `justification`; both are \
recorded and shown to the reviewer. Reviewer comments come back on list_variants.";

// ---- the catalogue ----

const TOOLS: [&str; 17] = [
    "list_sites",
    "get_site",
    "list_slots",
    "list_variants",
    "get_results",
    "list_decisions",
    "explain_decision",
    "propose_variant",
    "simulate",
    "fast_forward_visitor",
    "list_audiences",
    "create_audience",
    "create_slot",
    "update_slot",
    "retire_variant",
    "list_plugins",
    "install_plugin",
];

fn tool(name: &str, description: &str, props: Value, required: &[&str]) -> Value {
    let mut p = json!({
        "agent": { "type": "string", "description": "Your name as an agent, e.g. \"Claude Code\" or \"Cursor\". Shown on the approval card." },
        "justification": { "type": "string", "description": "One line: why you are making this call. Required; recorded in the audit log." },
        "on_behalf_of": { "type": "string", "description": "The person you are working for, if known." },
    });
    p.as_object_mut().unwrap().extend(props.as_object().unwrap().clone());
    let mut req = vec!["agent", "justification"];
    req.extend_from_slice(required);
    json!({ "name": name, "description": description,
            "inputSchema": { "type": "object", "properties": p, "required": req, "additionalProperties": false } })
}

fn catalogue() -> Vec<Value> {
    let slot = json!({ "type": "string", "description": "The slot key, e.g. \"hero\"." });
    vec![
        tool("list_sites", "The site this token is scoped to, with its slot count and pending drafts.", json!({}), &[]),
        tool("get_site", "The site's origin, holdout fraction, forbidden claims, facts (its facts.md: the only figures your copy may use) and slots.",
             json!({ "site": { "type": "string", "description": "Optional; must be this token's site." } }), &[]),
        tool("list_slots", "Every slot: selector, success metric, current champion and variant counts by status.", json!({}), &[]),
        tool("list_variants", "A slot's variants (any status) with their HTML, approval details and the reviewer comment thread.",
             json!({ "slot": slot }), &["slot"]),
        tool("get_results", "Per-slot and per-variant results over closed attribution windows, with lift against the holdout.",
             json!({ "slot": { "type": "string", "description": "Optional: one slot only." },
                     "days": { "type": "integer", "minimum": 1, "description": "Optional: only the last N days (default: all time)." } }), &[]),
        tool("list_decisions", "Recent decisions, newest first.",
             json!({ "slot": { "type": "string" }, "variant": { "type": "string" },
                     "outcome": { "type": "string", "enum": ["all", "converted", "missed", "open", "unexposed", "holdout"] },
                     "limit": { "type": "integer", "minimum": 1, "maximum": 500, "description": "Default 50." } }), &[]),
        tool("explain_decision", "Why one decision showed what it showed: features, scores, a replay check, the counterfactual and a plain sentence.",
             json!({ "id": { "type": "string", "description": "The decision id (a UUID)." } }), &["id"]),
        tool("propose_variant", "Submit a new variant for a slot. It lands as a DRAFT and is never served until a person approves it in the dashboard. Returns the constraint-check report a reviewer will see.",
             json!({ "slot": slot, "key": { "type": "string", "description": "A new lowercase slug for the variant (a-z, 0-9, - and _)." },
                     "html": { "type": "string", "description": "The HTML fragment that replaces the slot's content. No scripts or event handlers." },
                     "note": { "type": "string", "description": "Optional note to the reviewer; starts the comment thread." } }),
             &["slot", "key", "html"]),
        tool("create_slot", "Declare a new swap slot. It lands as a DRAFT: its baseline (the page's own content) is a draft, so the slot is in no manifest and decides nothing until a person approves that baseline in the dashboard, which activates it. Then propose variants for it with propose_variant.",
             json!({ "key": { "type": "string", "description": "A new lowercase slug for the slot (a-z, 0-9, - and _), e.g. \"promo\"." },
                     "selector": { "type": "string", "description": "The CSS selector of the element the slot replaces, e.g. \"#promo\"." },
                     "metric": { "type": "string", "description": "Optional: the event that counts as success (default \"click\")." },
                     "attribution_window_secs": { "type": "integer", "minimum": 1, "description": "Optional: how long after a decision a success still counts (default 1800)." },
                     "decision_ttl_secs": { "type": "integer", "minimum": 0, "description": "Optional: how long a visitor keeps the same decision (default 60)." },
                     "note": { "type": "string", "description": "Optional note to the reviewer; starts the comment thread." } }),
             &["key", "selector"]),
        tool("update_slot", "Change a DRAFT slot's selector, metric, attribution window or decision TTL (only the fields you send). A live slot's settings belong to the site owner and are refused.",
             json!({ "slot": slot,
                     "selector": { "type": "string" }, "metric": { "type": "string" },
                     "attribution_window_secs": { "type": "integer", "minimum": 1 }, "decision_ttl_secs": { "type": "integer", "minimum": 0 } }),
             &["slot"]),
        tool("retire_variant", "PROPOSE retiring a live variant (your justification is the reason the reviewer reads). It keeps serving until a person confirms in the dashboard; you cannot retire it yourself. The baseline cannot be retired. The answer appears on list_variants.",
             json!({ "slot": slot, "variant": { "type": "string", "description": "The variant key." } }),
             &["slot", "variant"]),
        tool("list_plugins", "The compiled-in plugin catalogue as this site has it: each plugin's kind, whether it is a de-anonymiser, its install (or null) and your latest request with its answer; plus the site's warnings.", json!({}), &[]),
        tool("install_plugin", "REQUEST installing a plugin on this site (your justification is the reason the reviewer reads). Nothing is installed until a person confirms in the dashboard API; you cannot install it yourself. The answer appears on list_plugins.",
             json!({ "plugin": { "type": "string", "description": "The plugin name from list_plugins, e.g. \"attribution.linear\"." },
                     "config": { "type": "object", "description": "Optional: the plugin's config, e.g. {\"url\": \"https://...\"} for an event sink." } }),
             &["plugin"]),
        tool("list_audiences", "The site's segments (draft, active, audience) with their predicates, and the variants each promoted audience restricts.", json!({}), &[]),
        tool("create_audience", "Draft a segment from a predicate over behaviour and situation features (returning, mobile, from_ad, is_night_local, is_weekend_local, is_business_hours_local, hour_local: [from, to]). It lands as a DRAFT: a person promotes it to an audience in the dashboard before it can restrict any variant. Demographic attributes are refused.",
             json!({ "predicate": { "type": "object", "description": "e.g. {\"returning\": true, \"is_night_local\": true}" },
                     "name": { "type": "string", "description": "Optional; a plain description is generated from the predicate." } }),
             &["predicate"]),
        tool("simulate", "Run the simulator in this site's SANDBOX (a copy of the site with its own clock and models; the real site is never touched). Sends a synthetic cohort with a planted truth; with `days`, time-travels a day at a time with the cohort arriving each day. Creates the sandbox on first use. Returns each day and the sandbox's results as of its clock.",
             json!({ "cohort": { "type": "object", "description": "{visitors (<= 5000), seed, rates: {slot: {variant: p}}, returning_rates, default_rate, returning, mobile, from_ad}" },
                     "days": { "type": "number", "minimum": 0, "maximum": 366, "description": "Optional: simulated days to run. Omit for one batch at the current clock." },
                     "fresh": { "type": "boolean", "description": "Optional: wipe the sandbox first and start its clock at now." } }),
             &["cohort"]),
        tool("fast_forward_visitor", "Time-travel one subject in this site's SANDBOX: move the sandbox's clock forward (closing attribution windows and training its models, as the worker would), then decide every slot for the subject at the new clock. The sandbox has one clock, so every simulated visitor moves with it. Never touches the real site.",
             json!({ "subject": { "type": "string", "description": "The subject id, e.g. one from list_decisions on the sandbox or your own test visitor." },
                     "days": { "type": "number", "description": "How far to move the clock (or use `seconds`)." },
                     "seconds": { "type": "integer" },
                     "ctx": { "type": "object", "description": "Optional decision context {returning, mobile, from_ad, tz_offset, tz (an IANA zone name)}. Default: a returning visitor." } }),
             &["subject"]),
    ]
}

// ---- tools/call, with the audit stub ----

fn tool_result(v: Value) -> Value {
    json!({ "content": [{ "type": "text", "text": v.to_string() }], "structuredContent": v, "isError": false })
}

fn tool_error(msg: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": msg }], "isError": true })
}

fn text_arg(args: &Map<String, Value>, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

struct Audit<'a> {
    caller: &'a Caller,
    call_id: &'a Value,
    tool: &'a str,
    agent: Option<String>,
    on_behalf_of: Option<String>,
    justification: Option<String>,
    arguments: &'a Value,
}

impl Audit<'_> {
    async fn record(&self, pool: &PgPool, outcome: &str, error: Option<&str>) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            "INSERT INTO mcp_calls (token_id, site_id, call_id, tool, agent, on_behalf_of, justification, arguments, outcome, error)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
        )
        .bind(self.caller.token_id)
        .bind(&self.caller.site)
        .bind(self.call_id)
        .bind(self.tool)
        .bind(self.agent.as_deref().unwrap_or(""))
        .bind(&self.on_behalf_of)
        .bind(&self.justification)
        .bind(self.arguments)
        .bind(outcome)
        .bind(error)
        .fetch_one(pool)
        .await
    }
}

async fn tools_call(state: &AppState, caller: &Caller, id: &Value, params: &Value) -> Response {
    let pool = &state.pool;
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
    let empty = Map::new();
    let args = arguments.as_object().unwrap_or(&empty);
    let audit = Audit {
        caller,
        call_id: id,
        tool: if name.is_empty() { "(none)" } else { name },
        agent: text_arg(args, "agent").map(|a| a.chars().take(128).collect()),
        on_behalf_of: text_arg(args, "on_behalf_of").map(|a| a.chars().take(128).collect()),
        justification: text_arg(args, "justification").map(|j| j.chars().take(2000).collect()),
        arguments: &arguments,
    };
    if !arguments.is_object() {
        let msg = "arguments must be an object";
        return match audit.record(pool, "refused", Some(msg)).await {
            Ok(_) => rpc_error(id, -32602, msg),
            Err(e) => ApiError::from(e).into_response(),
        };
    }
    if !TOOLS.contains(&name) {
        let msg = format!("unknown tool {name:?}; there is no approve tool, approval happens in the dashboard");
        return match audit.record(pool, "refused", Some(&msg)).await {
            Ok(_) => rpc_error(id, -32602, &msg),
            Err(e) => ApiError::from(e).into_response(),
        };
    }
    if audit.agent.is_none() || audit.justification.is_none() {
        let msg = "every tool call needs `agent` (your name) and a non-empty `justification`";
        return match audit.record(pool, "refused", Some(msg)).await {
            Ok(_) => rpc_result(id, tool_error(msg)),
            Err(e) => ApiError::from(e).into_response(),
        };
    }
    let row = match audit.record(pool, "ok", None).await {
        Ok(r) => r,
        Err(e) => return ApiError::from(e).into_response(),
    };
    let mut rest = args.clone();
    for k in ["agent", "justification", "on_behalf_of"] {
        rest.remove(k);
    }
    let who = Who { agent: audit.agent.as_deref().unwrap_or(""), justification: audit.justification.as_deref().unwrap_or("") };
    let out = run(state, caller, name, Value::Object(rest), row, &who).await;
    match out {
        Ok(mut v) => {
            // The audit stub, returned on every result so the agent sees what was recorded.
            if let Some(o) = v.as_object_mut() {
                o.insert(
                    "audit".into(),
                    json!({ "id": row, "call_id": id, "tool": name, "agent": audit.agent, "on_behalf_of": audit.on_behalf_of, "justification": audit.justification }),
                );
            }
            rpc_result(id, tool_result(v))
        }
        Err(e) => {
            let msg = match e {
                ApiError::NotFound(m) | ApiError::BadRequest(m) | ApiError::Conflict(m) | ApiError::Gone(m) => m,
                ApiError::Internal(e) => {
                    tracing::error!(error = %e, tool = name, "mcp tool failed");
                    "internal error".into()
                }
            };
            if let Err(e) = sqlx::query("UPDATE mcp_calls SET outcome = 'error', error = $2 WHERE id = $1").bind(row).bind(&msg).execute(pool).await {
                return ApiError::from(e).into_response();
            }
            rpc_result(id, tool_error(&msg))
        }
    }
}

fn parse<T: DeserializeOwned>(v: Value) -> Result<T, ApiError> {
    serde_json::from_value(v).map_err(|e| ApiError::BadRequest(format!("bad arguments: {e}")))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SiteArg {
    site: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SlotArg {
    slot: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultsArgs {
    slot: Option<String>,
    days: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionsArgs {
    slot: Option<String>,
    variant: Option<String>,
    outcome: Option<String>,
    limit: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdArg {
    id: String,
}

/// Exactly the fields a proposal may carry. Status, approver, share and baseline are not
/// among them, so an attempt to pass one fails the call instead of being ignored.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    slot: String,
    key: String,
    html: String,
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SimulateArgs {
    cohort: crate::sim::Cohort,
    days: Option<f64>,
    #[serde(default)]
    fresh: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FastForwardArgs {
    subject: String,
    days: Option<f64>,
    seconds: Option<i64>,
    ctx: Option<crate::http::decide::Ctx>,
}

/// Exactly what an agent may set on a new slot: a swap slot, never a picker or an on-device one.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewSlot {
    key: String,
    selector: String,
    metric: Option<String>,
    attribution_window_secs: Option<i32>,
    decision_ttl_secs: Option<i32>,
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SlotUpdate {
    slot: String,
    selector: Option<String>,
    metric: Option<String>,
    attribution_window_secs: Option<i32>,
    decision_ttl_secs: Option<i32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginArgs {
    plugin: String,
    #[serde(default)]
    config: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetireArgs {
    slot: String,
    variant: String,
}

fn check_slot_settings(selector: &str, metric: &str, window: i32, ttl: i32) -> Result<(), ApiError> {
    if selector.trim().is_empty() || selector.len() > 512 {
        return Err(ApiError::BadRequest("selector must be a CSS selector of 1 to 512 characters".into()));
    }
    if !crate::site_file::is_event_name(metric) {
        return Err(ApiError::BadRequest("metric must be an event name (lowercase letters, digits, _ and :)".into()));
    }
    if window <= 0 || ttl < 0 {
        return Err(ApiError::BadRequest("attribution_window_secs must be > 0 and decision_ttl_secs >= 0".into()));
    }
    Ok(())
}

/// A slot is a draft while its baseline awaits approval. Nothing else of it can be approved
/// before that (the approve route refuses), so a draft slot serves nobody.
async fn is_draft_slot(pool: &PgPool, slot_id: i64) -> Result<bool, ApiError> {
    Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM variants WHERE slot_id = $1 AND is_baseline AND status = 'draft')")
    .bind(slot_id)
    .fetch_one(pool)
    .await?)
}

/// `create_slot`: a slot whose baseline is a draft, so nothing of it reaches a visitor.
async fn create_slot(pool: &PgPool, site: &str, a: NewSlot, call_row: i64, agent: &str) -> Result<Value, ApiError> {
    if !crate::site_file::is_slug(&a.key) {
        return Err(ApiError::BadRequest("key must be a lowercase slug (a-z, 0-9, - and _)".into()));
    }
    let metric = a.metric.unwrap_or_else(|| "click".into());
    let window = a.attribution_window_secs.unwrap_or_else(crate::site_file::default_window);
    let ttl = a.decision_ttl_secs.unwrap_or_else(crate::site_file::default_ttl);
    let selector = a.selector.trim().to_string();
    check_slot_settings(&selector, &metric, window, ttl)?;
    let mut tx = pool.begin().await?;
    let slot_id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO slots (site_id, key, selector, metric, attribution_window_secs, decision_ttl_secs, kind, created_by, mcp_call_id)
         VALUES ($1, $2, $3, $4, $5, $6, 'swap', $7, $8) ON CONFLICT (site_id, key) DO NOTHING RETURNING id",
    )
    .bind(site)
    .bind(&a.key)
    .bind(&selector)
    .bind(&metric)
    .bind(window)
    .bind(ttl)
    .bind(format!("mcp:{agent}"))
    .bind(call_row)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(slot_id) = slot_id else {
        return Err(ApiError::Conflict(format!("site {site:?} already has a slot called {:?}; pick a new key or use update_slot", a.key)));
    };
    sqlx::query("INSERT INTO models (slot_id) VALUES ($1)").bind(slot_id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO model_versions (slot_id, version, state, reason) SELECT slot_id, version, state, 'init' FROM models WHERE slot_id = $1")
        .bind(slot_id)
        .execute(&mut *tx)
        .await?;
    // The baseline is the page's own content. As a draft, it keeps the whole slot dark.
    let baseline: i64 = sqlx::query_scalar(
        "INSERT INTO variants (slot_id, key, payload, is_baseline, author, status, mcp_call_id)
         VALUES ($1, 'baseline', NULL, true, 'agent', 'draft', $2) RETURNING id",
    )
    .bind(slot_id)
    .bind(call_row)
    .fetch_one(&mut *tx)
    .await?;
    if let Some(note) = a.note.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        sqlx::query("INSERT INTO approvals (variant_id, action, actor, comment) VALUES ($1, 'comment', $2, $3)")
            .bind(baseline)
            .bind(agent.chars().take(128).collect::<String>())
            .bind(note)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(json!({
        "slot": a.key, "selector": selector, "metric": metric, "attribution_window_secs": window, "decision_ttl_secs": ttl,
        "status": "draft", "baseline_id": baseline,
        "next": "Draft slot saved. It reaches no visitor until a person approves its baseline in the dashboard. Propose variants for it with propose_variant.",
    }))
}

/// `update_slot`: only a draft slot's settings are the agent's to change.
async fn update_slot(pool: &PgPool, site: &str, u: SlotUpdate) -> Result<Value, ApiError> {
    let slot_id = slot_id(pool, site, &u.slot).await?;
    if u.selector.is_none() && u.metric.is_none() && u.attribution_window_secs.is_none() && u.decision_ttl_secs.is_none() {
        return Err(ApiError::BadRequest("send at least one of selector, metric, attribution_window_secs, decision_ttl_secs".into()));
    }
    if !is_draft_slot(pool, slot_id).await? {
        return Err(ApiError::Conflict(format!(
            "slot {:?} is live: its settings belong to the site owner (site file or dashboard). Tell the person what to change instead.",
            u.slot
        )));
    }
    let (selector, metric, window, ttl): (String, String, i32, i32) =
        sqlx::query_as("SELECT selector, metric, attribution_window_secs, decision_ttl_secs FROM slots WHERE id = $1").bind(slot_id).fetch_one(pool).await?;
    let selector = u.selector.map(|s| s.trim().to_string()).unwrap_or(selector);
    let metric = u.metric.unwrap_or(metric);
    let window = u.attribution_window_secs.unwrap_or(window);
    let ttl = u.decision_ttl_secs.unwrap_or(ttl);
    check_slot_settings(&selector, &metric, window, ttl)?;
    sqlx::query("UPDATE slots SET selector = $2, metric = $3, attribution_window_secs = $4, decision_ttl_secs = $5 WHERE id = $1")
        .bind(slot_id)
        .bind(&selector)
        .bind(&metric)
        .bind(window)
        .bind(ttl)
        .execute(pool)
        .await?;
    Ok(json!({ "slot": u.slot, "selector": selector, "metric": metric, "attribution_window_secs": window, "decision_ttl_secs": ttl, "status": "draft" }))
}

/// `retire_variant`: a proposal only. The variant's status is never touched here.
async fn retire_variant(pool: &PgPool, site: &str, a: RetireArgs, call_row: i64, who: &Who<'_>) -> Result<Value, ApiError> {
    let slot_id = slot_id(pool, site, &a.slot).await?;
    let row: Option<(i64, String, bool)> = sqlx::query_as("SELECT id, status, is_baseline FROM variants WHERE slot_id = $1 AND key = $2")
        .bind(slot_id)
        .bind(&a.variant)
        .fetch_optional(pool)
        .await?;
    let Some((variant_id, status, is_baseline)) = row else {
        return Err(ApiError::NotFound(format!("slot {:?} has no variant {:?}", a.slot, a.variant)));
    };
    if is_baseline {
        return Err(ApiError::BadRequest("the baseline is the page's own content and cannot be retired".into()));
    }
    if status != "approved" && status != "paused" {
        return Err(ApiError::Conflict(format!("variant {:?} is {status}; only a live variant can be retired", a.variant)));
    }
    let id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO retire_requests (variant_id, mcp_call_id, agent, reason) VALUES ($1, $2, $3, $4)
         ON CONFLICT (variant_id) WHERE status = 'pending' DO NOTHING RETURNING id",
    )
    .bind(variant_id)
    .bind(call_row)
    .bind(who.agent.chars().take(128).collect::<String>())
    .bind(who.justification)
    .fetch_optional(pool)
    .await?;
    let Some(id) = id else {
        return Err(ApiError::Conflict(format!("variant {:?} already has an open retirement proposal; see list_variants", a.variant)));
    };
    Ok(json!({
        "id": id, "slot": a.slot, "variant": a.variant, "status": "proposed", "variant_status": status,
        "next": "Proposal recorded. The variant keeps serving until a person confirms the retirement in the dashboard; the answer appears on list_variants.",
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AudienceArgs {
    predicate: Value,
    name: Option<String>,
}

async fn list_audiences(pool: &PgPool, site: &str) -> Result<Value, ApiError> {
    let rows: Vec<(i64, String, Value, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, name, predicate, status, origin, promoted_by FROM segments WHERE site_id = $1 AND status <> 'archived' ORDER BY id",
    )
    .bind(site)
    .fetch_all(pool)
    .await?;
    let gates: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT v.audience_id, s.key, v.key FROM variants v JOIN slots s ON s.id = v.slot_id
          WHERE s.site_id = $1 AND v.audience_id IS NOT NULL AND v.status = 'approved' ORDER BY s.key, v.key",
    )
    .bind(site)
    .fetch_all(pool)
    .await?;
    let audiences: Vec<Value> = rows
        .into_iter()
        .map(|(id, name, predicate, status, origin, promoted_by)| {
            let g: Vec<Value> = gates.iter().filter(|x| x.0 == id).map(|x| json!({ "slot": x.1, "variant": x.2 })).collect();
            json!({ "id": id, "name": name, "predicate": predicate, "status": status, "origin": origin, "promoted_by": promoted_by, "gates": g })
        })
        .collect();
    Ok(json!({ "audiences": audiences }))
}

/// A draft segment, authored by the agent. Never promoted here.
async fn create_audience(pool: &PgPool, site: &str, a: AudienceArgs, agent: &str) -> Result<Value, ApiError> {
    use crate::insights::segments::Predicate;
    let p = Predicate::parse(&a.predicate).map_err(ApiError::BadRequest)?;
    let name = a.name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| p.describe());
    if name.chars().count() > 200 {
        return Err(ApiError::BadRequest("name is at most 200 characters".into()));
    }
    let row: Option<(i64,)> = sqlx::query_as(
        "INSERT INTO segments (site_id, name, predicate, predicate_key, origin, status, created_by)
         VALUES ($1, $2, $3, $4, 'authored', 'draft', $5) ON CONFLICT (site_id, predicate_key) DO NOTHING RETURNING id",
    )
    .bind(site)
    .bind(&name)
    .bind(p.to_json())
    .bind(p.key())
    .bind(format!("mcp:{agent}"))
    .fetch_optional(pool)
    .await?;
    let (id,) = row.ok_or_else(|| ApiError::Conflict("a segment with this predicate already exists; see list_audiences".into()))?;
    Ok(json!({ "id": id, "name": name, "predicate": p.to_json(), "status": "draft",
               "next": "A person promotes it to an audience in the Pratibimba dashboard; until then it restricts nothing." }))
}

/// `simulate`: the sandbox of the token's site only.
async fn simulate(pool: &PgPool, site: &str, a: SimulateArgs) -> Result<Value, ApiError> {
    use crate::sim;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sites WHERE id = $1 AND sandbox_of = $2)")
        .bind(sim::sandbox_id(site))
        .bind(site)
        .fetch_one(pool)
        .await?;
    if a.fresh || !exists {
        sim::reset(pool, site, Utc::now()).await?;
    }
    let days = match a.days {
        Some(d) if d.is_finite() && d > 0.0 => sim::advance(pool, site, (d * sim::DAY as f64).round() as i64, Some(&a.cohort)).await?,
        Some(d) if d == 0.0 => vec![sim::visitors(pool, site, &a.cohort).await?],
        Some(_) => return Err(ApiError::BadRequest("days must be a number from 0 to 366".into())),
        None => vec![sim::visitors(pool, site, &a.cohort).await?],
    };
    Ok(json!({ "sandbox": sim::sandbox_id(site), "days": days, "status": sim::status(pool, site).await? }))
}

/// `fast_forward_visitor`: move the sandbox's clock, then decide every slot for one subject.
async fn fast_forward(pool: &PgPool, site: &str, a: FastForwardArgs) -> Result<Value, ApiError> {
    use crate::{http::decide, sim};
    let secs = match (a.days, a.seconds) {
        (Some(d), None) if d.is_finite() => (d * sim::DAY as f64).round() as i64,
        (None, Some(s)) => s,
        _ => return Err(ApiError::BadRequest("send exactly one of days or seconds".into())),
    };
    let before = sim::clock(pool, site).await?;
    let days = sim::advance(pool, site, secs, None).await?;
    let clock = sim::clock(pool, site).await?;
    let sandbox = sim::sandbox_id(site);
    let ctx = a.ctx.unwrap_or(decide::Ctx { returning: true, ..Default::default() });
    let state = AppState::with_seed(pool.clone(), rand::rng().next_u64());
    let slots: Vec<String> = sqlx::query_scalar("SELECT key FROM slots WHERE site_id = $1 ORDER BY id").bind(&sandbox).fetch_all(pool).await?;
    let mut decisions = vec![];
    for slot in slots {
        let req = decide::DecideRequest { site: sandbox.clone(), slot: slot.clone(), subject: a.subject.clone(), ctx: ctx.clone() };
        decisions.push(match decide::decide(&state, &req).await {
            Ok(d) => json!(d),
            Err(ApiError::Conflict(m) | ApiError::NotFound(m)) => json!({ "slot": slot, "error": m }),
            Err(e) => return Err(e),
        });
    }
    let settled: u64 = days.iter().map(|d| d.credited + d.settled_negative).sum();
    Ok(json!({ "sandbox": sandbox, "subject": a.subject, "clock_before": before, "clock": clock, "settled": settled, "decisions": decisions }))
}

/// Who made a call, as audited: the agent's name and its justification.
struct Who<'a> {
    agent: &'a str,
    justification: &'a str,
}

async fn run(state: &AppState, caller: &Caller, tool: &str, args: Value, call_row: i64, who: &Who<'_>) -> Result<Value, ApiError> {
    let agent = who.agent;
    let pool = &state.pool;
    let site = caller.site.as_str();
    match tool {
        "list_sites" => {
            parse::<NoArgs>(args)?;
            let all = admin::site_list(pool).await?;
            Ok(json!({ "sites": all.into_iter().filter(|s| s["id"] == site).collect::<Vec<_>>() }))
        }
        "get_site" => {
            let a: SiteArg = parse(args)?;
            if let Some(s) = a.site.filter(|s| s != site) {
                return Err(ApiError::NotFound(format!("this token is scoped to site {site:?}, not {s:?}")));
            }
            get_site(pool, site).await
        }
        "list_slots" => {
            parse::<NoArgs>(args)?;
            list_slots(pool, site).await
        }
        "list_variants" => {
            let a: SlotArg = parse(args)?;
            list_variants(pool, site, &a.slot).await
        }
        "get_results" => {
            let a: ResultsArgs = parse(args)?;
            let now = Utc::now();
            let since = match a.days {
                Some(d) if d >= 1 => now - Duration::days(d),
                Some(_) => return Err(ApiError::BadRequest("days must be at least 1".into())),
                None => results::ALL_TIME,
            };
            let mut r = results::site(pool, site, since, now).await?.ok_or_else(|| ApiError::NotFound(format!("unknown site {site:?}")))?;
            if let Some(slot) = &a.slot {
                r.slots.retain(|s| &s.key == slot);
                if r.slots.is_empty() {
                    return Err(ApiError::NotFound(format!("unknown slot {slot:?} on site {site:?}")));
                }
            }
            Ok(json!(r))
        }
        "list_decisions" => {
            let a: DecisionsArgs = parse(args)?;
            let rows = admin::decision_list(pool, site, a.slot.as_deref(), a.variant.as_deref(), a.outcome.as_deref(), a.limit.unwrap_or(50).clamp(1, 500)).await?;
            Ok(json!({ "decisions": rows }))
        }
        "explain_decision" => {
            let a: IdArg = parse(args)?;
            let id = Uuid::parse_str(a.id.trim()).map_err(|_| ApiError::BadRequest(format!("{:?} is not a decision id (a UUID)", a.id)))?;
            match explain::detail(pool, id, Utc::now()).await? {
                Some(d) if d.site == site => Ok(json!(d)),
                _ => Err(ApiError::NotFound(format!("unknown decision {id}"))),
            }
        }
        "propose_variant" => {
            let p: Proposal = parse(args)?;
            propose(pool, site, p, call_row, agent).await
        }
        "simulate" => simulate(pool, site, parse(args)?).await,
        "list_audiences" => {
            parse::<NoArgs>(args)?;
            list_audiences(pool, site).await
        }
        "create_audience" => create_audience(pool, site, parse(args)?, agent).await,
        "fast_forward_visitor" => fast_forward(pool, site, parse(args)?).await,
        "create_slot" => create_slot(pool, site, parse(args)?, call_row, agent).await,
        "update_slot" => update_slot(pool, site, parse(args)?).await,
        "retire_variant" => retire_variant(pool, site, parse(args)?, call_row, who).await,
        "list_plugins" => {
            parse::<NoArgs>(args)?;
            crate::plugins::listing(pool, site).await
        }
        "install_plugin" => {
            let a: PluginArgs = parse(args)?;
            crate::plugins::request(pool, site, &a.plugin, a.config.as_ref(), call_row, who.agent, who.justification).await
        }
        _ => unreachable!("tools/call checks the catalogue first"),
    }
}

async fn slot_id(pool: &PgPool, site: &str, slot: &str) -> Result<i64, ApiError> {
    sqlx::query_scalar("SELECT id FROM slots WHERE site_id = $1 AND key = $2")
        .bind(site)
        .bind(slot)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("unknown slot {slot:?} on site {site:?}")))
}

async fn get_site(pool: &PgPool, site: &str) -> Result<Value, ApiError> {
    let (origin, holdout, forbidden, facts): (String, f32, Vec<String>, Option<String>) =
        sqlx::query_as("SELECT origin, holdout_fraction, forbidden_claims, facts FROM sites WHERE id = $1").bind(site).fetch_one(pool).await?;
    let slots: Vec<(String, String, String, i32)> =
        sqlx::query_as("SELECT key, selector, metric, attribution_window_secs FROM slots WHERE site_id = $1 ORDER BY key").bind(site).fetch_all(pool).await?;
    Ok(json!({
        "id": site, "origin": origin, "holdout_fraction": holdout, "forbidden_claims": forbidden, "facts": facts,
        "slots": slots.into_iter().map(|(key, selector, metric, w)| json!({ "key": key, "selector": selector, "metric": metric, "attribution_window_secs": w })).collect::<Vec<_>>(),
    }))
}

async fn list_slots(pool: &PgPool, site: &str) -> Result<Value, ApiError> {
    let r = results::site(pool, site, results::ALL_TIME, Utc::now()).await?.ok_or_else(|| ApiError::NotFound(format!("unknown site {site:?}")))?;
    let rows: Vec<(i64, String)> = sqlx::query_as("SELECT id, selector FROM slots WHERE site_id = $1").bind(site).fetch_all(pool).await?;
    let slots: Vec<Value> = r
        .slots
        .iter()
        .map(|s| {
            let by = |st: &str| s.variants.iter().filter(|v| v.status == st).count();
            json!({
                "key": s.key, "metric": s.metric,
                "selector": rows.iter().find(|(id, _)| *id == s.id).map(|(_, sel)| sel.clone()),
                "champion": admin::champion(&s.variants).map(|v| v.key.clone()),
                "variants": { "approved": by("approved"), "draft": by("draft"), "paused": by("paused"), "rejected": by("rejected"), "retired": by("retired") },
            })
        })
        .collect();
    Ok(json!({ "slots": slots }))
}

async fn list_variants(pool: &PgPool, site: &str, slot: &str) -> Result<Value, ApiError> {
    let slot_id = slot_id(pool, site, slot).await?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(i64, String, String, String, bool, Option<String>, Option<f32>, Option<String>, Option<chrono::DateTime<Utc>>, chrono::DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, key, status, author, is_baseline, payload, traffic_share, approved_by, approved_at, created_at
           FROM variants WHERE slot_id = $1 ORDER BY id",
    )
    .bind(slot_id)
    .fetch_all(pool)
    .await?;
    #[allow(clippy::type_complexity)]
    let notes: Vec<(i64, String, String, Option<String>, chrono::DateTime<Utc>)> = sqlx::query_as(
        "SELECT a.variant_id, a.action, a.actor, a.comment, a.created_at FROM approvals a JOIN variants v ON v.id = a.variant_id
          WHERE v.slot_id = $1 ORDER BY a.created_at, a.id",
    )
    .bind(slot_id)
    .fetch_all(pool)
    .await?;
    #[allow(clippy::type_complexity)]
    let retires: Vec<(i64, i64, String, String, Option<String>, chrono::DateTime<Utc>)> = sqlx::query_as(
        "SELECT DISTINCT ON (r.variant_id) r.variant_id, r.id, r.status, r.reason, r.decided_by, r.created_at
           FROM retire_requests r JOIN variants v ON v.id = r.variant_id
          WHERE v.slot_id = $1 ORDER BY r.variant_id, r.created_at DESC, r.id DESC",
    )
    .bind(slot_id)
    .fetch_all(pool)
    .await?;
    let variants: Vec<Value> = rows
        .into_iter()
        .map(|(id, key, status, author, base, html, share, by, at, created)| {
            let retire = retires
                .iter()
                .find(|r| r.0 == id)
                .map(|(_, rid, st, reason, by, at)| json!({ "id": rid, "status": st, "reason": reason, "decided_by": by, "at": at }));
            let comments: Vec<Value> = notes
                .iter()
                .filter(|n| n.0 == id)
                .map(|(_, action, actor, comment, at)| json!({ "action": action, "actor": actor, "comment": comment, "at": at }))
                .collect();
            json!({ "id": id, "key": key, "status": status, "author": author, "is_baseline": base, "html": html,
                    "traffic_share": share, "approved_by": by, "approved_at": at, "created_at": created, "comments": comments,
                    "retire_request": retire })
        })
        .collect();
    Ok(json!({ "slot": slot, "variants": variants }))
}

async fn propose(pool: &PgPool, site: &str, p: Proposal, call_row: i64, agent: &str) -> Result<Value, ApiError> {
    if !crate::site_file::is_slug(&p.key) {
        return Err(ApiError::BadRequest("key must be a lowercase slug (a-z, 0-9, - and _)".into()));
    }
    if p.html.trim().is_empty() || p.html.len() > admin::MAX_PAYLOAD {
        return Err(ApiError::BadRequest(format!("html must be 1 to {} bytes", admin::MAX_PAYLOAD)));
    }
    let slot_id = slot_id(pool, site, &p.slot).await?;
    let mut tx = pool.begin().await?;
    // The status is a literal: nothing an agent sends can make this row anything but a draft.
    let id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO variants (slot_id, key, payload, author, status, mcp_call_id) VALUES ($1, $2, $3, 'agent', 'draft', $4)
         ON CONFLICT (slot_id, key) DO NOTHING RETURNING id",
    )
    .bind(slot_id)
    .bind(&p.key)
    .bind(&p.html)
    .bind(call_row)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(id) = id else {
        return Err(ApiError::Conflict(format!("slot {:?} already has a variant called {:?}; pick a new key", p.slot, p.key)));
    };
    if let Some(note) = p.note.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        sqlx::query("INSERT INTO approvals (variant_id, action, actor, comment) VALUES ($1, 'comment', $2, $3)")
            .bind(id)
            .bind(agent.chars().take(128).collect::<String>())
            .bind(note)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    let (forbidden, facts): (Vec<String>, Option<String>) =
        sqlx::query_as("SELECT forbidden_claims, facts FROM sites WHERE id = $1").bind(site).fetch_one(pool).await?;
    let report = checks::run(&p.html, &checks::Context { forbidden_claims: &forbidden, author: "agent", facts: facts.as_deref() });
    Ok(json!({
        "id": id, "slot": p.slot, "key": p.key, "status": "draft", "checks": report,
        "next": if checks::blocks(&report) {
            "Draft saved, but a constraint check fails, so it cannot be approved as it is. Fix it and propose again under a new key."
        } else {
            "Draft saved. A person must approve it in the dashboard before any visitor sees it. Their comments appear on list_variants."
        },
    }))
}
