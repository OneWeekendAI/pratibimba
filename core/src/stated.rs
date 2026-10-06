//! Stated preference: the `picker` slot kind and the `stated` feature group.
//!
//! A picker slot shows owner-defined options; a visitor picks 1-3 of them or skips. A pick is
//! a `pick` event carrying the option keys. The picks become the `stated` features of every
//! later decision for that subject on the site: `stated:<option key>` = 1. Skipping sends a
//! `skip` event and sets nothing.
//!
//! Scope follows the subject. Without cookies the subject lives in the tab's sessionStorage,
//! so stated preferences are session-scoped; with the site's cookie on, the subject persists
//! and so do they. Nothing else is stored.
//!
//! Option keys are owner-defined slugs, and a key that names a demographic attribute is
//! refused when the site file is applied, so `stated` can never carry one.
//!
//! The model. A site with picker options decides with [`StatedModel`]: per arm, the same
//! Bayesian logistic regression as [`crate::bandit`] over the base features extended by one
//! dimension per option key. An arm is created from the base arm's posterior the first time it
//! is needed (every observation it had was taken with all stated features at zero, so the base
//! posterior IS the posterior over those dimensions, and the new ones sit at the prior). A key
//! added later extends every arm the same way. The base model keeps learning beside it, so
//! dropping every picker loses nothing.

use crate::bandit::{self, LinTs, Vector, sigmoid};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, StandardNormal};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const PREFIX: &str = "stated:";
/// The policy name stored on a decision scored with the stated model.
pub const POLICY: &str = "lints_stated";
/// Distinct option keys a site may define across all its pickers.
pub const MAX_KEYS: usize = 16;
pub const MAX_PICKS: usize = 3;

/// Words an option key may not contain: `stated` is for tastes, never for who someone is.
pub const DEMOGRAPHIC: &[&str] = &[
    "age", "aged", "gender", "sex", "male", "female", "man", "woman", "men", "women", "boy", "girl", "race", "racial",
    "ethnic", "ethnicity", "religion", "religious", "caste", "nationality", "income", "salary", "married", "marital",
    "pregnant", "pregnancy", "disability", "disabled", "orientation", "gay", "lesbian", "straight", "trans", "teen",
    "senior", "elderly", "young", "old", "child", "kid", "kids", "mother", "father", "parent",
];

/// True when an option key names a demographic attribute (any `-`/`_`-separated word).
pub fn is_demographic(key: &str) -> bool {
    key.split(['-', '_']).any(|w| DEMOGRAPHIC.contains(&w))
}

/// The model key of an extension dimension that is not a stated taste. Option keys are slugs, so
/// they never contain a `:` and cannot collide with these.
pub const TIME_PREFIX: &str = "time:";
/// The time-of-day features (see [`crate::insights::localtime`]), by decision feature name.
///
/// Append-only: `is_weekend_local` and `is_business_hours_local` joined after the first three.
/// A model saved with only the first three keeps what they learned; the two new dimensions
/// join at the prior on the first decision that carries them (see [`StatedModel`]).
pub const TIME_FEATURES: [&str; 5] = ["sin_hour", "cos_hour", "is_night_local", "is_weekend_local", "is_business_hours_local"];
/// The policy name of a decision scored with the extension for time features alone.
pub const TIME_POLICY: &str = "lints_time";

/// True for the two policies scored by the extension model.
pub fn is_extended(policy: &str) -> bool {
    policy == POLICY || policy == TIME_POLICY
}

/// The model keys of the time features, in the order they are appended to a model.
pub fn time_keys() -> Vec<String> {
    TIME_FEATURES.iter().map(|n| format!("{TIME_PREFIX}{n}")).collect()
}

/// A model key's decision feature name: `time:sin_hour` is `sin_hour`, an option key `k` is
/// `stated:k`.
pub fn feature_name(key: &str) -> String {
    match key.strip_prefix(TIME_PREFIX) {
        Some(n) => n.to_string(),
        None => format!("{PREFIX}{key}"),
    }
}

