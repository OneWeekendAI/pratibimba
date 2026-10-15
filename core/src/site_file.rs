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
    /// `cookieless`, `consent_aware` (the default for a new site) or `full`; see
    /// [`crate::privacy`]. Omitted: keep the current value.
    #[serde(default)]
    pub privacy_mode: Option<String>,
    /// The optional fields this site collects ([`crate::privacy::FIELDS`]). Omitted: keep the
    /// current list (a new site collects every field).
    #[serde(default)]
    pub fields_allowed: Option<Vec<String>>,
    /// Features the client may send in `ctx.features`, each tagged `pii` (see
    /// [`crate::features`]). Omitted: keep the current list; `[]` clears it.
    #[serde(default)]
    pub features: Option<Vec<crate::features::FeatureSpec>>,
    /// How a conversion's credit is shared among decisions ([`crate::attribution`]):
    /// `last_touch` (the default for a new site) or `linear`. Omitted: keep the current value.
    #[serde(default)]
    pub attribution: Option<String>,
    /// ISO 4217 code revenue is counted in (see [`crate::commerce`]); a new site gets USD.
    /// Omitted: keep the current value.
    #[serde(default)]
    pub currency: Option<String>,
    /// Visitor history as learner features ([`crate::history`]). Omitted: keep the current
    /// block; `enabled: false` turns it off.
    #[serde(default)]
    pub history: Option<crate::history::Spec>,
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
    /// Exposures during which every approved arm takes an equal turn before LinTS decides
    /// (see [`crate::http::decide`]). 0 switches cold start off.
    #[serde(default = "default_cold_start")]
    pub cold_start_exposures: i32,
    /// `swap` (default) or `picker` (see [`crate::stated`]).
    #[serde(default)]
    pub kind: Option<String>,
    /// Ship payloads and weights in the manifest so the SDK may re-score this slot on the device.
    #[serde(default)]
    pub in_session: bool,
    /// A picker slot's options. Required for `kind: "picker"`, refused otherwise.
    #[serde(default)]
    pub picker: Option<crate::stated::PickerSpec>,
    /// Overrides the site's attributor for this slot. Omitted: follow the site.
    #[serde(default)]
    pub attribution: Option<String>,
    /// What the bandit optimises: `conversion` (the default: the metric) or `revenue` (the
    /// revenue credited to a decision over `revenue_cap`, capped at 1).
    #[serde(default)]
    pub goal: Option<String>,
    /// Required with `goal: "revenue"`, refused otherwise: an order value at or above which a
    /// decision counts as a full success. Pick about the 95th percentile of an order.
    #[serde(default)]
    pub revenue_cap: Option<f64>,
    /// A `kind: "sorted"` slot's options (see [`crate::sorted`]); refused on any other kind.
    #[serde(default)]
    pub sorted: Option<crate::sorted::SortedSpec>,
    /// A `kind: "copy"` slot's fields (see [`crate::copy`]); refused on any other kind.
    #[serde(default)]
    pub copy: Option<crate::copy::CopySpec>,
    pub variants: Vec<VariantSpec>,
}

impl SlotSpec {
    pub fn is_picker(&self) -> bool {
        self.kind.as_deref() == Some("picker")
    }
    pub fn is_copy(&self) -> bool {
        self.kind.as_deref() == Some(crate::copy::KIND)
    }
    pub fn is_sorted(&self) -> bool {
        self.kind.as_deref() == Some(crate::sorted::KIND)
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
    /// A copy slot's fields (see [`crate::copy`]), instead of html.
    #[serde(default)]
    pub copy: Option<crate::copy::Text>,
}

fn default_metric() -> String {
    "click".into()
}
pub fn default_window() -> i32 {
    1800
}
pub fn default_ttl() -> i32 {
    60
}
pub const MAX_COLD_START: i32 = 100_000;
pub fn default_cold_start() -> i32 {
    100
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
        if let Err(e) = crate::privacy::validate(self.privacy_mode.as_deref(), self.fields_allowed.as_deref()) {
            bail!("{e}");
        }
        if let Some(f) = &self.features {
            crate::features::validate(f)?;
        }
        if let Some(h) = &self.history {
            h.validate()?;
        }
        check_attribution("site", self.attribution.as_deref())?;
        if let Some(c) = &self.currency {
            if crate::commerce::currency(c).is_none() {
                bail!("currency {c:?} must be a three-letter ISO 4217 code, e.g. USD");
            }
        }
        if let Some(h) = self.holdout_fraction {
            if !(0.0..=0.5).contains(&h) {
                bail!("holdout_fraction must be between 0 and 0.5");
            }
        }
        for slot in &self.slots {
            validate_slot(slot)?;
        }
        crate::stated::validate_site(self)?;
        Ok(())
    }
}

