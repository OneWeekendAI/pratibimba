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
    /// The site's consent mode; the SDK uses it to choose between a cookie and session storage
    /// (see [`crate::privacy`]).
    pub privacy_mode: String,
    pub slots: Vec<ManifestSlot>,
    /// The site's declared features with their PII tags (see [`crate::features`]): the SDK
    /// sends only these names, and a PII one only when the visitor is identified.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<crate::features::FeatureSpec>,
    /// The site's night window, `[start, end)` in local hours, when an in-session slot ships:
    /// the on-device scorer computes `is_night_local` from the browser clock with it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub night: Option<NightSpec>,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct NightSpec {
    pub start: i16,
    pub end: i16,
}

#[derive(Serialize)]
pub struct ManifestSlot {
    pub key: String,
    pub selector: String,
    pub metric: String,
    pub ttl_secs: i32,
    pub variants: Vec<ManifestVariant>,
    /// `"picker"` for a picker slot; absent for a swap slot. Set by [`crate::adapt::enrich`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picker: Option<crate::stated::PickerSpec>,
    /// A sorted slot's `{visible}` (see [`crate::sorted`]); its `variants` are its items, in the
    /// declared default order. Set by [`crate::adapt::enrich`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sorted: Option<serde_json::Value>,
    /// `true` when the SDK may re-score this slot on the device.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_session: Option<bool>,
}

#[derive(Serialize)]
pub struct ManifestVariant {
    pub key: String,
    pub baseline: bool,
    /// In-session slots only: the HTML (null = the page's own) and the linear weights by
    /// feature name that the on-device scorer dots with the visitor's features.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weights: Option<serde_json::Value>,
}

pub async fn load(pool: &sqlx::PgPool, site: &str) -> Result<Manifest, ApiError> {
    let mode: Option<String> = sqlx::query_scalar("SELECT privacy_mode FROM sites WHERE id = $1").bind(site).fetch_optional(pool).await?;
    let Some(privacy_mode) = mode else {
        return Err(ApiError::NotFound(format!("unknown site {site:?}")));
    };
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
            slots.push(ManifestSlot { key, selector, metric, ttl_secs, variants: vec![], kind: None, picker: None, sorted: None, in_session: None });
        }
        slots.last_mut().unwrap().variants.push(ManifestVariant { key: vkey, baseline, payload: None, weights: None });
    }
    let features = crate::features::declared(pool, site).await?.into_iter().map(|(key, pii)| crate::features::FeatureSpec { key, pii }).collect();
    let mut manifest = Manifest { site: site.to_string(), privacy_mode, slots, features, night: None };
    crate::adapt::enrich(pool, &mut manifest).await?;
    Ok(manifest)
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
