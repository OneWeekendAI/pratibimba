//! The Preview room's backend: what a visitor COULD see, with no side effects.
//!
//! - `POST /api/sites/{site}/preview-token` (admin) mints a short-lived token, signed with a key
//!   derived from `PRB_ADMIN_TOKEN`, and the site page URL to open with it (`?prb_preview=`).
//!   The token names the site, its expiry, and the dashboard origin that asked for it (the
//!   request's `Origin`), so the SDK talks to that parent window only.
//! - `POST /preview` (public, token-authed: the SDK calls it through the site's `/_prb` proxy)
//!   returns, per slot, EVERY version this visitor is eligible for after the audience gates, and
//!   why each one is there. Approved versions first (those made for a matched audience, then the
//!   ones open to everyone, then the page's own), then drafts.
//! - `POST /api/variants/{id}/retire` (admin): the room's Reject on a live variant. A draft is
//!   rejected through the approvals route instead.
//!
//! `/preview` reads and never writes: no decision, no exposure, no webhook row, no model update,
//! no sticky decision, and it draws nothing from the decision seed stream, so the production
//! decide sequence is the same with or without previews in between. Traffic share, holdout,
//! experiments and cold start do not apply: the room shows every version, not one draw.

use super::decide::{ArmRow, Ctx, GateRow, close_by_requires, gate_by_audience};
use super::{ApiError, AppState};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use std::sync::Arc;

/// How long a preview token lives. The room mints a fresh one before this runs out.
pub const TOKEN_TTL_SECS: i64 = 30 * 60;
/// The query parameter the SDK reads the token from.
pub const PARAM: &str = "prb_preview";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claims {
    pub site: String,
    /// Unix seconds.
    pub exp: i64,
    /// The dashboard origin that minted it; the SDK posts to and accepts from this window only.
    pub parent: Option<String>,
}

fn mac(admin_token: &str) -> Hmac<Sha256> {
    // A key of its own: the admin token itself is never what signs a URL that leaves the dashboard.
    Hmac::<Sha256>::new_from_slice(format!("prb-preview\0{admin_token}").as_bytes()).expect("HMAC takes any key length")
}

pub fn mint(admin_token: &str, claims: &Claims) -> String {
    let body = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).expect("claims serialise"));
    let mut m = mac(admin_token);
    m.update(body.as_bytes());
    format!("{body}.{}", URL_SAFE_NO_PAD.encode(m.finalize().into_bytes()))
}

/// The claims of a token this server signed and that has not expired, else `None`.
pub fn verify(admin_token: &str, token: &str, now: i64) -> Option<Claims> {
    let (body, sig) = token.split_once('.')?;
    let sig = URL_SAFE_NO_PAD.decode(sig).ok()?;
    let mut m = mac(admin_token);
    m.update(body.as_bytes());
    m.verify_slice(&sig).ok()?;
    let claims: Claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(body).ok()?).ok()?;
    (claims.exp > now).then_some(claims)
}

pub fn public_routes() -> Router<Arc<AppState>> {
    Router::new().route("/preview", post(preview))
}

pub fn admin_routes() -> Router<Arc<AppState>> {
    Router::new().route("/sites/{site}/preview-token", post(issue)).route("/variants/{id}/retire", post(retire))
}

async fn issue(State(state): State<Arc<AppState>>, Path(site): Path<String>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let admin = state.admin_token.as_deref().ok_or_else(|| ApiError::Conflict("the dashboard API is off".into()))?;
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT origin, preview_url FROM sites WHERE id = $1").bind(&site).fetch_optional(&state.pool).await?;
    let (origin, preview_url) = row.ok_or_else(|| ApiError::NotFound(format!("unknown site {site:?}")))?;
    let parent = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()).filter(|o| *o != "null").map(String::from);
    let exp = chrono::Utc::now().timestamp() + TOKEN_TTL_SECS;
    let token = mint(admin, &Claims { site: site.clone(), exp, parent });
    let page = preview_url.unwrap_or_else(|| format!("{}/", origin.trim_end_matches('/')));
    let mut url = reqwest::Url::parse(&page).map_err(|_| ApiError::Conflict(format!("site {site:?} has no usable page URL ({page:?})")))?;
    url.query_pairs_mut().append_pair(PARAM, &token);
    Ok(Json(json!({ "token": token, "expires_in": TOKEN_TTL_SECS, "page": page, "url": url.to_string() })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewRequest {
    pub token: String,
    #[serde(default)]
    pub ctx: Ctx,
    /// P1: a synthetic visitor from [`crate::personas`] whose history fills templates and
    /// decides `requires`. None: as before, no history (templates show their fallbacks) and
    /// nothing is closed by `requires`.
    #[serde(default)]
    pub persona: Option<String>,
}

fn refused(msg: &str) -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({ "error": msg }))).into_response()
}