/// Every extension dimension in stored named features, keyed by model key: the stated tastes
/// and, when the decision knew the visitor's local hour, the time features.
pub fn ext_from_features(features: &Value) -> BTreeMap<String, f64> {
    let mut out = from_features(features);
    for n in TIME_FEATURES {
        if let Some(v) = features.get(n).and_then(Value::as_f64) {
            out.insert(format!("{TIME_PREFIX}{n}"), v);
        }
    }
    out
}

/// The time features of a local clock, by model key.
pub fn time_values(t: &crate::insights::localtime::LocalTime) -> BTreeMap<String, f64> {
    let b = |v: bool| if v { 1.0 } else { 0.0 };
    let v = [t.sin_hour, t.cos_hour, b(t.is_night_local), b(t.is_weekend_local), b(t.is_business_hours_local)];
    time_keys().into_iter().zip(v).collect()
}

/// The stated part of stored named features: option key -> value.
pub fn from_features(features: &Value) -> BTreeMap<String, f64> {
    features
        .as_object()
        .map(|m| m.iter().filter_map(|(k, v)| Some((k.strip_prefix(PREFIX)?.to_string(), v.as_f64()?))).collect())
        .unwrap_or_default()
}

/// Add `stated:<key>` for every site key to base named features (0 when not stated).
pub fn with_stated(mut features: Value, site_keys: &[String], stated: &BTreeMap<String, f64>) -> Value {
    if let Some(m) = features.as_object_mut() {
        for k in site_keys {
            m.insert(feature_name(k), json!(stated.get(k).copied().unwrap_or(0.0)));
        }
    }
    features
}

/// A per-arm posterior of any dimension. Same algorithm as [`bandit::Arm`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DynArm {
    pub mean: Vec<f64>,
    pub cov: Vec<Vec<f64>>,
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

impl DynArm {
    fn from_base(a: &bandit::Arm, dim: usize) -> Self {
        let mut arm = DynArm { mean: a.mean.to_vec(), cov: a.cov.iter().map(|r| r.to_vec()).collect() };
        arm.extend_to(dim);
        arm
    }

    /// New dimensions at the prior N(0, 1), uncorrelated with the old ones.
    fn extend_to(&mut self, dim: usize) {
        let old = self.mean.len();
        self.mean.resize(dim, 0.0);
        for row in &mut self.cov {
            row.resize(dim, 0.0);
        }
        for i in old..dim {
            let mut row = vec![0.0; dim];
            row[i] = 1.0;
            self.cov.push(row);
        }
    }

    /// The twin of [`bandit::Arm::flip_miss`]: the mean moves by `cov x`.
    pub fn flip_miss(&mut self, x: &[f64]) {
        let sx: Vec<f64> = self.cov.iter().map(|r| dot(r, x)).collect();
        for (m, s) in self.mean.iter_mut().zip(&sx) {
            *m += s;
        }
    }

    pub fn update(&mut self, x: &[f64], y: bool) {
        let n = x.len();
        let y = if y { 1.0 } else { 0.0 };
        let sx: Vec<f64> = self.cov.iter().map(|r| dot(r, x)).collect();
        let s = dot(x, &sx);
        let a = dot(&self.mean, x);
        let mut t = 0.0;
        for _ in 0..50 {
            let p = sigmoid(a + t * s);
            let step = (t - y + p) / (1.0 + p * (1.0 - p) * s);
            t = (t - step).clamp(-1.0, 1.0);
            if step.abs() < 1e-12 {
                break;
            }
        }
        for (m, s) in self.mean.iter_mut().zip(&sx) {
            *m += t * s;
        }
        let p = sigmoid(dot(&self.mean, x));
        let c = p * (1.0 - p);
        let denom = 1.0 + c * s;
        for i in 0..n {
            for j in 0..n {
                self.cov[i][j] -= c * sx[i] * sx[j] / denom;
            }
        }
    }

