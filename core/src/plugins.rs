//! Plugins: compiled-in, named extensions of known kinds. There is no dynamic code loading: the
//! catalogue below is every plugin this binary has, and installing one records it (and its
//! config) for a site in `plugins` and turns its effect on.
//!
//! - **`attributor`**: replaces the site's attributor ([`crate::attribution`]). The worker uses
//!   it for every slot that does not name its own; uninstalling falls back to the site file's
//!   `attribution`. At most one per site (a unique index; this build has one attributor plugin).
//! - **`event_sink`**: forwards the site's events somewhere. It is a webhook subscription
//!   ([`crate::webhooks`]) the plugin owns: created on install, deleted on uninstall.
//!
//! An agent cannot install one: MCP's `install_plugin` records a request ([`request`]) that a
//! person confirms or declines (`/api/plugin-requests/{id}/...`), as with `retire_variant`.
//!
//! Each plugin declares `deanonymiser`: whether it hands visitor identity to a third party. Core
//! ships none switched on; while one is installed the site carries a warning ([`warnings`]),
//! which the API returns and the dashboard shows.

use crate::http::ApiError;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Carries the attributor's name in [`crate::attribution::NAMES`].
    Attributor(&'static str),
    EventSink,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Attributor(_) => "attributor",
            Kind::EventSink => "event_sink",
        }
    }
}

pub struct Plugin {
    pub name: &'static str,
    pub kind: Kind,
    pub description: &'static str,
    pub deanonymiser: bool,
}

pub const CATALOGUE: &[Plugin] = &[
    Plugin {
        name: "attribution.linear",
        kind: Kind::Attributor("linear"),
        description: "Multi-touch attribution: one conversion's credit is split evenly across every open exposed decision in its window. Takes no config.",
        deanonymiser: false,
    },
    Plugin {
        name: "sink.identity_resolver",
        kind: Kind::EventSink,
        description: "Forwards every exposure and conversion, subject id included, to a third-party identity resolver at `url` (an RB2B-style de-anonymiser). Config: {\"url\": \"https://...\"}.",
        deanonymiser: true,
    },
];

/// The events an event sink receives.
pub const SINK_EVENTS: [&str; 2] = ["exposure", "conversion"];

pub fn by_name(name: &str) -> Option<&'static Plugin> {
    CATALOGUE.iter().find(|p| p.name == name)
}

pub fn known(name: &str) -> Result<&'static Plugin, ApiError> {
    by_name(name).ok_or_else(|| {
        ApiError::NotFound(format!("unknown plugin {name:?}; this build has {:?}", CATALOGUE.iter().map(|p| p.name).collect::<Vec<_>>()))
    })
}

/// The config a plugin takes, checked: absent means `{}`, and any key it does not take is refused.
pub fn check_config(p: &Plugin, config: Option<&Value>) -> Result<Value, ApiError> {
    let empty = json!({});
    let c = config.unwrap_or(&empty);
    let Some(obj) = c.as_object() else {
        return Err(ApiError::BadRequest("config must be a JSON object".into()));
    };
    let takes: &[&str] = match p.kind {
        Kind::Attributor(_) => &[],
        Kind::EventSink => &["url"],
    };
    if let Some(k) = obj.keys().find(|k| !takes.contains(&k.as_str())) {
        return Err(ApiError::BadRequest(format!("{} takes no config key {k:?}", p.name)));
    }
    if p.kind == Kind::EventSink {
        let url = obj.get("url").and_then(Value::as_str).map(str::trim).unwrap_or("");
        if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 2048 {
            return Err(ApiError::BadRequest(format!("{} needs config.url, an http(s) URL", p.name)));
        }
        return Ok(json!({ "url": url }));
    }
    Ok(json!({}))
}

fn warning(p: &Plugin, config: &Value, by: &str) -> String {
    let to = config["url"].as_str().map(|u| format!(" to {u}")).unwrap_or_default();
    format!("{} is a de-anonymiser: it sends visitor identity{to}. Installed by {by}.", p.name)
}

async fn site_exists(conn: &mut PgConnection, site: &str) -> Result<(), ApiError> {
    let found: Option<i32> = sqlx::query_scalar("SELECT 1 FROM sites WHERE id = $1").bind(site).fetch_optional(&mut *conn).await?;
    found.map(|_| ()).ok_or_else(|| ApiError::NotFound(format!("unknown site {site:?}")))
}

