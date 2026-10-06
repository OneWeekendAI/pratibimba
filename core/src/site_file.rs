//! `pratibimba apply site.json`: the Phase 0 way to declare sites, slots and variants.
//!
//! The file is declarative. Applying it upserts the site and each slot. Variants listed in the
//! file become `approved` by `cli:apply`, because applying a file is the owner's own act.
//! Approved variants of an applied slot that the file no longer lists are retired. Drafts are
//! never touched, and nothing is approved except what the file names. The dashboard approval
//! gate replaces this in Phase 1.

use anyhow::{Result, bail};
use serde::Deserialize;
use crate::webhooks;
use serde_json::json;
use sqlx::PgPool;

pub const APPROVER: &str = "cli:apply";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteFile {
    pub site: String,
    pub origin: String,
    /// Share of visitors (0 to 0.5) who always get the baseline. Omitted: keep the current value.
    #[serde(default)]
    pub holdout_fraction: Option<f32>,
    /// The site's facts.md, as text: what an agent may claim. Omitted: keep the current value.
    #[serde(default)]
    pub facts: Option<String>,
    pub slots: Vec<SlotSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotSpec {
    pub key: String,
    pub selector: String,
    #[serde(default = "default_metric")]
    pub metric: String,
    #[serde(default = "default_window")]
    pub attribution_window_secs: i32,
    #[serde(default = "default_ttl")]
    pub decision_ttl_secs: i32,
    /// `swap` (default) or `picker` (see [`crate::stated`]).
    #[serde(default)]
    pub kind: Option<String>,
    /// Ship payloads and weights in the manifest so the SDK may re-score this slot on the device.
    #[serde(default)]
    pub in_session: bool,
    /// A picker slot's options. Required for `kind: "picker"`, refused otherwise.
    #[serde(default)]
    pub picker: Option<crate::stated::PickerSpec>,
    pub variants: Vec<VariantSpec>,
}

impl SlotSpec {
    pub fn is_picker(&self) -> bool {
        self.kind.as_deref() == Some("picker")
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VariantSpec {
    pub key: String,
    #[serde(default)]
    pub baseline: bool,
    /// HTML fragment swapped into the slot. The baseline has none: it is the page's own content.
    pub html: Option<String>,
}

fn default_metric() -> String {
    "click".into()
}
fn default_window() -> i32 {
    1800
}
fn default_ttl() -> i32 {
    60
}

/// Lowercase slug: letters, digits, `-` and `_`, 1-64 chars.
pub fn is_slug(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// Event and metric names: lowercase letters, digits, `_` and `:`, 1-64 chars.
pub fn is_event_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b':')
}

impl SiteFile {
    pub fn validate(&self) -> Result<()> {
        if !is_slug(&self.site) {
            bail!("site {:?}: must be a lowercase slug", self.site);
        }
        if let Some(h) = self.holdout_fraction {
            if !(0.0..=0.5).contains(&h) {
                bail!("holdout_fraction must be between 0 and 0.5");
            }
        }
        for slot in &self.slots {
            let k = &slot.key;
            if !is_slug(k) {
                bail!("slot {k:?}: key must be a lowercase slug");
            }
            if slot.selector.trim().is_empty() {
                bail!("slot {k:?}: selector is empty");
            }
            if !is_event_name(&slot.metric) {
                bail!("slot {k:?}: metric {:?} is not a valid event name", slot.metric);
            }
            if slot.attribution_window_secs <= 0 || slot.decision_ttl_secs < 0 {
                bail!("slot {k:?}: attribution_window_secs must be > 0 and decision_ttl_secs >= 0");
            }
            crate::stated::validate_slot(slot)?;
            let baselines = slot.variants.iter().filter(|v| v.baseline).count();
            if baselines != 1 {
                bail!("slot {k:?}: exactly one variant must be marked baseline (found {baselines})");
            }
            let mut keys = std::collections::HashSet::new();
            for v in &slot.variants {
                if !is_slug(&v.key) || !keys.insert(&v.key) {
                    bail!("slot {k:?}: variant key {:?} is invalid or repeated", v.key);
                }
                match (v.baseline, &v.html) {
                    (true, Some(_)) => bail!("slot {k:?}: baseline {:?} must not carry html; it is the page's own content", v.key),
                    (false, None) if !slot.is_picker() => bail!("slot {k:?}: variant {:?} needs html", v.key),
                    _ => {}
                }
            }
        }
        crate::stated::validate_site(self)?;
        Ok(())
    }
}

pub async fn apply_path(pool: &PgPool, path: &std::path::Path) -> Result<()> {
    let text = std::fs::read_to_string(path)?;
    let file: SiteFile = serde_json::from_str(&text)?;
    apply(pool, &file).await
}

pub async fn apply(pool: &PgPool, file: &SiteFile) -> Result<()> {
    file.validate()?;
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO sites (id, origin, holdout_fraction, facts) VALUES ($1, $2, COALESCE($3, 0.10), $4)
         ON CONFLICT (id) DO UPDATE SET origin = EXCLUDED.origin, holdout_fraction = COALESCE($3, sites.holdout_fraction),
                                        facts = COALESCE($4, sites.facts)",
    )
        .bind(&file.site)
        .bind(&file.origin)
        .bind(file.holdout_fraction)
        .bind(&file.facts)
        .execute(&mut *tx)
        .await?;
    for slot in &file.slots {
        let slot_id: i64 = sqlx::query_scalar(
            "INSERT INTO slots (site_id, key, selector, metric, attribution_window_secs, decision_ttl_secs, kind, in_session, picker)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (site_id, key) DO UPDATE SET selector = EXCLUDED.selector, metric = EXCLUDED.metric,
                 attribution_window_secs = EXCLUDED.attribution_window_secs, decision_ttl_secs = EXCLUDED.decision_ttl_secs,
                 kind = EXCLUDED.kind, in_session = EXCLUDED.in_session, picker = EXCLUDED.picker
             RETURNING id",
        )
        .bind(&file.site)
        .bind(&slot.key)
        .bind(&slot.selector)
        .bind(&slot.metric)
        .bind(slot.attribution_window_secs)
        .bind(slot.decision_ttl_secs)
        .bind(slot.kind.as_deref().unwrap_or("swap"))
        .bind(slot.in_session)
        .bind(slot.picker.as_ref().map(serde_json::to_value).transpose()?)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO models (slot_id) VALUES ($1) ON CONFLICT DO NOTHING").bind(slot_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO model_versions (slot_id, version, state, reason) SELECT slot_id, version, state, 'init' FROM models WHERE slot_id = $1 ON CONFLICT DO NOTHING")
            .bind(slot_id)
            .execute(&mut *tx)
            .await?;
        // Which listed variants this apply newly approves: the webhook says only what changed.
        let before: Vec<(String, String)> =
            sqlx::query_as("SELECT key, status FROM variants WHERE slot_id = $1").bind(slot_id).fetch_all(&mut *tx).await?;
        // Clear the baseline flag first so moving it between variants never trips the
        // one-baseline index halfway through.
        sqlx::query("UPDATE variants SET is_baseline = false WHERE slot_id = $1").bind(slot_id).execute(&mut *tx).await?;
        for v in &slot.variants {
            sqlx::query(
                "INSERT INTO variants (slot_id, key, payload, is_baseline, status, approved_by, approved_at, approved_model_version)
                 VALUES ($1, $2, $3, $4, 'approved', $5, now(), (SELECT version FROM models WHERE slot_id = $1))
                 ON CONFLICT (slot_id, key) DO UPDATE SET payload = EXCLUDED.payload, is_baseline = EXCLUDED.is_baseline,
                     status = 'approved', approved_by = EXCLUDED.approved_by, approved_at = now(),
                     approved_model_version = COALESCE(variants.approved_model_version, EXCLUDED.approved_model_version),
                     traffic_share = NULL",
            )
            .bind(slot_id)
            .bind(&v.key)
            .bind(&v.html)
            .bind(v.baseline)
            .bind(APPROVER)
            .execute(&mut *tx)
            .await?;
        }
        for v in &slot.variants {
            if !before.iter().any(|(k, s)| k == &v.key && s == "approved") {
                let data = json!({ "slot": slot.key, "variant": v.key, "approved_by": APPROVER, "traffic_share": null });
                webhooks::enqueue(&mut *tx, &file.site, "variant.approved", data).await?;
            }
        }
        let keys: Vec<&str> = slot.variants.iter().map(|v| v.key.as_str()).collect();
        let retired: Vec<String> = sqlx::query_scalar(
            "UPDATE variants SET status = 'retired' WHERE slot_id = $1 AND status = 'approved' AND NOT (key = ANY($2)) RETURNING key",
        )
        .bind(slot_id)
        .bind(&keys)
        .fetch_all(&mut *tx)
        .await?;
        for key in retired {
            let data = json!({ "slot": slot.key, "variant": key, "retired_by": APPROVER });
            webhooks::enqueue(&mut *tx, &file.site, "variant.retired", data).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}