    fn cholesky(&self) -> Vec<Vec<f64>> {
        let n = self.mean.len();
        let mut l = vec![vec![0.0; n]; n];
        for i in 0..n {
            for j in 0..=i {
                let mut sum = self.cov[i][j];
                for k in 0..j {
                    sum -= l[i][k] * l[j][k];
                }
                l[i][j] = if i == j { sum.max(1e-12).sqrt() } else { sum / l[j][j] };
            }
        }
        l
    }

    pub fn sample_score(&self, x: &[f64], rng: &mut ChaCha8Rng) -> f64 {
        let n = self.mean.len();
        let l = self.cholesky();
        let z: Vec<f64> = (0..n).map(|_| StandardNormal.sample(rng)).collect();
        let mut w = self.mean.clone();
        for i in 0..n {
            w[i] += l[i][..=i].iter().zip(&z).map(|(a, b)| a * b).sum::<f64>();
        }
        sigmoid(dot(&w, x))
    }

    pub fn mean_score(&self, x: &[f64]) -> f64 {
        sigmoid(dot(&self.mean, x))
    }
}

/// The extension of one slot's model, persisted inside [`LinTs`]: the stated tastes and the
/// time-of-day features, as extra named dimensions after the base four.
///
/// **Versioning.** The layout is self-describing: `keys` names every dimension after the base,
/// in vector order, and is only ever appended to. A model saved before the time features
/// existed has keys like `["garden", "kitchen"]` and arms of dimension 6; the first decision
/// that carries time features appends `time:sin_hour`, `time:cos_hour`, `time:is_night_local`
/// and extends every arm at the prior, so what the old dimensions learned is kept exactly and
/// a model with no time data scores as it did. An arm whose shape does not match its keys
/// (a hand-edited or corrupt state) is reset to the base posterior rather than read misaligned.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StatedModel {
    /// Model keys in vector order (after the base features). Only ever appended to.
    pub keys: Vec<String>,
    pub arms: BTreeMap<i64, DynArm>,
}

impl StatedModel {
    pub fn dim(&self) -> usize {
        bandit::DIM + self.keys.len()
    }

    /// Append any of `keys` not yet known, in sorted order, extending every arm.
    pub fn ensure_keys<'a>(&mut self, keys: impl IntoIterator<Item = &'a String>) {
        let known: BTreeSet<&String> = self.keys.iter().collect();
        let new: BTreeSet<String> = keys.into_iter().filter(|k| !known.contains(k)).cloned().collect();
        if new.is_empty() {
            return;
        }
        self.keys.extend(new);
        let dim = self.dim();
        for arm in self.arms.values_mut() {
            arm.extend_to(dim);
        }
    }

    /// The arm's posterior, or the base arm's posterior extended to this model's keys. A stored
    /// arm whose shape does not fit the keys is reset to that, never read misaligned.
    pub fn arm(&self, base: &LinTs, id: i64) -> DynArm {
        let dim = self.dim();
        match self.arms.get(&id) {
            Some(a) if a.mean.len() == dim && a.cov.len() == dim && a.cov.iter().all(|r| r.len() == dim) => a.clone(),
            Some(_) => {
                tracing::warn!(arm = id, dim, "a stored extension arm has the wrong shape; reset to the base posterior");
                DynArm::from_base(&base.arm(id), dim)
            }
            None => DynArm::from_base(&base.arm(id), dim),
        }
    }

    pub fn vector(&self, x: &Vector, stated: &BTreeMap<String, f64>) -> Vec<f64> {
        let mut v = x.to_vec();
        v.extend(self.keys.iter().map(|k| stated.get(k).copied().unwrap_or(0.0)));
        v
    }

    /// Feature names in vector order: the base names, then `stated:<key>` and the time names.
    pub fn names(&self) -> Vec<String> {
        bandit::FEATURE_NAMES.iter().map(|s| s.to_string()).chain(self.keys.iter().map(|k| feature_name(k))).collect()
    }
}