async fn preview(State(state): State<Arc<AppState>>, Json(req): Json<PreviewRequest>) -> Response {
    let Some(admin) = state.admin_token.as_deref() else {
        return refused("preview is off: the dashboard API is off");
    };
    let Some(claims) = verify(admin, &req.token, chrono::Utc::now().timestamp()) else {
        return refused("preview token is not valid or has expired");
    };
    let persona = match req.persona.as_deref().map(|k| crate::personas::find(k).ok_or(k)) {
        None => None,
        Some(Ok(p)) => Some(p),
        Some(Err(k)) => {
            let known: Vec<&str> = crate::personas::all().iter().map(|p| p.key).collect();
            return ApiError::BadRequest(format!("unknown persona {k:?}; one of {}", known.join(", "))).into_response();
        }
    };
    match versions(&state.pool, &claims.site, &req.ctx, persona.as_ref()).await {
        Ok(slots) => Json(json!({ "site": claims.site, "parent": claims.parent, "slots": slots })).into_response(),
        Err(e) => e.into_response(),
    }
}

/// Every slot of `site` with the versions `ctx` (and `persona`'s history) is eligible for, in the
/// room's order. Reads only. With a persona, a version whose `requires` the persona lacks stays
/// listed, marked `closed`, with no copy and the version the visitor falls to in its `why`.
pub async fn versions(pool: &sqlx::PgPool, site: &str, ctx: &Ctx, persona: Option<&crate::personas::Persona>) -> Result<Vec<Value>, ApiError> {
    let facts = persona.and_then(|p| p.facts.as_ref());
    if ctx.tz_offset.is_some_and(|m| !crate::insights::localtime::valid_offset(m)) {
        return Err(ApiError::BadRequest("ctx.tz_offset is minutes east of UTC, -720 to 840".into()));
    }
    let site_row: Option<(chrono::DateTime<chrono::Utc>, i16, i16)> =
        sqlx::query_as("SELECT COALESCE(sim_now, now()), night_start_hour, night_end_hour FROM sites WHERE id = $1").bind(site).fetch_optional(pool).await?;
    let (at, ns, ne) = site_row.ok_or_else(|| ApiError::NotFound(format!("unknown site {site:?}")))?;
    let night = crate::insights::localtime::NightWindow { start: ns as u8, end: ne as u8 };
    // A named zone wins over an offset, as in decide.
    let ctx = match ctx.tz.as_deref() {
        None => ctx.clone(),
        Some(name) => {
            let z = crate::insights::localtime::parse_zone(name).ok_or_else(|| ApiError::BadRequest(format!("ctx.tz {name:?} is not an IANA time zone")))?;
            Ctx { tz_offset: Some(crate::insights::localtime::zone_offset(z, at)), ..ctx.clone() }
        }
    };
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, Option<Value>, Value, Value, Option<Value>)> = sqlx::query_as(
        "SELECT s.key, s.kind, s.sorted,
                COALESCE((SELECT jsonb_agg(jsonb_build_array(v.id, v.key, v.payload, v.is_baseline, v.traffic_share, v.status, v.requires) ORDER BY v.id)
                            FROM variants v WHERE v.slot_id = s.id AND v.status IN ('approved', 'draft')), '[]'),
                COALESCE((SELECT jsonb_agg(jsonb_build_array(v.id, v.key, g.id, g.name, g.predicate, g.status) ORDER BY g.id, v.id)
                            FROM variants v JOIN segments g ON g.id = v.audience_id
                           WHERE v.slot_id = s.id AND v.status IN ('approved', 'draft')), '[]'),
                s.copy
           FROM slots s WHERE s.site_id = $1 ORDER BY s.key",
    )
    .bind(site)
    .fetch_all(pool)
    .await?;
    let bad = || ApiError::Internal(anyhow::anyhow!("preview: malformed variant rows"));
    let mut out = Vec::with_capacity(rows.len());
    for (key, kind, sorted, variants, gates, copy_spec) in rows {
        #[allow(clippy::type_complexity)]
        let all: Vec<(i64, String, Option<String>, bool, Option<f32>, String, Vec<String>)> = serde_json::from_value(variants).map_err(|_| bad())?;
        let gates: Vec<GateRow> = serde_json::from_value(gates).map_err(|_| bad())?;
        let gate_of: Vec<(i64, String)> = gates.iter().map(|g| (g.0, g.3.clone())).collect();
        let mut arms: Vec<ArmRow> = all.iter().map(|(id, k, p, b, s, _, _)| (*id, k.clone(), p.clone(), *b, *s)).collect();
        let audiences = gate_by_audience(gates, &ctx, at, night, facts, &mut arms);
        // With a persona, `requires` closes arms as decide does; the room still lists them.
        let requires: Vec<(i64, Vec<String>)> = all.iter().map(|a| (a.0, a.6.clone())).collect();
        let mut open = arms.clone();
        if persona.is_some() {
            close_by_requires(&mut open, &requires, facts);
        }
        let falls_to = arms.iter().find(|a| a.3).map(|a| a.1.clone()).unwrap_or_else(|| "the page's own version".into());
        let kept_out: Vec<Value> = audiences
            .as_array()
            .into_iter()
            .flatten()
            .filter(|a| a["matched"] == json!(false))
            .flat_map(|a| a["variants"].as_array().cloned().unwrap_or_default().into_iter().map(move |v| json!({ "key": v, "why": format!("only for {}", a["name"].as_str().unwrap_or("an audience")) })))
            .collect();
        let status_of = |id: i64| all.iter().find(|a| a.0 == id).map(|a| a.5.clone()).unwrap_or_default();
        let audience_of = |id: i64| gate_of.iter().find(|g| g.0 == id).map(|g| g.1.clone());
        let versions: Vec<Value> = match kind.as_str() {
            // A picker asks the visitor; there is nothing to flip through.
            "picker" => vec![],
            // A sorted slot's versions are one list: the default order of the items this visitor
            // may see (the learner reorders it per visitor in production).
            "sorted" => {
                let order: Vec<String> = sorted
                    .as_ref()
                    .and_then(|v| v["order"].as_array())
                    .map(|a| a.iter().filter_map(|k| k.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                let live: Vec<&ArmRow> = arms.iter().filter(|a| status_of(a.0) == "approved").collect();
                let keyed: Vec<(i64, String)> = live.iter().map(|a| (a.0, a.1.clone())).collect();
                let ids: Vec<i64> = live.iter().map(|a| a.0).collect();
                let ranked = crate::sorted::default_order(&order, &keyed, &ids);
                let keys: Vec<String> = ranked.iter().filter_map(|id| keyed.iter().find(|k| k.0 == *id).map(|k| k.1.clone())).collect();
                vec![json!({ "key": "default_order", "status": "approved", "baseline": true, "order": keys,
                             "why": "the default order; the learner reorders it for each visitor" })]
            }
            _ => {
                let rank = |a: &ArmRow| {
                    let draft = status_of(a.0) == "draft";
                    let tier = if audience_of(a.0).is_some() { 0 } else if !a.3 { 1 } else { 2 };
                    (draft, tier, a.0)
                };
                let mut sorted_arms: Vec<&ArmRow> = arms.iter().collect();
                sorted_arms.sort_by_key(|a| rank(a));
                sorted_arms
                    .into_iter()
                    .map(|(id, k, payload, baseline, share)| {
                        let status = status_of(*id);
                        let mut why = match (baseline, audience_of(*id)) {
                            (true, _) => "the page's own version".to_string(),
                            (false, Some(name)) => format!("made for {name}"),
                            (false, None) => "open to every visitor".to_string(),
                        };
                        if let (false, Some(s)) = (baseline, share) {
                            why.push_str(&format!(", up to {}% of traffic", (f64::from(*s) * 100.0).round()));
                        }
                        if status == "draft" {
                            why = format!("draft, not live yet; {why}");
                        }
                        let mut v = json!({ "id": id, "key": k, "status": status, "baseline": baseline, "why": why });
                        if !open.iter().any(|a| a.0 == *id) {
                            let needs = requires.iter().find(|r| r.0 == *id).map(|r| r.1.join(", ")).unwrap_or_default();
                            v["closed"] = json!(true);
                            v["why"] = json!(format!("needs {needs}; this visitor falls to {falls_to}"));
                            v["copy"] = Value::Null;
                        } else if kind == "copy" {
                            // A template is filled from the persona's history (none: its fallbacks).
                            let (copy, templated) = crate::copy::render(payload.as_deref(), copy_spec.as_ref(), facts);
                            v["copy"] = copy;
                            if templated {
                                v["filled"] = json!(true);
                            }
                        } else {
                            v["payload"] = json!(payload);
                        }
                        v
                    })
                    .collect()
            }
        };
        out.push(json!({ "slot": key, "kind": kind, "versions": versions, "kept_out": kept_out }));
    }
    Ok(out)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Retire {
    actor: String,
    #[serde(default)]
    comment: Option<String>,
}

/// A person takes a live variant off the site from the Preview room: it leaves the arm set, and
/// the approvals log says who and why. The page's own version (the baseline) cannot be retired.
async fn retire(State(state): State<Arc<AppState>>, Path(id): Path<i64>, Json(b): Json<Retire>) -> Result<Json<Value>, ApiError> {
    let who = super::admin::actor(&b.actor)?;
    let mut tx = state.pool.begin().await?;
    let row: Option<(String, String, String)> = sqlx::query_as(
        "UPDATE variants v SET status = 'retired' FROM slots s
          WHERE v.id = $1 AND s.id = v.slot_id AND v.status IN ('approved', 'paused') AND NOT v.is_baseline
          RETURNING s.site_id, s.key, v.key",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((site, slot, key)) = row else {
        return Err(ApiError::Conflict(format!("variant {id} is not a live variant that can be taken off (the page's own version stays)")));
    };
    sqlx::query("INSERT INTO approvals (variant_id, action, actor, comment) VALUES ($1, 'retire', $2, $3)")
        .bind(id)
        .bind(&who)
        .bind(b.comment.as_deref().unwrap_or("rejected in preview"))
        .execute(&mut *tx)
        .await?;
    crate::webhooks::enqueue(&mut *tx, &site, "variant.retired", json!({ "variant_id": id, "slot": slot, "variant": key, "retired_by": who })).await?;
    tx.commit().await?;
    crate::edge::changed(&state.pool, &state.edge, &site).await;
    Ok(Json(json!({ "id": id, "status": "retired", "variant": key })))
}
