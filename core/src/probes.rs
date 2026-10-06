//! The install wizard's probes. A step is done when Core **observes** the thing, never when the
//! owner ticks a box (frontend.md, "Onboarding and install flow").
//!
//! The probes run in Core rather than in the dashboard's browser, because a browser cannot read
//! a cross-origin response from the owner's site without CORS headers nobody has set up yet.
//!
//! 1. `framework` is a choice, not a probe.
//! 2. `package`: `GET {origin}/` carries `x-prb-sdk` (set by `@pratibimba/next` and
//!    `@pratibimba/edge`) or the page loads `/_prb/sdk.js` (the script fallback).
//! 3. `reach`: `GET {origin}/_prb/manifest/{site}` answers with this site's manifest.
//! 4. `slot`: an `exposure` event has arrived for this site.
//! 5. `agent`: a coding agent's MCP tool call has run (not been refused) on this site.
//! 6. `live`: a variant approved through the dashboard gate (not `apply`) has been shown.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Choice,
    Done,
    Pending,
    Failed,
    Later,
}

#[derive(Clone, Debug, Serialize)]
pub struct Step {
    pub id: &'static str,
    pub state: State,
    pub detail: String,
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent(concat!("pratibimba-probe/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("static client config")
}

fn base(origin: &str) -> &str {
    origin.trim_end_matches('/')
}

pub async fn package(http: &reqwest::Client, origin: &str) -> Step {
    let url = format!("{}/", base(origin));
    let step = |state, detail: String| Step { id: "package", state, detail };
    match http.get(&url).send().await {
        Err(e) => step(State::Failed, format!("Could not reach {url}: {e}")),
        Ok(r) => {
            let status = r.status();
            if let Some(sdk) = r.headers().get("x-prb-sdk").and_then(|v| v.to_str().ok()) {
                return step(State::Done, format!("{url} answered with x-prb-sdk: {sdk}"));
            }
            let body = r.text().await.unwrap_or_default();
            if body.contains("/_prb/sdk.js") {
                step(State::Done, format!("{url} loads /_prb/sdk.js (the script fallback)"))
            } else {
                step(State::Pending, format!("Fetched {url} ({status}) but saw no x-prb-sdk header and no /_prb/sdk.js script tag."))
            }
        }
    }
}

pub async fn reach(http: &reqwest::Client, origin: &str, site: &str) -> Step {
    let url = format!("{}/_prb/manifest/{site}", base(origin));
    let step = |state, detail: String| Step { id: "reach", state, detail };
    match http.get(&url).send().await {
        Err(e) => step(State::Failed, format!("Could not reach {url}: {e}")),
        Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND => step(
            State::Pending,
            format!(
                "{url} returned 404: the /_prb route is not mounted. In Next.js add app/%5Fprb/[...path]/route.ts (a plain _prb folder is private in the App Router); elsewhere proxy /_prb/* to Core."
            ),
        ),
        Ok(r) if !r.status().is_success() => step(State::Failed, format!("{url} returned {}", r.status())),
        Ok(r) => match serde_json::from_str::<serde_json::Value>(&r.text().await.unwrap_or_default()) {
            Ok(m) if m.get("site").and_then(|s| s.as_str()) == Some(site) => {
                let n = m.get("slots").and_then(|s| s.as_array()).map(|a| a.len()).unwrap_or(0);
                step(State::Done, format!("{url} answered with this site's manifest ({n} slot{})", if n == 1 { "" } else { "s" }))
            }
            Ok(_) => step(State::Failed, format!("{url} answered, but not with the manifest for {site:?}")),
            Err(_) => step(State::Failed, format!("{url} answered with something that is not JSON")),
        },
    }
}

pub async fn slot(pool: &PgPool, site: &str) -> Result<Step> {
    let first: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT min(received_at) FROM events WHERE site_id = $1 AND name = 'exposure'").bind(site).fetch_one(pool).await?;
    Ok(match first {
        Some(at) => Step { id: "slot", state: State::Done, detail: format!("First exposure arrived at {}", at.format("%Y-%m-%d %H:%M UTC")) },
        None => Step { id: "slot", state: State::Pending, detail: "Waiting for the first exposure from a marked slot.".into() },
    })
}

pub async fn live(pool: &PgPool, site: &str) -> Result<Step> {
    let gated: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT s.key, v.key, EXISTS (SELECT 1 FROM decisions d WHERE d.variant_id = v.id AND d.exposed_at IS NOT NULL)
           FROM variants v JOIN slots s ON s.id = v.slot_id
          WHERE s.site_id = $1 AND v.status = 'approved' AND NOT v.is_baseline AND v.approved_by <> $2
          ORDER BY v.approved_at",
    )
    .bind(site)
    .bind(crate::site_file::APPROVER)
    .fetch_all(pool)
    .await?;
    Ok(match gated.iter().find(|g| g.2) {
        Some((slot, key, _)) => Step { id: "live", state: State::Done, detail: format!("`{key}` on `{slot}` was approved here and has been shown to a visitor.") },
        None => match gated.first() {
            Some((slot, key, _)) => {
                Step { id: "live", state: State::Pending, detail: format!("`{key}` on `{slot}` is approved; waiting for a visitor to see it.") }
            }
            None => Step { id: "live", state: State::Pending, detail: "No variant has been approved through the dashboard yet.".into() },
        },
    })
}

pub async fn agent(pool: &PgPool, site: &str) -> Result<Step> {
    let last: Option<(String, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT agent, tool, created_at FROM mcp_calls WHERE site_id = $1 AND outcome <> 'refused' ORDER BY created_at DESC LIMIT 1",
    )
    .bind(site)
    .fetch_optional(pool)
    .await?;
    Ok(match last {
        Some((agent, tool, at)) => Step {
            id: "agent",
            state: State::Done,
            detail: format!("{agent} called `{tool}` over MCP at {}.", at.format("%Y-%m-%d %H:%M UTC")),
        },
        None => Step {
            id: "agent",
            state: State::Pending,
            detail: format!("No MCP tool call has arrived yet. Mint a token with `pratibimba mcp-token {site} <name>` and connect your agent."),
        },
    })
}

pub async fn all(pool: &PgPool, http: &reqwest::Client, site: &str, origin: &str) -> Result<Vec<Step>> {
    let (package, reach) = tokio::join!(package(http, origin), reach(http, origin, site));
    Ok(vec![
        Step { id: "framework", state: State::Choice, detail: "Next.js is the primary install; the edge worker and the script are the alternatives.".into() },
        package,
        reach,
        slot(pool, site).await?,
        agent(pool, site).await?,
        live(pool, site).await?,
    ])
}