/// Install `p` on `site` with an already checked `config`, inside the caller's transaction.
pub async fn install(conn: &mut PgConnection, site: &str, p: &Plugin, config: Value, by: &str) -> Result<Value, ApiError> {
    site_exists(conn, site).await?;
    let taken: Option<i32> = sqlx::query_scalar("SELECT 1 FROM plugins WHERE site_id = $1 AND name = $2")
        .bind(site)
        .bind(p.name)
        .fetch_optional(&mut *conn)
        .await?;
    if taken.is_some() {
        return Err(ApiError::Conflict(format!("{} is already installed on {site:?}", p.name)));
    }
    let mut out = json!({ "site": site, "plugin": p.name, "kind": p.kind.as_str(), "config": config, "installed_by": by,
                          "deanonymiser": p.deanonymiser });
    let (attributor, subscription) = match p.kind {
        Kind::Attributor(a) => (Some(a), None),
        Kind::EventSink => {
            let secret = crate::webhooks::new_secret();
            let id: i64 = sqlx::query_scalar("INSERT INTO webhook_subscriptions (site_id, url, events, secret) VALUES ($1, $2, $3, $4) RETURNING id")
                .bind(site)
                .bind(config["url"].as_str())
                .bind(&SINK_EVENTS[..])
                .bind(&secret)
                .fetch_one(&mut *conn)
                .await?;
            // Shown once, as for any webhook subscription.
            out["secret"] = json!(secret);
            out["subscription_id"] = json!(id);
            (None, Some(id))
        }
    };
    sqlx::query(
        "INSERT INTO plugins (site_id, name, kind, config, deanonymiser, attributor, subscription_id, installed_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(site)
    .bind(p.name)
    .bind(p.kind.as_str())
    .bind(&config)
    .bind(p.deanonymiser)
    .bind(attributor)
    .bind(subscription)
    .bind(by)
    .execute(&mut *conn)
    .await?;
    if p.deanonymiser {
        out["warning"] = json!(warning(p, &config, by));
    }
    Ok(out)
}

/// Uninstall `name` from `site`: the row goes, and so does anything the plugin owned.
pub async fn uninstall(pool: &PgPool, site: &str, name: &str, by: &str) -> Result<Value, ApiError> {
    let p = known(name)?;
    let mut tx = pool.begin().await?;
    site_exists(&mut tx, site).await?;
    let row: Option<(Option<i64>,)> = sqlx::query_as("DELETE FROM plugins WHERE site_id = $1 AND name = $2 RETURNING subscription_id")
        .bind(site)
        .bind(p.name)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((subscription,)) = row else {
        return Err(ApiError::NotFound(format!("{} is not installed on {site:?}", p.name)));
    };
    if let Some(id) = subscription {
        sqlx::query("DELETE FROM webhook_subscriptions WHERE id = $1").bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(json!({ "site": site, "plugin": p.name, "status": "uninstalled", "uninstalled_by": by }))
}

/// One line per installed de-anonymiser on `site`, oldest first.
pub async fn warnings(pool: &PgPool, site: &str) -> Result<Vec<String>, ApiError> {
    let rows: Vec<(String, Value, String)> =
        sqlx::query_as("SELECT name, config, installed_by FROM plugins WHERE site_id = $1 AND deanonymiser ORDER BY installed_at, name")
            .bind(site)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().filter_map(|(n, c, by)| Some(warning(by_name(&n)?, &c, &by))).collect())
}

/// The catalogue as `site` sees it: each plugin with its install (or null), plus the warnings.
pub async fn listing(pool: &PgPool, site: &str) -> Result<Value, ApiError> {
    site_exists(&mut *pool.acquire().await?, site).await?;
    let rows: Vec<(String, Value, String, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as("SELECT name, config, installed_by, installed_at FROM plugins WHERE site_id = $1").bind(site).fetch_all(pool).await?;
    // Each plugin's latest request, so an agent reads its answer (and a decline's reason).
    let reqs: Vec<(String, Value)> = sqlx::query_as(
        "SELECT DISTINCT ON (name) name, jsonb_build_object('id', id, 'status', status, 'config', config, 'agent', agent,
                'decided_by', decided_by, 'comment', comment, 'at', created_at)
           FROM plugin_requests WHERE site_id = $1 ORDER BY name, id DESC",
    )
    .bind(site)
    .fetch_all(pool)
    .await?;
    let plugins: Vec<Value> = CATALOGUE
        .iter()
        .map(|p| {
            let installed = rows.iter().find(|r| r.0 == p.name).map(|(_, c, by, at)| json!({ "config": c, "by": by, "at": at }));
            let request = reqs.iter().find(|r| r.0 == p.name).map(|r| r.1.clone());
            json!({ "name": p.name, "kind": p.kind.as_str(), "description": p.description, "deanonymiser": p.deanonymiser,
                    "installed": installed, "request": request })
        })
        .collect();
    Ok(json!({ "site": site, "plugins": plugins, "warnings": warnings(pool, site).await? }))
}

/// An agent's install request: checked now (known plugin, valid config, not installed, no open
/// request), installed only when a person confirms it.
pub async fn request(pool: &PgPool, site: &str, name: &str, config: Option<&Value>, call_row: i64, agent: &str, reason: &str) -> Result<Value, ApiError> {
    let p = known(name)?;
    let config = check_config(p, config)?;
    let installed: Option<i32> = sqlx::query_scalar("SELECT 1 FROM plugins WHERE site_id = $1 AND name = $2").bind(site).bind(p.name).fetch_optional(pool).await?;
    if installed.is_some() {
        return Err(ApiError::Conflict(format!("{} is already installed on {site:?}", p.name)));
    }
    let id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO plugin_requests (site_id, name, config, mcp_call_id, agent, reason) VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (site_id, name) WHERE status = 'pending' DO NOTHING RETURNING id",
    )
    .bind(site)
    .bind(p.name)
    .bind(&config)
    .bind(call_row)
    .bind(agent.chars().take(128).collect::<String>())
    .bind(reason)
    .fetch_optional(pool)
    .await?;
    let Some(id) = id else {
        return Err(ApiError::Conflict(format!("{} already has an open install request; see list_plugins", p.name)));
    };
    Ok(json!({
        "id": id, "plugin": p.name, "config": config, "status": "requested", "deanonymiser": p.deanonymiser,
        "next": "Request recorded. Nothing is installed until a person confirms it; the answer appears on list_plugins.",
    }))
}

