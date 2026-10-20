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
//!   (which activates it). `update_slot` edits a slot only while it is such a draft (its
//!   settings, and its kind options judged with its variants by the site file's rules); a live
//!   slot's settings are the owner's. Its one live-slot change is a PROPOSED `traffic_share`
//!   for a live variant, which a person confirms (`POST /api/share-requests/{id}/confirm`).
//! - **Retirement, proposed:** `retire_variant` records a proposal; the variant keeps serving
//!   until a person confirms it (`POST /api/retire-requests/{id}/confirm`). backend.md leaves the
//!   retire permission open, so this takes the drafts-only floor.
//! - **Plugins, requested:** `list_plugins` reads the compiled-in catalogue as this site has it;
//!   `install_plugin` records a request that a person confirms
//!   (`POST /api/plugin-requests/{id}/confirm`) before anything is installed.
//! - **Experiments, read-only:** `list_experiments` reads the site's experiments and their results
//!   per arm. Creating and stopping one is the owner's, in the dashboard API.
//! - **Assets:** `upload_asset` stores an image (base64 in the call; judged by its bytes,
//!   capped, SVG script refused) in the site's asset store and returns its id; `list_assets`
//!   reads the site's uploads. An upload changes nothing a visitor sees until a variant that
//!   names it is approved. The audit row keeps the upload's size and hash, not its bytes.
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
On a copy slot (kind copy) use propose_copy instead: plain-text fields within each field's max, no HTML. \
create_slot makes a DRAFT slot of kind swap, copy (with copy.fields) or sorted (with items, in the default order), live only once a person approves its baseline; update_slot edits only draft slots (and only proposes a live variant's traffic_share). \
retire_variant PROPOSES a retirement; the variant keeps serving until a person confirms. \
install_plugin REQUESTS a plugin; nothing is installed until a person confirms. \
simulate and fast_forward_visitor run only in the site's sandbox, never on real traffic. \
Every tool needs `agent` (your name, e.g. \"Claude Code\") and a one-line `justification`; both are \
recorded and shown to the reviewer. Reviewer comments come back on list_variants.";

// ---- the catalogue ----

