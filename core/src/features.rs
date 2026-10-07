//! Declared features (backend.md, "PII features are opt-in and tagged"). A site file may declare
//! boolean features the client sends in `ctx.features`, each tagged `pii: true` or `pii: false`
//! (no default: the owner says which). They join the decision as extension dimensions,
//! `feature:<key>`, beside the stated tastes and the local clock, and the model learns them.
//!
//! A PII feature (one derived from an email domain or a phone country code, say) is used only
//! on an identified request: `full` mode, or `consent_aware` with consent. In `cookieless`
//! mode, and in `consent_aware` without consent, it is dropped before the decision is made,
//! so it is neither scored nor stored. An undeclared name is refused, as every other unknown
//! context field is.

use crate::http::ApiError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::{BTreeMap, BTreeSet};

/// The model key and decision feature name prefix of a declared feature.
pub const PREFIX: &str = "feature:";
/// The policy name of a decision whose only extension dimensions are declared features and,
/// maybe, the local clock.
pub const POLICY: &str = "lints_features";
/// Declared features a site may have.
pub const MAX_FEATURES: usize = 16;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureSpec {
    pub key: String,
    /// Whether the feature can identify or profile a person. Required.
    pub pii: bool,
}

pub fn validate(features: &[FeatureSpec]) -> anyhow::Result<()> {
    if features.len() > MAX_FEATURES {
        anyhow::bail!("{} declared features; the limit is {MAX_FEATURES}", features.len());
    }
    let mut seen = BTreeSet::new();
    for f in features {
        if !crate::site_file::is_slug(&f.key) {
            anyhow::bail!("feature {:?}: key must be a lowercase slug", f.key);
        }
        if crate::stated::is_demographic(&f.key) {
            anyhow::bail!("feature {:?}: names a demographic attribute, which Pratibimba never infers", f.key);
        }
        if !seen.insert(&f.key) {
            anyhow::bail!("feature {:?} is declared twice", f.key);
        }
    }
    Ok(())
}

/// The site's declared features: key -> pii.
pub async fn declared(pool: &PgPool, site: &str) -> Result<BTreeMap<String, bool>, sqlx::Error> {
    let rows: Vec<(String, bool)> = sqlx::query_as("SELECT key, pii FROM site_features WHERE site_id = $1").bind(site).fetch_all(pool).await?;
    Ok(rows.into_iter().collect())
}

/// The declared features this request may use, by model key (`feature:<key>` -> 0 or 1): every
/// declared feature, minus the PII ones when the request is not identified. Refuses a name
/// the site did not declare.
pub fn values(declared: &BTreeMap<String, bool>, sent: &BTreeMap<String, bool>, identified: bool) -> Result<BTreeMap<String, f64>, ApiError> {
    if let Some(bad) = sent.keys().find(|k| !declared.contains_key(*k)) {
        return Err(ApiError::BadRequest(format!("ctx.features {bad:?} is not a feature this site declares")));
    }
    Ok(declared
        .iter()
        .filter(|(_, pii)| !**pii || identified)
        .map(|(k, _)| (format!("{PREFIX}{k}"), if sent.get(k) == Some(&true) { 1.0 } else { 0.0 }))
        .collect())
}