/// A person's answer to a request: confirm installs the plugin with the requested config, in the
/// same transaction; decline needs a reason.
pub async fn decide(pool: &PgPool, id: i64, confirm: bool, by: &str, comment: Option<&str>) -> Result<Value, ApiError> {
    let mut tx = pool.begin().await?;
    let row: Option<(String, String, Value, String)> =
        sqlx::query_as("SELECT site_id, name, config, status FROM plugin_requests WHERE id = $1 FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?;
    let Some((site, name, config, status)) = row else {
        return Err(ApiError::NotFound(format!("unknown plugin request {id}")));
    };
    if status != "pending" {
        return Err(ApiError::Conflict(format!("plugin request {id} is already {status}")));
    }
    let mut out = json!({ "id": id, "plugin": name, "status": if confirm { "confirmed" } else { "declined" } });
    if confirm {
        out["install"] = install(&mut tx, &site, known(&name)?, config, by).await?;
    }
    sqlx::query("UPDATE plugin_requests SET status = $2, decided_by = $3, decided_at = now(), comment = $4 WHERE id = $1")
        .bind(id)
        .bind(if confirm { "confirmed" } else { "declined" })
        .bind(by)
        .bind(comment)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(out)
}

/// Open requests for the approvals queue, oldest first, with the audited call behind each.
pub async fn open_requests(pool: &PgPool, site: &str) -> Result<Vec<Value>, ApiError> {
    let rows: Vec<(Value,)> = sqlx::query_as(
        "SELECT jsonb_build_object('id', r.id, 'plugin', r.name, 'config', r.config, 'reason', r.reason, 'at', r.created_at,
                'agent', jsonb_build_object('name', r.agent, 'justification', c.justification, 'on_behalf_of', c.on_behalf_of,
                                            'token', k.name, 'audit_id', c.id, 'tool', c.tool))
           FROM plugin_requests r LEFT JOIN mcp_calls c ON c.id = r.mcp_call_id LEFT JOIN mcp_tokens k ON k.id = c.token_id
          WHERE r.site_id = $1 AND r.status = 'pending' ORDER BY r.created_at, r.id",
    )
    .bind(site)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}