pub const GOALS: [&str; 2] = ["conversion", "revenue"];

/// One slot's rules, as the site file holds them. The MCP `create_slot` runs the same check on
/// the slot an agent describes, so a draft slot can never be one the site file would refuse.
pub fn validate_slot(slot: &SlotSpec) -> Result<()> {
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
    if !(0..=MAX_COLD_START).contains(&slot.cold_start_exposures) {
        bail!("slot {k:?}: cold_start_exposures must be between 0 and {MAX_COLD_START}");
    }
    check_attribution(&format!("slot {k:?}"), slot.attribution.as_deref())?;
    check_goal(&format!("slot {k:?}"), slot.goal.as_deref(), slot.revenue_cap)?;
    crate::stated::validate_slot(slot)?;
    crate::sorted::validate_slot(slot)?;
    crate::copy::validate_slot(slot)?;
    let baselines = slot.variants.iter().filter(|v| v.baseline).count();
    if baselines != 1 && !slot.is_sorted() {
        bail!("slot {k:?}: exactly one variant must be marked baseline (found {baselines})");
    }
    let mut keys = std::collections::HashSet::new();
    for v in &slot.variants {
        if !is_slug(&v.key) || !keys.insert(&v.key) {
            bail!("slot {k:?}: variant key {:?} is invalid or repeated", v.key);
        }
        match (v.baseline, &v.html) {
            (true, Some(_)) => bail!("slot {k:?}: baseline {:?} must not carry html; it is the page's own content", v.key),
            (false, None) if !slot.is_picker() && !slot.is_sorted() && !slot.is_copy() => bail!("slot {k:?}: variant {:?} needs html", v.key),
            _ => {}
        }
    }
    Ok(())
}

/// A slot's goal and revenue cap: the cap goes with `revenue` and only with it.
pub fn check_goal(what: &str, goal: Option<&str>, cap: Option<f64>) -> Result<()> {
    match (goal.unwrap_or("conversion"), cap) {
        (g, _) if !GOALS.contains(&g) => bail!("{what}: goal {g:?} is not one of {GOALS:?}"),
        ("revenue", Some(c)) if c.is_finite() && c > 0.0 && c <= crate::commerce::MAX_VALUE => Ok(()),
        ("revenue", _) => bail!("{what}: goal \"revenue\" needs revenue_cap, an order value > 0 that counts as a full success"),
        (_, Some(_)) => bail!("{what}: revenue_cap goes with goal \"revenue\""),
        _ => Ok(()),
    }
}

fn check_attribution(what: &str, name: Option<&str>) -> Result<()> {
    match name {
        Some(n) if crate::attribution::by_name(n).is_none() => {
            bail!("{what}: attribution {n:?} is not one of {:?}", crate::attribution::NAMES)
        }
        _ => Ok(()),
    }
}

pub async fn apply_path(pool: &PgPool, path: &std::path::Path) -> Result<()> {
    let text = std::fs::read_to_string(path)?;
    let file: SiteFile = serde_json::from_str(&text)?;
    apply(pool, &file).await
}

/// [`apply_path`], then publish the site's edge snapshot ([`crate::edge`]; nothing when off).
/// The file is applied even when the publish fails; the error says so, and the next worker pass
/// retries the publish.
pub async fn apply_path_edge(pool: &PgPool, path: &std::path::Path, edge: &crate::edge::Edge) -> Result<()> {
    let text = std::fs::read_to_string(path)?;
    let file: SiteFile = serde_json::from_str(&text)?;
    apply_edge(pool, &file, edge).await
}