const TOOLS: [&str; 21] = [
    "list_sites",
    "get_site",
    "list_slots",
    "list_variants",
    "get_results",
    "list_decisions",
    "explain_decision",
    "propose_variant",
    "propose_copy",
    "simulate",
    "fast_forward_visitor",
    "list_audiences",
    "create_audience",
    "create_slot",
    "update_slot",
    "retire_variant",
    "list_plugins",
    "install_plugin",
    "list_experiments",
    "upload_asset",
    "list_assets",
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
        tool("get_results", "Per-slot and per-variant results over closed attribution windows: conversion rate and revenue per visitor (each with a 95% always-valid interval), lift and revenue lift against the holdout, the slot's goal (conversion or revenue), the site's currency, and each slot's `lessons` (observations its model has absorbed) with `learned`: while `learned` is false nothing has been learned and the model still holds its prior. Only decisions whose window has closed are counted, so right after traffic starts every `n` can be 0: `pending` (per site, slot and variant; variant is treated only) gives the decisions, exposures and conversions still inside an open window, and `note` names each slot with any. Pending traffic is real but not yet evidence: do not read a rate from it. Revenue comes from purchase events with a value; each slot gets its attributor's share of every order in its window.",
             json!({ "slot": { "type": "string", "description": "Optional: one slot only." },
                     "days": { "type": "integer", "minimum": 1, "description": "Optional: only the last N days (default: all time)." } }), &[]),
        tool("list_decisions", "Recent decisions, newest first.",
             json!({ "slot": { "type": "string" }, "variant": { "type": "string" },
                     "outcome": { "type": "string", "enum": ["all", "converted", "missed", "open", "unexposed", "holdout"] },
                     "limit": { "type": "integer", "minimum": 1, "maximum": 500, "description": "Default 50." } }), &[]),
        tool("explain_decision", "Why one decision showed what it showed: features, scores, a replay check, the counterfactual and a plain sentence. `learned` false (`lessons` 0) means the model had learned nothing when it scored this: the scores are prior draws and every `inferred_rate` is the prior's 50%, not an estimate; the counterfactual's `learned`/`shown_learned` say the same per variant.",
             json!({ "id": { "type": "string", "description": "The decision id (a UUID)." } }), &["id"]),
        tool("propose_variant", "Submit a new variant for a slot. It lands as a DRAFT and is never served until a person approves it in the dashboard. Returns the constraint-check report a reviewer will see.",
             json!({ "slot": slot, "key": { "type": "string", "description": "A new lowercase slug for the variant (a-z, 0-9, - and _)." },
                     "html": { "type": "string", "description": "The HTML fragment that replaces the slot's content. No scripts or event handlers." },
                     "note": { "type": "string", "description": "Optional note to the reviewer; starts the comment thread." } }),
             &["slot", "key", "html"]),
        tool("propose_copy", "Submit new text and images for a copy slot (kind copy): one value per declared field. A text field takes plain text within its max length, no < or > and no newlines. An image field takes {asset (an id from upload_asset) or url, alt (required, plain text), width?, height?}: this is how to change a picture without writing HTML. Every required field filled. Lands as a DRAFT like propose_variant; list_slots shows the fields.",
             json!({ "slot": slot, "key": { "type": "string", "description": "A new lowercase slug for the variant (a-z, 0-9, - and _)." },
                     "copy": { "type": "object", "additionalProperties": { "oneOf": [ { "type": "string" },
                                   { "type": "object", "properties": { "asset": { "type": "string", "description": "An id upload_asset returned." }, "url": { "type": "string", "description": "https://... or a root-relative /path." },
                                     "alt": { "type": "string", "description": "Required: what the image shows, plain text." }, "width": { "type": "integer" }, "height": { "type": "integer" } }, "required": ["alt"] } ] },
                               "description": "Field name to value: text for a text field, e.g. {\"headline\": \"...\"}; for an image field {\"hero\": {\"asset\": \"<id from upload_asset>\", \"alt\": \"...\"}} (or \"url\" instead of \"asset\")." },
                     "note": { "type": "string", "description": "Optional note to the reviewer; starts the comment thread." } }),
             &["slot", "key", "copy"]),
        tool("create_slot", "Declare a new slot: swap (HTML variants, the default), copy (named plain-text fields with max lengths) or sorted (an order of items). It lands as a DRAFT: its baseline (the page's own content; a sorted slot's first item) is a draft, so the slot is in no manifest and decides nothing until a person approves that baseline in the dashboard, which activates it. The kind's options are checked as the site file checks them. Then propose variants (propose_variant, or propose_copy on a copy slot).",
             json!({ "key": { "type": "string", "description": "A new lowercase slug for the slot (a-z, 0-9, - and _), e.g. \"promo\"." },
                     "selector": { "type": "string", "description": "The CSS selector of the element the slot replaces, e.g. \"#promo\"." },
                     "kind": { "type": "string", "enum": ["swap", "copy", "sorted"], "description": "Optional (default swap)." },
                     "copy": { "type": "object", "description": "Required for kind copy, refused otherwise: {\"fields\": [{\"name\": \"headline\", \"max\": 60, \"required\": true}, {\"name\": \"hero\", \"type\": \"image\"}]}, 1-8 fields; a text field's max is 1-500; an image field's value is an uploaded asset or URL with alt text.",
                               "properties": { "fields": { "type": "array", "items": { "type": "object",
                                   "properties": { "name": { "type": "string" }, "max": { "type": "integer", "minimum": 1, "maximum": 500, "description": "Text: required, the value's max length. Image: optional, the alt text's max (default 250)." }, "required": { "type": "boolean" },
                                                  "type": { "type": "string", "enum": ["text", "image"], "description": "Optional (default text). An image field's value is {asset or url, alt, width?, height?}." } },
                                   "required": ["name"] } } }, "required": ["fields"] },
                     "items": { "type": "array", "items": { "type": "string" }, "description": "Required for kind sorted, refused otherwise: 2-50 unique item keys (slugs) in the default order; the first is the baseline." },
                     "sorted": { "type": "object", "properties": { "visible": { "type": "integer", "minimum": 1 } }, "description": "Optional for kind sorted: how many positions a visitor reads (default 4)." },
                     "metric": { "type": "string", "description": "Optional: the event that counts as success (default \"click\")." },
                     "attribution_window_secs": { "type": "integer", "minimum": 1, "description": "Optional: how long after a decision a success still counts (default 1800)." },
                     "decision_ttl_secs": { "type": "integer", "minimum": 0, "description": "Optional: how long a visitor keeps the same decision (default 60)." },
                     "note": { "type": "string", "description": "Optional note to the reviewer; starts the comment thread." } }),
             &["key", "selector"]),
        tool("update_slot", "Change a DRAFT slot's selector, metric, attribution window, decision TTL or kind options (only the fields you send): a copy slot's copy.fields, a sorted slot's sorted.visible or items. Kind options are checked as the site file checks them, together with the slot's existing variants: fields a variant would no longer fit (a newly required field it lacks, a max its text exceeds, a field it fills that you drop) are refused, and nothing changes. A live slot's settings belong to the site owner and are refused. On a live slot you may only PROPOSE a live variant's traffic_share (your justification is the reason the reviewer reads); it keeps its share until a person confirms, and the answer appears on list_variants.",
             json!({ "slot": slot,
                     "selector": { "type": "string" }, "metric": { "type": "string" },
                     "attribution_window_secs": { "type": "integer", "minimum": 1 }, "decision_ttl_secs": { "type": "integer", "minimum": 0 },
                     "copy": { "type": "object", "description": "Draft copy slot only: the new {\"fields\": [{\"name\", \"max\", \"required\"}]} (replaces the old list).",
                               "properties": { "fields": { "type": "array", "items": { "type": "object",
                                   "properties": { "name": { "type": "string" }, "max": { "type": "integer", "minimum": 1, "maximum": 500, "description": "Text: required, the value's max length. Image: optional, the alt text's max (default 250)." }, "required": { "type": "boolean" },
                                                  "type": { "type": "string", "enum": ["text", "image"], "description": "Optional (default text). An image field's value is {asset or url, alt, width?, height?}." } },
                                   "required": ["name"] } } }, "required": ["fields"] },
                     "sorted": { "type": "object", "properties": { "visible": { "type": "integer", "minimum": 1 } }, "description": "Draft sorted slot only: how many positions a visitor reads." },
                     "items": { "type": "array", "items": { "type": "string" }, "description": "Draft sorted slot only: the new item keys in the default order (2-50); the first becomes the head. Dropped items are removed, new ones arrive as drafts." },
                     "traffic_share": { "type": "object", "description": "PROPOSE a new share for one live (approved) variant: {\"variant\": \"gift\", \"share\": 0.3}, above 0 and at most 0.5.",
                                        "properties": { "variant": { "type": "string" }, "share": { "type": "number", "exclusiveMinimum": 0, "maximum": 0.5 } },
                                        "required": ["variant", "share"] } }),
             &["slot"]),
        tool("retire_variant", "PROPOSE retiring a live variant (your justification is the reason the reviewer reads). It keeps serving until a person confirms in the dashboard; you cannot retire it yourself. The baseline cannot be retired. The answer appears on list_variants.",
             json!({ "slot": slot, "variant": { "type": "string", "description": "The variant key." } }),
             &["slot", "variant"]),
        tool("list_plugins", "The compiled-in plugin catalogue as this site has it: each plugin's kind, whether it is a de-anonymiser, its install (or null) and your latest request with its answer; plus the site's warnings.", json!({}), &[]),
        tool("list_experiments", "The site's A/B experiments (fixed-allocation overlays on one slot): arms and weights, start and stop, status, and each arm's conversion rate and lift over the control with confidence sequences. Read-only: a person creates and stops experiments in the dashboard API.", json!({}), &[]),
        tool("install_plugin", "REQUEST installing a plugin on this site (your justification is the reason the reviewer reads). Nothing is installed until a person confirms in the dashboard API; you cannot install it yourself. The answer appears on list_plugins.",
             json!({ "plugin": { "type": "string", "description": "The plugin name from list_plugins, e.g. \"attribution.linear\"." },
                     "config": { "type": "object", "description": "Optional: the plugin's config, e.g. {\"url\": \"https://...\"} for an event sink." } }),
             &["plugin"]),
        tool("upload_asset", "Upload an image to this site's asset store (png, jpeg, webp, avif, gif or svg; judged by its bytes, not its name; size-capped; an SVG with script, event handlers or outside links is refused). Returns its id (the SHA-256 of the bytes). Use it in an image field of a copy slot as {\"asset\": id, \"alt\": \"...\"} (propose_copy), or in swap HTML as <img src=\"/_prb/assets/<site>/<id>\" alt=\"...\"> (propose_variant). Uploading changes nothing a visitor sees; an asset no draft or live variant names is deleted after a grace period.",
             json!({ "data": { "type": "string", "description": "The file's bytes, base64-encoded." },
                     "name": { "type": "string", "description": "Optional: the file name, for people reading the asset list." } }),
             &["data"]),
        tool("list_assets", "This site's uploaded images: id, type, size, pixel size where known, name, and whether a variant uses it.", json!({}), &[]),
        tool("list_audiences", "The site's segments (draft, active, audience) with their predicates, and the variants each promoted audience restricts.", json!({}), &[]),
        tool("create_audience", "Draft a segment from a predicate over behaviour and situation features (returning, mobile, from_ad, is_night_local, is_weekend_local, is_business_hours_local, hour_local: [from, to]) and the visitor's own history (days_since_last_purchase: {gte, lte}, purchase_count: {gte, lte}, lifetime_value: none|low|mid|high, cart_abandoned: bool, viewed: {item or category, days 1-90}; these match identified visitors only and nobody in cookieless mode). It lands as a DRAFT: a person promotes it to an audience in the dashboard before it can restrict any variant. Demographic attributes are refused.",
             json!({ "predicate": { "type": "object", "description": "e.g. {\"returning\": true, \"is_night_local\": true} or {\"days_since_last_purchase\": {\"gte\": 30}}" },
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

/// The arguments as the audit log keeps them: an upload's bytes become their length and hash,
/// so the log holds what was sent without holding the file.
fn redacted(args: &Value) -> Value {
    let mut a = args.clone();
    if let Some(d) = a.get("data").and_then(Value::as_str).map(String::from) {
        a["data"] = json!(format!("<{} base64 characters, sha256 of the text {}>", d.len(), crate::assets::sha256_hex(d.as_bytes())));
    }
    a
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
        .bind(redacted(self.arguments))
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
                ApiError::NotFound(m) | ApiError::BadRequest(m) | ApiError::Conflict(m) | ApiError::Gone(m) | ApiError::TooLarge(m) => m,
                ApiError::Unauthorized => "unauthorized".into(),
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
struct CopyProposal {
    slot: String,
    key: String,
    copy: crate::copy::Text,
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadArgs {
    data: String,
    name: Option<String>,
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

/// Exactly what an agent may set on a new slot: a swap, copy or sorted slot, never a picker or
/// an on-device one. The kind's own options are checked by the site file's own rules.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewSlot {
    key: String,
    selector: String,
    kind: Option<String>,
    /// A copy slot's fields, as the site file declares them.
    copy: Option<crate::copy::CopySpec>,
    /// A sorted slot's options (`visible`).
    sorted: Option<crate::sorted::SortedSpec>,
    /// A sorted slot's item keys, in the default order; the first is the head (its baseline).
    items: Option<Vec<String>>,
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
    /// A draft copy slot's new fields, judged with its variants by the site file's rules.
    copy: Option<crate::copy::CopySpec>,
    /// A draft sorted slot's options (`visible`).
    sorted: Option<crate::sorted::SortedSpec>,
    /// A draft sorted slot's new items, in the default order; the first becomes the head.
    items: Option<Vec<String>>,
    /// A share for one live variant: only proposed, a person confirms it.
    traffic_share: Option<ShareProposal>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShareProposal {
    variant: String,
    share: f32,
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
    let kind = a.kind.unwrap_or_else(|| "swap".into());
    if !["swap", crate::copy::KIND, crate::sorted::KIND].contains(&kind.as_str()) {
        return Err(ApiError::BadRequest(format!("kind must be swap, copy or sorted (got {kind:?})")));
    }
    let sorted = kind == crate::sorted::KIND;
    if a.items.is_some() && !sorted {
        return Err(ApiError::BadRequest("only a kind \"sorted\" slot takes items".into()));
    }
    // The slot as the site file would declare it, so the site file's rules judge it: a sorted
    // slot's items are its variants; any other kind has the one baseline.
    let variants = match a.items {
        Some(items) => items.into_iter().map(|key| crate::site_file::VariantSpec { key, baseline: false, html: None, copy: None, traffic_share: None }).collect(),
        None if sorted => vec![],
        None => vec![crate::site_file::VariantSpec { key: "baseline".into(), baseline: true, html: None, copy: None, traffic_share: None }],
    };
    let spec = crate::site_file::SlotSpec {
        key: a.key.clone(),
        selector: selector.clone(),
        metric: metric.clone(),
        attribution_window_secs: window,
        decision_ttl_secs: ttl,
        cold_start_exposures: crate::site_file::default_cold_start(),
        kind: Some(kind.clone()),
        in_session: false,
        picker: None,
        attribution: None,
        goal: None,
        revenue_cap: None,
        sorted: a.sorted,
        copy: a.copy,
        variants,
    };
    crate::site_file::validate_slot(&spec).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let mut tx = pool.begin().await?;
    let slot_id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO slots (site_id, key, selector, metric, attribution_window_secs, decision_ttl_secs, kind, sorted, copy, created_by, mcp_call_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) ON CONFLICT (site_id, key) DO NOTHING RETURNING id",
    )
    .bind(site)
    .bind(&a.key)
    .bind(&selector)
    .bind(&metric)
    .bind(window)
    .bind(ttl)
    .bind(&kind)
    .bind(crate::sorted::stored(&spec))
    .bind(crate::copy::stored(&spec))
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
    // The baseline is the page's own content (a sorted slot's: the head of its default order).
    // As a draft, it keeps the whole slot dark; a sorted slot's other items are drafts too.
    let mut baseline = 0;
    for (i, v) in spec.variants.iter().enumerate() {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO variants (slot_id, key, payload, is_baseline, author, status, mcp_call_id)
             VALUES ($1, $2, NULL, $3, 'agent', 'draft', $4) RETURNING id",
        )
        .bind(slot_id)
        .bind(&v.key)
        .bind(i == 0)
        .bind(call_row)
        .fetch_one(&mut *tx)
        .await?;
        if i == 0 {
            baseline = id;
        }
    }
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
        "slot": a.key, "kind": kind, "selector": selector, "metric": metric, "attribution_window_secs": window, "decision_ttl_secs": ttl,
        "status": "draft", "baseline_id": baseline,
        "next": match kind.as_str() {
            "copy" => "Draft copy slot saved. It reaches no visitor until a person approves its baseline in the dashboard. Propose text for it with propose_copy.",
            "sorted" => "Draft sorted slot saved, every item a draft. It reaches no visitor until a person approves its head item (and each other item) in the dashboard.",
            _ => "Draft slot saved. It reaches no visitor until a person approves its baseline in the dashboard. Propose variants for it with propose_variant.",
        },
    }))
}

/// `update_slot`: only a draft slot's settings and kind options are the agent's to change. A
/// live variant's traffic share is the one thing it may touch on a live slot, and only as a
/// proposal a person confirms (`share_requests`).
async fn update_slot(pool: &PgPool, site: &str, u: SlotUpdate, call_row: i64, who: &Who<'_>) -> Result<Value, ApiError> {
    let slot_id = slot_id(pool, site, &u.slot).await?;
    let settings = u.selector.is_some() || u.metric.is_some() || u.attribution_window_secs.is_some() || u.decision_ttl_secs.is_some();
    let options = u.copy.is_some() || u.sorted.is_some() || u.items.is_some();
    if !settings && !options && u.traffic_share.is_none() {
        return Err(ApiError::BadRequest(
            "send at least one of selector, metric, attribution_window_secs, decision_ttl_secs, copy, sorted, items, traffic_share".into(),
        ));
    }
    let draft = is_draft_slot(pool, slot_id).await?;
    if (settings || options) && !draft {
        return Err(ApiError::Conflict(format!(
            "slot {:?} is live: its settings belong to the site owner (site file or dashboard). Tell the person what to change instead; a live variant's traffic_share you may propose on its own.",
            u.slot
        )));
    }
    // The share proposal is checked before anything is written, so a refusal changes nothing.
    let share = match &u.traffic_share {
        Some(p) => Some(check_share_proposal(pool, slot_id, &u.slot, p).await?),
        None => None,
    };
    let mut tx = pool.begin().await?;
    let mut out = json!({ "slot": u.slot, "status": if draft { "draft" } else { "live" } });
    if settings || options {
        let (selector, metric, window, ttl, kind, copy, sorted): (String, String, i32, i32, String, Option<Value>, Option<Value>) = sqlx::query_as(
            "SELECT selector, metric, attribution_window_secs, decision_ttl_secs, kind, copy, sorted FROM slots WHERE id = $1 FOR UPDATE",
        )
        .bind(slot_id)
        .fetch_one(&mut *tx)
        .await?;
        let selector = u.selector.map(|s| s.trim().to_string()).unwrap_or(selector);
        let metric = u.metric.unwrap_or(metric);
        let window = u.attribution_window_secs.unwrap_or(window);
        let ttl = u.decision_ttl_secs.unwrap_or(ttl);
        check_slot_settings(&selector, &metric, window, ttl)?;
        let spec = kind_spec(&mut tx, slot_id, &u.slot, &kind, copy, sorted, u.copy, u.sorted, u.items.clone(), (&selector, &metric, window, ttl)).await?;
        // The site file's own per-slot rules judge the slot with the variants it already has, so
        // fields that a variant would no longer fit are refused here, not discovered at approval.
        crate::site_file::validate_slot(&spec).map_err(|e| ApiError::BadRequest(format!("{e} (the slot's existing variants must still fit; nothing was changed)")))?;
        if let Some(items) = &u.items {
            set_items(&mut tx, slot_id, items, call_row, who.agent).await?;
        }
        sqlx::query("UPDATE slots SET selector = $2, metric = $3, attribution_window_secs = $4, decision_ttl_secs = $5, copy = $6, sorted = $7 WHERE id = $1")
            .bind(slot_id)
            .bind(&selector)
            .bind(&metric)
            .bind(window)
            .bind(ttl)
            .bind(crate::copy::stored(&spec))
            .bind(crate::sorted::stored(&spec))
            .execute(&mut *tx)
            .await?;
        let o = out.as_object_mut().expect("object");
        o.insert("selector".into(), json!(selector));
        o.insert("metric".into(), json!(metric));
        o.insert("attribution_window_secs".into(), json!(window));
        o.insert("decision_ttl_secs".into(), json!(ttl));
        o.insert("copy".into(), json!(crate::copy::stored(&spec)));
        o.insert("sorted".into(), json!(crate::sorted::stored(&spec)));
    }
    if let (Some(p), Some(variant_id)) = (&u.traffic_share, share) {
        let id: Option<i64> = sqlx::query_scalar(
            "INSERT INTO share_requests (variant_id, mcp_call_id, agent, reason, traffic_share) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (variant_id) WHERE status = 'pending' DO NOTHING RETURNING id",
        )
        .bind(variant_id)
        .bind(call_row)
        .bind(who.agent.chars().take(128).collect::<String>())
        .bind(who.justification)
        .bind(p.share)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(id) = id else {
            return Err(ApiError::Conflict(format!("variant {:?} already has an open share proposal; see list_variants", p.variant)));
        };
        out["traffic_share"] = json!({
            "id": id, "variant": p.variant, "share": p.share, "status": "proposed",
            "next": "Share proposal recorded. The variant keeps its current share until a person confirms in the dashboard; the answer appears on list_variants.",
        });
    }
    tx.commit().await?;
    Ok(out)
}

/// A proposed share must be in range and for a live, non-baseline variant: its id.
async fn check_share_proposal(pool: &PgPool, slot_id: i64, slot: &str, p: &ShareProposal) -> Result<i64, ApiError> {
    admin::check_share(p.share)?;
    let row: Option<(i64, String, bool)> = sqlx::query_as("SELECT id, status, is_baseline FROM variants WHERE slot_id = $1 AND key = $2")
        .bind(slot_id)
        .bind(&p.variant)
        .fetch_optional(pool)
        .await?;
    let Some((id, status, is_baseline)) = row else {
        return Err(ApiError::NotFound(format!("slot {slot:?} has no variant {:?}", p.variant)));
    };
    if is_baseline {
        return Err(ApiError::BadRequest("the baseline is the page's own content and has no traffic share".into()));
    }
    if status != "approved" && status != "paused" {
        return Err(ApiError::Conflict(format!(
            "variant {:?} is {status}; only a live variant's share can change (a draft's share is set by the person who approves it: say what you suggest in its note)",
            p.variant
        )));
    }
    Ok(id)
}

/// The slot as the site file would declare it after the update, with the variants it has now
/// (a sorted slot's: the new items), so [`crate::site_file::validate_slot`] can judge both.
#[allow(clippy::too_many_arguments)]
async fn kind_spec(
    tx: &mut sqlx::PgConnection,
    slot_id: i64,
    key: &str,
    kind: &str,
    copy: Option<Value>,
    sorted: Option<Value>,
    new_copy: Option<crate::copy::CopySpec>,
    new_sorted: Option<crate::sorted::SortedSpec>,
    items: Option<Vec<String>>,
    (selector, metric, window, ttl): (&str, &str, i32, i32),
) -> Result<crate::site_file::SlotSpec, ApiError> {
    use crate::site_file::VariantSpec;
    let is_sorted = kind == crate::sorted::KIND;
    if items.is_some() && !is_sorted {
        return Err(ApiError::BadRequest("only a kind \"sorted\" slot takes items".into()));
    }
    // Rejected and retired variants serve nobody and never will, so they do not hold the fields.
    let rows: Vec<(String, bool, Option<String>, String)> =
        sqlx::query_as("SELECT key, is_baseline, payload, status FROM variants WHERE slot_id = $1 ORDER BY id").bind(slot_id).fetch_all(&mut *tx).await?;
    let copy = match new_copy {
        Some(c) => Some(c),
        None => copy.map(|c| serde_json::from_value(c).map_err(|e| ApiError::BadRequest(format!("bad stored copy spec: {e}")))).transpose()?,
    };
    let (variants, sorted) = if is_sorted {
        let stored: Option<(u32, Vec<String>)> = sorted.as_ref().map(|s| {
            (s["visible"].as_u64().unwrap_or(crate::sorted::DEFAULT_VISIBLE as u64) as u32, serde_json::from_value(s["order"].clone()).unwrap_or_default())
        });
        let (visible, order) = stored.unwrap_or((crate::sorted::DEFAULT_VISIBLE, vec![]));
        let items = items.unwrap_or(order);
        // A retired item on a draft slot was dropped from its items earlier; it may come back.
        if let Some((k, status)) = rows.iter().find(|r| items.contains(&r.0) && r.3 != "draft" && r.3 != "retired").map(|r| (&r.0, &r.3)) {
            return Err(ApiError::Conflict(format!("item {k:?} is {status}; only a draft slot's draft items can be reordered or replaced")));
        }
        // Visible follows the items down when there are fewer of them, as create_slot clamps it.
        let visible = new_sorted.and_then(|s| s.visible).unwrap_or(visible.min(items.len() as u32).max(1));
        let variants = items.into_iter().map(|key| VariantSpec { key, baseline: false, html: None, copy: None, traffic_share: None }).collect();
        (variants, Some(crate::sorted::SortedSpec { visible: Some(visible) }))
    } else {
        let is_copy = kind == crate::copy::KIND;
        let variants = rows
            .into_iter()
            .filter(|r| r.3 != "rejected" && r.3 != "retired")
            .map(|(key, baseline, payload, _)| {
                let text = (is_copy && !baseline).then(|| payload.as_deref().and_then(|p| serde_json::from_str(p).ok())).flatten();
                let html = (!is_copy && !baseline).then_some(payload).flatten();
                VariantSpec { key, baseline, html, copy: text, traffic_share: None }
            })
            .collect();
        (variants, new_sorted)
    };
    Ok(crate::site_file::SlotSpec {
        key: key.to_string(),
        selector: selector.to_string(),
        metric: metric.to_string(),
        attribution_window_secs: window,
        decision_ttl_secs: ttl,
        cold_start_exposures: crate::site_file::default_cold_start(),
        kind: Some(kind.to_string()),
        in_session: false,
        picker: None,
        attribution: None,
        goal: None,
        revenue_cap: None,
        sorted,
        copy,
        variants,
    })
}

/// A draft sorted slot's new items: dropped items are retired (kept with their comment thread,
/// and the retirement is in the trail under the agent's name), new ones arrive as drafts, a
/// retired item listed again is a draft again, and the head of the new order is the baseline.
async fn set_items(tx: &mut sqlx::PgConnection, slot_id: i64, items: &[String], call_row: i64, agent: &str) -> Result<(), ApiError> {
    let dropped: Vec<i64> = sqlx::query_scalar("UPDATE variants SET status = 'retired' WHERE slot_id = $1 AND status = 'draft' AND NOT (key = ANY($2)) RETURNING id")
        .bind(slot_id)
        .bind(items)
        .fetch_all(&mut *tx)
        .await?;
    for id in dropped {
        sqlx::query("INSERT INTO approvals (variant_id, action, actor, comment) VALUES ($1, 'retire', $2, 'dropped from the sorted items by update_slot')")
            .bind(id)
            .bind(agent)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE variants SET is_baseline = false WHERE slot_id = $1").bind(slot_id).execute(&mut *tx).await?;
    for (i, key) in items.iter().enumerate() {
        sqlx::query(
            "INSERT INTO variants (slot_id, key, payload, is_baseline, author, status, mcp_call_id) VALUES ($1, $2, NULL, $3, 'agent', 'draft', $4)
             ON CONFLICT (slot_id, key) DO UPDATE SET is_baseline = EXCLUDED.is_baseline,
                 status = CASE WHEN variants.status = 'retired' THEN 'draft' ELSE variants.status END",
        )
        .bind(slot_id)
        .bind(key)
        .bind(i == 0)
        .bind(call_row)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
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
            let pending: Vec<String> = r
                .slots
                .iter()
                .filter(|s| s.pending.decisions > 0)
                .map(|s| {
                    let p = s.pending;
                    format!("{}: {} decisions still in their attribution window ({} shown, {} converted so far), not counted yet", s.key, p.decisions, p.exposures, p.conversions)
                })
                .collect();
            let mut v = json!(r);
            if !pending.is_empty() {
                v["note"] = json!(pending.join("; "));
            }
            Ok(v)
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
            propose(pool, site, p, call_row, agent, false).await
        }
        "propose_copy" => {
            let p: CopyProposal = parse(args)?;
            let html = crate::copy::payload(&p.copy);
            propose(pool, site, Proposal { slot: p.slot, key: p.key, html, note: p.note }, call_row, agent, true).await
        }
        "simulate" => simulate(pool, site, parse(args)?).await,
        "list_audiences" => {
            parse::<NoArgs>(args)?;
            list_audiences(pool, site).await
        }
        "create_audience" => create_audience(pool, site, parse(args)?, agent).await,
        "fast_forward_visitor" => fast_forward(pool, site, parse(args)?).await,
        "create_slot" => create_slot(pool, site, parse(args)?, call_row, agent).await,
        "update_slot" => update_slot(pool, site, parse(args)?, call_row, who).await,
        "retire_variant" => retire_variant(pool, site, parse(args)?, call_row, who).await,
        "list_plugins" => {
            parse::<NoArgs>(args)?;
            crate::plugins::listing(pool, site).await
        }
        "install_plugin" => {
            let a: PluginArgs = parse(args)?;
            crate::plugins::request(pool, site, &a.plugin, a.config.as_ref(), call_row, who.agent, who.justification).await
        }
        "list_experiments" => {
            parse::<NoArgs>(args)?;
            crate::experiments::listing(pool, site).await
        }
        "upload_asset" => {
            let a: UploadArgs = parse(args)?;
            use base64::Engine;
            let clean: String = a.data.chars().filter(|c| !c.is_whitespace()).collect();
            let clean = clean.split_once("base64,").map(|(_, d)| d.to_string()).unwrap_or(clean);
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(clean.as_bytes())
                .map_err(|_| ApiError::BadRequest("data must be the file's bytes, base64-encoded".into()))?;
            crate::assets::upload(pool, &state.assets, site, &bytes, a.name.as_deref(), agent, Some(call_row)).await
        }
        "list_assets" => {
            parse::<NoArgs>(args)?;
            crate::assets::list(pool, site).await
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
    let slots: Vec<(String, String, String, i32, String)> =
        sqlx::query_as("SELECT key, selector, metric, attribution_window_secs, goal FROM slots WHERE site_id = $1 ORDER BY key").bind(site).fetch_all(pool).await?;
    Ok(json!({
        "id": site, "origin": origin, "holdout_fraction": holdout, "forbidden_claims": forbidden, "facts": facts,
        "slots": slots.into_iter().map(|(key, selector, metric, w, goal)| json!({ "key": key, "selector": selector, "metric": metric, "attribution_window_secs": w, "goal": goal })).collect::<Vec<_>>(),
    }))
}

async fn list_slots(pool: &PgPool, site: &str) -> Result<Value, ApiError> {
    let r = results::site(pool, site, results::ALL_TIME, Utc::now()).await?.ok_or_else(|| ApiError::NotFound(format!("unknown site {site:?}")))?;
    let rows: Vec<(i64, String, String, Option<Value>)> =
        sqlx::query_as("SELECT id, selector, kind, copy FROM slots WHERE site_id = $1").bind(site).fetch_all(pool).await?;
    let slots: Vec<Value> = r
        .slots
        .iter()
        .map(|s| {
            let by = |st: &str| s.variants.iter().filter(|v| v.status == st).count();
            let row = rows.iter().find(|r| r.0 == s.id);
            json!({
                "key": s.key, "metric": s.metric,
                "selector": row.map(|r| r.1.clone()),
                "kind": row.map(|r| r.2.clone()),
                // A copy slot's fields (F4): propose_copy text must fit these.
                "copy_fields": row.and_then(|r| r.3.as_ref()).map(|c| c["fields"].clone()),
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
    #[allow(clippy::type_complexity)]
    let shares: Vec<(i64, i64, String, f32, String, Option<String>, chrono::DateTime<Utc>)> = sqlx::query_as(
        "SELECT DISTINCT ON (r.variant_id) r.variant_id, r.id, r.status, r.traffic_share, r.reason, r.decided_by, r.created_at
           FROM share_requests r JOIN variants v ON v.id = r.variant_id
          WHERE v.slot_id = $1 ORDER BY r.variant_id, r.created_at DESC, r.id DESC",
    )
    .bind(slot_id)
    .fetch_all(pool)
    .await?;
    let variants: Vec<Value> = rows
        .into_iter()
        .map(|(id, key, status, author, base, html, share, by, at, created)| {
            let share_request = shares.iter().find(|r| r.0 == id).map(|(_, rid, st, proposed, reason, by, at)| {
                json!({ "id": rid, "status": st, "traffic_share": proposed, "reason": reason, "decided_by": by, "at": at })
            });
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
                    "retire_request": retire, "share_request": share_request })
        })
        .collect();
    Ok(json!({ "slot": slot, "variants": variants }))
}

async fn propose(pool: &PgPool, site: &str, p: Proposal, call_row: i64, agent: &str, as_copy: bool) -> Result<Value, ApiError> {
    if !crate::site_file::is_slug(&p.key) {
        return Err(ApiError::BadRequest("key must be a lowercase slug (a-z, 0-9, - and _)".into()));
    }
    if p.html.trim().is_empty() || p.html.len() > admin::MAX_PAYLOAD {
        return Err(ApiError::BadRequest(format!("html must be 1 to {} bytes", admin::MAX_PAYLOAD)));
    }
    let (slot_id, spec): (i64, Option<Value>) = sqlx::query_as("SELECT id, copy FROM slots WHERE site_id = $1 AND key = $2")
        .bind(site)
        .bind(&p.slot)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("unknown slot {:?} on site {site:?}", p.slot)))?;
    // A copy slot takes only propose_copy's fields, checked against the slot's own; html never.
    let is_copy = spec.is_some();
    match (&spec, as_copy) {
        (Some(_), false) => return Err(ApiError::BadRequest(format!("slot {:?} is a copy slot: use propose_copy with its fields, not html", p.slot))),
        (None, true) => return Err(ApiError::BadRequest(format!("slot {:?} is not a copy slot; use propose_variant", p.slot))),
        (Some(spec), true) => {
            crate::copy::check_stored(spec, Some(&p.html)).map_err(ApiError::BadRequest)?;
            let text = crate::copy::read_text(Some(&p.html)).map_err(ApiError::BadRequest)?;
            crate::assets::check_refs(pool, site, &crate::copy::asset_refs(&text)).await?;
        }
        (None, false) => {}
    }
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
    let text = if is_copy { crate::copy::plain(Some(&p.html)) } else { p.html.clone() };
    let report = checks::run(&text, &checks::Context { forbidden_claims: &forbidden, author: "agent", facts: facts.as_deref() });
    Ok(json!({
        "id": id, "slot": p.slot, "key": p.key, "status": "draft", "checks": report,
        "next": if checks::blocks(&report) {
            "Draft saved, but a constraint check fails, so it cannot be approved as it is. Fix it and propose again under a new key."
        } else {
            "Draft saved. A person must approve it in the dashboard before any visitor sees it. Their comments appear on list_variants."
        },
    }))
}
