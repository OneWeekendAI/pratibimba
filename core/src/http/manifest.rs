//! `GET /manifest/:site` (JSON) and `GET /manifest/:site.js` (a script that sets
//! `window.__prb_manifest`). The script form exists so the client can know the slot selectors
//! synchronously, before first paint, which is the only moment hiding them prevents flicker.
//! Approved variants only; payloads travel with decisions, not here.

use super::{ApiError, AppState};
use axum::{
    extract::{Path, State},
    http::header,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use std::sync::Arc;

const CACHE: &str = "public, max-age=60, stale-while-revalidate=600";

#[derive(Serialize)]
pub struct Manifest {
    pub site: String,
    pub slots: Vec<ManifestSlot>,
}

#[derive(Serialize)]
pub struct ManifestSlot {
    pub key: String,
    pub selector: String,
    pub metric: String,
    pub ttl_secs: i32,
    pub variants: Vec<ManifestVariant>,
}

#[derive(Serialize)]
pub struct ManifestVariant {
    pub key: String,
    pub baseline: bool,
}

pub async fn load(pool: &sqlx::PgPool, site: &str) -> Result<Manifest, ApiError> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sites WHERE id = $1)").bind(site).fetch_one(pool).await?;
    if !exists {
        return Err(ApiError::NotFound(format!("unknown site {site:?}")));
    }
    let rows: Vec<(String, String, String, i32, String, bool)> = sqlx::query_as(
        "SELECT s.key, s.selector, s.metric, s.decision_ttl_secs, v.key, v.is_baseline
           FROM slots s JOIN variants v ON v.slot_id = s.id AND v.status = 'approved'
          WHERE s.site_id = $1
          ORDER BY s.key, v.id",
    )
    .bind(site)
    .fetch_all(pool)
    .await?;
    let mut slots: Vec<ManifestSlot> = vec![];
    for (key, selector, metric, ttl_secs, vkey, baseline) in rows {
        if slots.last().map(|s| s.key != key).unwrap_or(true) {
            slots.push(ManifestSlot { key, selector, metric, ttl_secs, variants: vec![] });
        }
        slots.last_mut().unwrap().variants.push(ManifestVariant { key: vkey, baseline });
    }
    Ok(Manifest { site: site.to_string(), slots })
}

pub async fn handler(State(state): State<Arc<AppState>>, Path(site): Path<String>) -> Result<Response, ApiError> {
    let (site, as_js) = match site.strip_suffix(".js") {
        Some(s) => (s.to_string(), true),
        None => (site, false),
    };
    let manifest = load(&state.pool, &site).await?;
    let json = serde_json::to_string(&manifest).map_err(anyhow::Error::from)?;
    Ok(if as_js {
        // `</` cannot appear in the output: serde_json leaves `/` alone, so escape it to keep
        // a selector like `</script>` from closing an inline copy of this script.
        let body = format!("window.__prb_manifest={};", json.replace("</", "<\\/"));
        ([(header::CONTENT_TYPE, "application/javascript; charset=utf-8"), (header::CACHE_CONTROL, CACHE)], body).into_response()
    } else {
        ([(header::CONTENT_TYPE, "application/json"), (header::CACHE_CONTROL, CACHE)], json).into_response()
    })
}