/// The stated model of `model`, extended to `keys` (a fresh one when the slot has none yet).
pub fn model_for(model: &LinTs, keys: &[String]) -> StatedModel {
    let mut m = model.stated.clone().unwrap_or_default();
    m.ensure_keys(keys);
    m
}

/// Thompson draw for every arm, ascending id order, one seeded RNG: the stated twin of
/// [`LinTs::choose`]. `keys` are the site's option keys at decision time.
pub fn choose(model: &LinTs, keys: &[String], arm_ids: &[i64], x: &Vector, stated: &BTreeMap<String, f64>, seed: u64) -> (i64, Vec<(i64, f64)>) {
    let m = model_for(model, keys);
    let v = m.vector(x, stated);
    let mut ids = arm_ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let scores: Vec<(i64, f64)> = ids.iter().map(|&id| (id, m.arm(model, id).sample_score(&v, &mut rng))).collect();
    let mut best = scores[0];
    for &(id, s) in &scores[1..] {
        if s > best.1 {
            best = (id, s);
        }
    }
    (best.0, scores)
}

/// Absorb one settled observation. Extension features present (stated or time): the extension
/// arm learns first (it is seeded from the base arm BEFORE the base absorbs this observation),
/// then the base.
pub fn learn(model: &mut LinTs, arm: i64, x: &Vector, features: &Value, converted: bool) {
    let stated = ext_from_features(features);
    if !stated.is_empty() {
        let mut m = model_for(model, &stated.keys().cloned().collect::<Vec<_>>());
        let mut a = m.arm(model, arm);
        a.update(&m.vector(x, &stated), converted);
        m.arms.insert(arm, a);
        model.stated = Some(m);
    }
    model.update(arm, x, converted);
}

/// A settled miss turned out to be a conversion (a late platform webhook): flip it in the base
/// arm and, when the decision carried extension features, in the extension arm. The
/// observation count does not change; it is the same observation, corrected.
pub fn correct_miss(model: &mut LinTs, arm: i64, x: &Vector, features: &Value) {
    let ext = ext_from_features(features);
    if !ext.is_empty() {
        let mut m = model_for(model, &ext.keys().cloned().collect::<Vec<_>>());
        let mut a = m.arm(model, arm);
        a.flip_miss(&m.vector(x, &ext));
        m.arms.insert(arm, a);
        model.stated = Some(m);
    }
    model.arms.entry(arm).or_default().flip_miss(x);
}

/// Posterior-mean weights per arm, by feature name: what the on-device scorer ships with.
pub fn weights(model: &LinTs, keys: &[String], arm: i64) -> Map<String, Value> {
    let m = model_for(model, keys);
    let a = m.arm(model, arm);
    m.names().into_iter().zip(a.mean).map(|(n, w)| (n, json!(w))).collect()
}

/// Posterior-mean conversion rate for stored named features (base, stated or time).
pub fn mean_score(model: &LinTs, arm: i64, x: &Vector, features: &Value) -> f64 {
    let stated = ext_from_features(features);
    let keys: Vec<String> = stated.keys().cloned().collect();
    let m = model_for(model, &keys);
    m.arm(model, arm).mean_score(&m.vector(x, &stated))
}

// --- the picker slot kind ----------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PickerSpec {
    #[serde(default = "default_max_picks")]
    pub max_picks: u8,
    pub options: Vec<OptionSpec>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OptionSpec {
    pub key: String,
    pub label: String,
}

fn default_max_picks() -> u8 {
    MAX_PICKS as u8
}

