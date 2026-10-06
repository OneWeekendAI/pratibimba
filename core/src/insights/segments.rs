//! Segment predicates: a conjunction over a closed allowlist of behaviour and situation
//! features. Anything else is refused, and a demographic name is refused by name (backend.md,
//! "No demographic inference"): a segment can say "returning night-time visitors", never
//! "women aged 25-34".
//!
//! ```json
//! {"is_night_local": true, "returning": true}
//! {"hour_local": [22, 2]}            // inclusive, wraps midnight
//! ```

use super::localtime::LocalTime;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Boolean features. The first three are the decision's own context (bitmask order).
pub const BOOL_FEATURES: [&str; 6] = ["returning", "mobile", "from_ad", "is_night_local", "is_weekend_local", "is_business_hours_local"];

/// Named so the refusal says why, not just "unknown".
pub const DEMOGRAPHIC: [&str; 14] = [
    "gender", "sex", "age", "age_band", "age_range", "birth_year", "income", "ethnicity", "race", "religion", "nationality",
    "sexual_orientation", "marital_status", "disability",
];

pub const CTX_RETURNING: u8 = 1;
pub const CTX_MOBILE: u8 = 2;
pub const CTX_FROM_AD: u8 = 4;

/// What a predicate is evaluated against: one exposure's context.
#[derive(Debug, Clone, Copy)]
pub struct Visit {
    pub ctx: u8,
    pub local: LocalTime,
}

impl Visit {
    pub fn feature(&self, name: &str) -> Option<bool> {
        Some(match name {
            "returning" => self.ctx & CTX_RETURNING != 0,
            "mobile" => self.ctx & CTX_MOBILE != 0,
            "from_ad" => self.ctx & CTX_FROM_AD != 0,
            "is_night_local" => self.local.is_night_local,
            "is_weekend_local" => self.local.is_weekend_local,
            "is_business_hours_local" => self.local.is_business_hours_local,
            _ => return None,
        })
    }
}

/// The context bitmask of a decision's stored features (`{"returning":1,"mobile":0,...}`).
pub fn ctx_bits(features: &Value) -> u8 {
    let on = |k: &str| features.get(k).and_then(Value::as_f64).is_some_and(|v| v > 0.5);
    (on("returning") as u8) * CTX_RETURNING | (on("mobile") as u8) * CTX_MOBILE | (on("from_ad") as u8) * CTX_FROM_AD
}

#[derive(Debug, Clone, PartialEq)]
pub struct Predicate {
    bools: BTreeMap<String, bool>,
    hours: Option<(u8, u8)>,
}

impl Predicate {
    pub fn parse(v: &Value) -> Result<Predicate, String> {
        let obj = v.as_object().ok_or("a predicate is a JSON object of feature conditions")?;
        if obj.is_empty() {
            return Err("a predicate needs at least one condition".into());
        }
        let mut p = Predicate { bools: BTreeMap::new(), hours: None };
        for (k, val) in obj {
            if DEMOGRAPHIC.contains(&k.as_str()) {
                return Err(format!("{k:?} is a demographic attribute; segments use behaviour and situation only"));
            }
            if k == "hour_local" {
                let a = val.as_array().filter(|a| a.len() == 2).ok_or("hour_local is [from, to], both 0-23")?;
                let h = |x: &Value| x.as_u64().filter(|h| *h < 24).map(|h| h as u8);
                match (h(&a[0]), h(&a[1])) {
                    (Some(f), Some(t)) => p.hours = Some((f, t)),
                    _ => return Err("hour_local is [from, to], both 0-23".into()),
                }
            } else if BOOL_FEATURES.contains(&k.as_str()) {
                p.bools.insert(k.clone(), val.as_bool().ok_or_else(|| format!("{k} takes true or false"))?);
            } else {
                return Err(format!("unknown feature {k:?}; allowed: {}, hour_local", BOOL_FEATURES.join(", ")));
            }
        }
        Ok(p)
    }

    pub fn from_bools(conds: &[(&str, bool)]) -> Predicate {
        Predicate { bools: conds.iter().map(|(k, v)| (k.to_string(), *v)).collect(), hours: None }
    }

    pub fn matches(&self, v: &Visit) -> bool {
        if let Some((f, t)) = self.hours {
            let h = v.local.hour_local;
            let inside = if f <= t { h >= f && h <= t } else { h >= f || h <= t };
            if !inside {
                return false;
            }
        }
        self.bools.iter().all(|(k, want)| v.feature(k) == Some(*want))
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        for (k, v) in &self.bools {
            m.insert(k.clone(), Value::Bool(*v));
        }
        if let Some((f, t)) = self.hours {
            m.insert("hour_local".into(), serde_json::json!([f, t]));
        }
        Value::Object(m)
    }

    /// Canonical text: keys sorted, so `{a,b}` and `{b,a}` are one segment.
    pub fn key(&self) -> String {
        self.to_json().to_string()
    }

    /// True when the predicate reads the visitor's local clock, so it cannot be decided for a
    /// visitor whose clock is unknown.
    pub fn uses_local_time(&self) -> bool {
        self.hours.is_some() || self.bools.keys().any(|k| k.ends_with("_local"))
    }

    pub fn len(&self) -> usize {
        self.bools.len() + self.hours.is_some() as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A short English name, deterministic: "Night-time, returning visitors".
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for (k, v) in &self.bools {
            let s = match (k.as_str(), v) {
                ("returning", true) => "returning",
                ("returning", false) => "first-time",
                ("mobile", true) => "mobile",
                ("mobile", false) => "desktop",
                ("from_ad", true) => "ad-referred",
                ("from_ad", false) => "non-ad",
                ("is_night_local", true) => "night-time",
                ("is_night_local", false) => "daytime",
                ("is_weekend_local", true) => "weekend",
                ("is_weekend_local", false) => "weekday",
                ("is_business_hours_local", true) => "business-hours",
                ("is_business_hours_local", false) => "out-of-hours",
                _ => continue,
            };
            parts.push(s.to_string());
        }
        if let Some((f, t)) = self.hours {
            parts.push(format!("{f:02}:00-{:02}:59 local", t));
        }
        let mut s = parts.join(", ");
        if let Some(c) = s.get(0..1) {
            s = c.to_uppercase() + &s[1..];
        }
        s + " visitors"
    }
}