pub async fn apply_edge(pool: &PgPool, file: &SiteFile, edge: &crate::edge::Edge) -> Result<()> {
    apply(pool, file).await?;
    crate::edge::publish(pool, edge, &file.site)
        .await
        .map_err(|e| anyhow::anyhow!("applied, but the edge snapshot was not published (the worker retries): {e:#}"))?;
    Ok(())
}

pub async fn apply(pool: &PgPool, file: &SiteFile) -> Result<()> {
    file.validate()?;
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO sites (id, origin, holdout_fraction, facts, privacy_mode, fields_allowed, attribution, currency)
         VALUES ($1, $2, COALESCE($3, 0.10), $4, COALESCE($5, 'consent_aware'), $6, COALESCE($7, 'last_touch'), COALESCE($8, 'USD'))
         ON CONFLICT (id) DO UPDATE SET origin = EXCLUDED.origin, holdout_fraction = COALESCE($3, sites.holdout_fraction),
                                        facts = COALESCE($4, sites.facts), privacy_mode = COALESCE($5, sites.privacy_mode),
                                        fields_allowed = COALESCE($6, sites.fields_allowed),
                                        attribution = COALESCE($7, sites.attribution),
                                        currency = COALESCE($8, sites.currency)",
    )
        .bind(&file.site)
        .bind(&file.origin)
        .bind(file.holdout_fraction)
        .bind(&file.facts)
        .bind(&file.privacy_mode)
        .bind(&file.fields_allowed)
        .bind(&file.attribution)
        .bind(file.currency.as_deref().and_then(crate::commerce::currency))
        .execute(&mut *tx)
        .await?;
    if let Some(h) = &file.history {
        let v = h.enabled.then(|| serde_json::to_value(h)).transpose()?;
        sqlx::query("UPDATE sites SET history = $2 WHERE id = $1").bind(&file.site).bind(v).execute(&mut *tx).await?;
    }
    if let Some(features) = &file.features {
        sqlx::query("DELETE FROM site_features WHERE site_id = $1").bind(&file.site).execute(&mut *tx).await?;
        for f in features {
            sqlx::query("INSERT INTO site_features (site_id, key, pii) VALUES ($1, $2, $3)")
                .bind(&file.site)
                .bind(&f.key)
                .bind(f.pii)
                .execute(&mut *tx)
                .await?;
        }
    }
    for slot in &file.slots {
        let slot_id: i64 = sqlx::query_scalar(
            "INSERT INTO slots (site_id, key, selector, metric, attribution_window_secs, decision_ttl_secs, kind, in_session, picker, cold_start_exposures, attribution,
                                goal, revenue_cap, sorted, copy)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
             ON CONFLICT (site_id, key) DO UPDATE SET selector = EXCLUDED.selector, metric = EXCLUDED.metric,
                 attribution_window_secs = EXCLUDED.attribution_window_secs, decision_ttl_secs = EXCLUDED.decision_ttl_secs,
                 kind = EXCLUDED.kind, in_session = EXCLUDED.in_session, picker = EXCLUDED.picker,
                 cold_start_exposures = EXCLUDED.cold_start_exposures, attribution = EXCLUDED.attribution,
                 goal = EXCLUDED.goal, revenue_cap = EXCLUDED.revenue_cap, sorted = EXCLUDED.sorted, copy = EXCLUDED.copy
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
        .bind(slot.cold_start_exposures)
        .bind(&slot.attribution)
        .bind(slot.goal.as_deref().unwrap_or("conversion"))
        .bind(slot.revenue_cap)
        .bind(crate::sorted::stored(slot))
        .bind(crate::copy::stored(slot))
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
        for (i, v) in slot.variants.iter().enumerate() {
            // A sorted slot's first item stands as its baseline: the head of the default order.
            let baseline = v.baseline || (slot.is_sorted() && i == 0);
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
            .bind(v.copy.as_ref().map(crate::copy::payload).or_else(|| v.html.clone()))
            .bind(baseline)
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