/// Per-slot rules: a picker has options and exactly the variants `none` (baseline) and
/// `picker`; it is never re-scored in session; a swap slot carries no options.
pub fn validate_slot(slot: &crate::site_file::SlotSpec) -> anyhow::Result<()> {
    use anyhow::bail;
    let k = &slot.key;
    match slot.kind.as_deref() {
        None | Some("swap") => {
            if slot.picker.is_some() {
                bail!("slot {k:?}: only a kind \"picker\" slot takes picker options");
            }
            return Ok(());
        }
        Some("picker") => {}
        Some(other) => bail!("slot {k:?}: kind {other:?} must be \"swap\" or \"picker\""),
    }
    if slot.in_session {
        bail!("slot {k:?}: a picker cannot be in_session; the visitor is using it");
    }
    let Some(p) = &slot.picker else { bail!("slot {k:?}: a picker needs picker.options") };
    if !(1..=MAX_PICKS as u8).contains(&p.max_picks) {
        bail!("slot {k:?}: picker.max_picks must be 1-{MAX_PICKS}");
    }
    if p.options.len() < 2 || p.options.len() > MAX_KEYS {
        bail!("slot {k:?}: a picker needs 2-{MAX_KEYS} options");
    }
    let mut seen = BTreeSet::new();
    for o in &p.options {
        if !crate::site_file::is_slug(&o.key) || !seen.insert(&o.key) {
            bail!("slot {k:?}: option key {:?} is invalid or repeated", o.key);
        }
        if is_demographic(&o.key) {
            bail!("slot {k:?}: option {:?} names a demographic attribute; stated preferences are tastes, not who someone is", o.key);
        }
        if o.label.trim().is_empty() || o.label.len() > 80 {
            bail!("slot {k:?}: option {:?} needs a label of 1-80 characters", o.key);
        }
    }
    let mut keys: Vec<(&str, bool)> = slot.variants.iter().map(|v| (v.key.as_str(), v.baseline)).collect();
    keys.sort();
    if keys != [("none", true), ("picker", false)] {
        bail!("slot {k:?}: a picker has exactly two variants, \"none\" (the baseline) and \"picker\"");
    }
    Ok(())
}

/// Site-wide: at most [`MAX_KEYS`] distinct option keys across every picker.
pub fn validate_site(file: &crate::site_file::SiteFile) -> anyhow::Result<()> {
    let keys: BTreeSet<&String> = file.slots.iter().filter_map(|s| s.picker.as_ref()).flat_map(|p| p.options.iter().map(|o| &o.key)).collect();
    if keys.len() > MAX_KEYS {
        anyhow::bail!("site {:?}: {} distinct picker options; the limit is {MAX_KEYS}", file.site, keys.len());
    }
    Ok(())
}

/// The site's option keys, sorted: the `stated` feature group's names.
pub async fn site_keys(pool: &sqlx::PgPool, site: &str) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT DISTINCT o->>'key' FROM slots s, jsonb_array_elements(s.picker->'options') o
          WHERE s.site_id = $1 AND s.kind = 'picker' ORDER BY 1",
    )
    .bind(site)
    .fetch_all(pool)
    .await
}

/// The subject's stated preferences: the latest pick on each picker slot, plus what the
/// client sent in `ctx.stated`. Every key must be one of the site's options.
pub async fn resolve(pool: &sqlx::PgPool, site: &str, subject: &str, site_keys: &[String], from_ctx: &[String]) -> Result<BTreeMap<String, f64>, crate::http::ApiError> {
    if let Some(bad) = from_ctx.iter().find(|k| !site_keys.contains(k)) {
        return Err(crate::http::ApiError::BadRequest(format!("ctx.stated {bad:?} is not one of this site's picker options")));
    }
    let mut out: BTreeMap<String, f64> = from_ctx.iter().map(|k| (k.clone(), 1.0)).collect();
    if site_keys.is_empty() {
        return Ok(out);
    }
    let picks: Vec<Vec<String>> = sqlx::query_scalar(
        "SELECT DISTINCT ON (slot_id) picks FROM events
          WHERE site_id = $1 AND subject = $2 AND picks IS NOT NULL
          ORDER BY slot_id, received_at DESC",
    )
    .bind(site)
    .bind(subject)
    .fetch_all(pool)
    .await?;
    for k in picks.into_iter().flatten().filter(|k| site_keys.contains(k)) {
        out.insert(k, 1.0);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const X: Vector = [1.0, 0.0, 0.0, 0.0];

    #[test]
    fn demographic_words_are_refused() {
        assert!(is_demographic("gender"));
        assert!(is_demographic("age_25_34"));
        assert!(is_demographic("for-women"));
        assert!(!is_demographic("kitchen"));
        assert!(!is_demographic("garden"));
        assert!(!is_demographic("manga"), "a word containing 'man' is not the word 'man'");
    }

    #[test]
    fn a_fresh_stated_arm_is_the_base_posterior_extended() {
        let mut base = LinTs::default();
        for i in 0..30 {
            base.update(1, &X, i % 3 == 0);
        }
        let keys = vec!["garden".to_string(), "kitchen".to_string()];
        let m = model_for(&base, &keys);
        let a = m.arm(&base, 1);
        assert_eq!(&a.mean[..4], &base.arm(1).mean[..]);
        assert_eq!(&a.mean[4..], &[0.0, 0.0]);
        assert_eq!(a.cov[4][4], 1.0);
        assert_eq!(a.cov[0][4], 0.0);
        // With every stated feature at zero, the stated model scores exactly as the base does.
        let zero = BTreeMap::new();
        assert!((a.mean_score(&m.vector(&X, &zero)) - sigmoid(base.arm(1).mean[0])).abs() < 1e-12);
    }

    #[test]
    fn stated_learning_separates_tastes_the_base_cannot() {
        let mut model = LinTs::default();
        let feat = |k: f64, g: f64| json!({ "bias": 1, "returning": 0, "mobile": 0, "from_ad": 0, "stated:kitchen": k, "stated:garden": g });
        for _ in 0..200 {
            // Arm 1 converts for kitchen people only.
            learn(&mut model, 1, &X, &feat(1.0, 0.0), true);
            learn(&mut model, 1, &X, &feat(0.0, 1.0), false);
        }
        let keys = vec!["garden".to_string(), "kitchen".to_string()];
        let k = mean_score(&model, 1, &X, &feat(1.0, 0.0));
        let g = mean_score(&model, 1, &X, &feat(0.0, 1.0));
        assert!(k > 0.8 && g < 0.2, "kitchen {k:.2} garden {g:.2}");
        assert!(weights(&model, &keys, 1)["stated:kitchen"].as_f64().unwrap() > 1.0);
        // The base model saw the same data and can only say "about half".
        let b = sigmoid(model.arm(1).mean[0]);
        assert!((0.3..0.7).contains(&b), "base {b:.2}");
    }

    #[test]
    fn a_key_added_later_extends_existing_arms_at_the_prior() {
        let mut model = LinTs::default();
        learn(&mut model, 1, &X, &json!({ "bias": 1, "returning": 0, "mobile": 0, "from_ad": 0, "stated:a": 1 }), true);
        let before = model.stated.clone().unwrap().arms[&1].clone();
        let m = model_for(&model, &["a".to_string(), "b".to_string()]);
        let after = &m.arms[&1];
        assert_eq!(&after.mean[..5], &before.mean[..]);
        assert_eq!(after.mean[5], 0.0);
        assert_eq!(after.cov[5][5], 1.0);
    }

    #[test]
    fn choose_is_replayable() {
        let mut model = LinTs::default();
        learn(&mut model, 2, &X, &json!({ "bias": 1, "returning": 0, "mobile": 0, "from_ad": 0, "stated:a": 1 }), true);
        let keys = vec!["a".to_string()];
        let s: BTreeMap<String, f64> = [("a".to_string(), 1.0)].into();
        let round: LinTs = serde_json::from_value(serde_json::to_value(&model).unwrap()).unwrap();
        assert_eq!(choose(&model, &keys, &[1, 2, 3], &X, &s, 99), choose(&round, &keys, &[3, 2, 1], &X, &s, 99));
    }

    /// A model saved before the time features existed: base arms of 4, stated arms of 4 + 1.
    /// This JSON is the exact shape the previous release wrote.
    fn saved_v1() -> LinTs {
        let mut m = LinTs::default();
        for i in 0..40 {
            learn(&mut m, 1, &X, &json!({ "bias": 1, "returning": 0, "mobile": 0, "from_ad": 0, "stated:a": (i % 2) as f64 }), i % 3 == 0);
        }
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["stated"]["keys"], json!(["a"]));
        assert_eq!(v["stated"]["arms"]["1"]["mean"].as_array().unwrap().len(), 5);
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn time_features_extend_a_saved_model_and_keep_what_it_learned() {
        let old = saved_v1();
        let before = old.stated.clone().unwrap().arms[&1].clone();
        let mut keys = vec!["a".to_string()];
        keys.extend(time_keys());
        let m = model_for(&old, &keys);
        assert_eq!(
            m.keys,
            ["a", "time:cos_hour", "time:is_business_hours_local", "time:is_night_local", "time:is_weekend_local", "time:sin_hour"],
            "appended after the old keys"
        );
        let a = m.arm(&old, 1);
        assert_eq!(&a.mean[..5], &before.mean[..], "the old dimensions keep their posterior");
        assert_eq!(&a.mean[5..], &[0.0; 5]);
        for i in 5..10 {
            assert_eq!(a.cov[i][i], 1.0);
            assert_eq!(a.cov[0][i], 0.0);
        }
        let w = weights(&old, &keys, 1);
        assert_eq!(w["stated:a"], json!(before.mean[4]));
        assert_eq!(w["sin_hour"], json!(0.0));
        assert_eq!(w["is_night_local"], json!(0.0));
        // With no time data yet, the extended model scores exactly as the saved one did.
        let f = json!({ "bias": 1, "returning": 0, "mobile": 0, "from_ad": 0, "stated:a": 1 });
        let mut ft = f.clone();
        ft["sin_hour"] = json!(0.5);
        ft["cos_hour"] = json!(-0.8);
        ft["is_night_local"] = json!(1);
        assert!((mean_score(&old, 1, &X, &f) - mean_score(&old, 1, &X, &ft)).abs() < 1e-12);
    }

    #[test]
    fn a_misshapen_stored_arm_resets_to_the_base_posterior() {
        let mut old = saved_v1();
        let st = old.stated.as_mut().unwrap();
        st.arms.get_mut(&1).unwrap().mean.truncate(4);
        let m = model_for(&old, &["a".to_string()]);
        let a = m.arm(&old, 1);
        assert_eq!(a.mean.len(), 5);
        assert_eq!(&a.mean[..4], &old.arm(1).mean[..], "reset from the base arm, not read misaligned");
        assert_eq!(a.mean[4], 0.0);
    }

    #[test]
    fn time_learning_separates_night_from_day() {
        let mut model = LinTs::default();
        let feat = |night: f64| json!({ "bias": 1, "returning": 0, "mobile": 0, "from_ad": 0, "sin_hour": 0.0, "cos_hour": 0.0, "is_night_local": night });
        for _ in 0..200 {
            learn(&mut model, 1, &X, &feat(1.0), true);
            learn(&mut model, 1, &X, &feat(0.0), false);
        }
        let night = mean_score(&model, 1, &X, &feat(1.0));
        let day = mean_score(&model, 1, &X, &feat(0.0));
        assert!(night > 0.8 && day < 0.2, "night {night:.2} day {day:.2}");
        assert!(ext_from_features(&feat(1.0)).contains_key("time:is_night_local"));
        assert_eq!(feature_name("time:cos_hour"), "cos_hour");
        assert_eq!(feature_name("garden"), "stated:garden");
    }

    /// A model saved by the release before weekend and business hours joined: three time keys,
    /// arms of dimension 4 + 3. The JSON is the exact shape that release wrote.
    fn saved_v2() -> LinTs {
        let mut m = LinTs::default();
        for i in 0..60 {
            let night = (i % 2) as f64;
            learn(&mut m, 1, &X, &json!({ "bias": 1, "returning": 0, "mobile": 0, "from_ad": 0,
                "sin_hour": 0.5, "cos_hour": -0.5, "is_night_local": night }), i % 2 == 0);
        }
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["stated"]["keys"], json!(["time:cos_hour", "time:is_night_local", "time:sin_hour"]));
        assert_eq!(v["stated"]["arms"]["1"]["mean"].as_array().unwrap().len(), 7);
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn weekend_and_business_hours_extend_a_three_time_feature_model() {
        let old = saved_v2();
        let before = old.stated.clone().unwrap().arms[&1].clone();
        let m = model_for(&old, &time_keys());
        assert_eq!(
            m.keys,
            ["time:cos_hour", "time:is_night_local", "time:sin_hour", "time:is_business_hours_local", "time:is_weekend_local"],
            "the two new keys are appended; the old three keep their places"
        );
        let a = m.arm(&old, 1);
        assert_eq!(&a.mean[..7], &before.mean[..], "the old dimensions keep their posterior");
        assert_eq!(&a.cov[3][..7], &before.cov[3][..], "and their covariance");
        assert_eq!(&a.mean[7..], &[0.0, 0.0]);
        for i in 7..9 {
            assert_eq!(a.cov[i][i], 1.0);
            assert_eq!(a.cov[6][i], 0.0);
        }
        let w = weights(&old, &time_keys(), 1);
        assert_eq!(w["is_night_local"], json!(before.mean[5]));
        assert_eq!(w["is_weekend_local"], json!(0.0));
        // Before any weekend data, the extended model scores exactly as the saved one did.
        let f = json!({ "bias": 1, "returning": 0, "mobile": 0, "from_ad": 0, "sin_hour": 0.5, "cos_hour": -0.5, "is_night_local": 1 });
        let mut fw = f.clone();
        fw["is_weekend_local"] = json!(1);
        fw["is_business_hours_local"] = json!(1);
        assert!((mean_score(&old, 1, &X, &f) - mean_score(&old, 1, &X, &fw)).abs() < 1e-12);
    }

    #[test]
    fn time_learning_separates_weekend_from_weekday() {
        let mut model = LinTs::default();
        let feat = |weekend: f64| json!({ "bias": 1, "returning": 0, "mobile": 0, "from_ad": 0, "sin_hour": 0.0, "cos_hour": 1.0,
            "is_night_local": 0, "is_weekend_local": weekend, "is_business_hours_local": 1.0 - weekend });
        for _ in 0..200 {
            learn(&mut model, 1, &X, &feat(1.0), true);
            learn(&mut model, 1, &X, &feat(0.0), false);
        }
        let weekend = mean_score(&model, 1, &X, &feat(1.0));
        let weekday = mean_score(&model, 1, &X, &feat(0.0));
        assert!(weekend > 0.8 && weekday < 0.2, "weekend {weekend:.2} weekday {weekday:.2}");
    }

    #[test]
    fn a_corrected_miss_moves_the_extension_arm_too() {
        let mut model = LinTs::default();
        let f = json!({ "bias": 1, "returning": 0, "mobile": 0, "from_ad": 0, "sin_hour": 0.5, "cos_hour": 0.8, "is_night_local": 1 });
        learn(&mut model, 1, &X, &f, false);
        let before = model.stated.clone().unwrap();
        let v = before.vector(&X, &ext_from_features(&f));
        let a = &before.arms[&1];
        correct_miss(&mut model, 1, &X, &f);
        let after = &model.stated.as_ref().unwrap().arms[&1];
        for i in 0..v.len() {
            let shift: f64 = a.cov[i].iter().zip(&v).map(|(c, x)| c * x).sum();
            assert!((after.mean[i] - a.mean[i] - shift).abs() < 1e-12, "dim {i}");
        }
        assert_eq!(after.cov, a.cov);
    }
}
