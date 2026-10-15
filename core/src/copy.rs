//! F4: the `copy` slot kind. A copy slot's variants are named plain-text fields with declared
//! maximum lengths, not HTML (backend.md, "Copy slots"). The payload is the field map as a JSON
//! object; a decision returns it as `copy` and a `null` payload.

use crate::site_file::SlotSpec;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub const KIND: &str = "copy";
pub const MAX_FIELDS: usize = 8;
pub const MAX_LEN: u32 = 500;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub name: String,
    pub max: u32,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CopySpec {
    pub fields: Vec<Field>,
}

/// A variant's text: field name to value.
pub type Text = BTreeMap<String, String>;

/// Why `text` is not a valid variant for `spec`, or `Ok`.
pub fn check(spec: &CopySpec, text: &Text) -> Result<(), String> {
    for (name, value) in text {
        let Some(f) = spec.fields.iter().find(|f| &f.name == name) else {
            return Err(format!("{name:?} is not a field of this slot"));
        };
        if value.chars().count() > f.max as usize {
            return Err(format!("{name:?} is {} characters; its max is {}", value.chars().count(), f.max));
        }
        if value.contains(['<', '>']) {
            return Err(format!("{name:?} contains < or >: copy fields are plain text, no HTML"));
        }
        if value.chars().any(char::is_control) {
            return Err(format!("{name:?} contains a control character or newline: a field is one line"));
        }
    }
    for f in spec.fields.iter().filter(|f| f.required) {
        if text.get(&f.name).is_none_or(|v| v.trim().is_empty()) {
            return Err(format!("{:?} is required", f.name));
        }
    }
    Ok(())
}

/// The stored payload for a valid text.
pub fn payload(text: &Text) -> String {
    serde_json::to_string(text).expect("a string map serialises")
}

/// A stored payload read back as fields; `None` for a baseline or anything not an object.
pub fn parse(payload: Option<&str>) -> Option<Value> {
    payload.and_then(|p| serde_json::from_str::<Map<String, Value>>(p).ok()).map(Value::Object)
}

/// A stored payload checked against the slot's stored spec (for approval).
pub fn check_stored(spec: &Value, payload: Option<&str>) -> Result<(), String> {
    let spec: CopySpec = serde_json::from_value(spec.clone()).map_err(|e| format!("bad stored copy spec: {e}"))?;
    let text: Text = payload.and_then(|p| serde_json::from_str(p).ok()).ok_or("the draft carries no copy fields")?;
    check(&spec, &text)
}

/// The slot's stored `copy` column, or `None` for another kind.
pub fn stored(slot: &SlotSpec) -> Option<Value> {
    (slot.kind.as_deref() == Some(KIND)).then(|| serde_json::to_value(slot.copy.as_ref().expect("validated")).expect("serialises"))
}

/// A copy slot's own rules, and the refusal of copy options on any other kind.
pub fn validate_slot(slot: &SlotSpec) -> Result<()> {
    let k = &slot.key;
    if slot.kind.as_deref() != Some(KIND) {
        if slot.copy.is_some() || slot.variants.iter().any(|v| v.copy.is_some()) {
            bail!("slot {k:?}: only a kind \"copy\" slot takes copy fields");
        }
        return Ok(());
    }
    if slot.in_session || slot.picker.is_some() || slot.sorted.is_some() {
        bail!("slot {k:?}: a copy slot cannot be in_session or carry picker or sorted options");
    }
    let Some(spec) = &slot.copy else { bail!("slot {k:?}: a copy slot needs copy.fields") };
    if spec.fields.is_empty() || spec.fields.len() > MAX_FIELDS {
        bail!("slot {k:?}: a copy slot needs 1-{MAX_FIELDS} fields");
    }
    let mut names = std::collections::HashSet::new();
    for f in &spec.fields {
        if !crate::site_file::is_slug(&f.name) || !names.insert(&f.name) {
            bail!("slot {k:?}: field name {:?} is invalid or repeated", f.name);
        }
        if f.max == 0 || f.max > MAX_LEN {
            bail!("slot {k:?}: field {:?} max must be 1 to {MAX_LEN}", f.name);
        }
    }
    for v in &slot.variants {
        if v.html.is_some() {
            bail!("slot {k:?}: variant {:?} takes copy fields, not html", v.key);
        }
        match (v.baseline, &v.copy) {
            (true, Some(_)) => bail!("slot {k:?}: baseline {:?} must not carry copy; it is the page's own", v.key),
            (false, None) => bail!("slot {k:?}: variant {:?} needs copy", v.key),
            (false, Some(t)) => check(spec, t).map_err(|e| anyhow::anyhow!("slot {k:?}: variant {:?}: {e}", v.key))?,
            _ => {}
        }
    }
    Ok(())
}

/// A stored payload's values, one per line, for the constraint checks.
pub fn plain(payload: Option<&str>) -> String {
    let text: Text = payload.and_then(|p| serde_json::from_str(p).ok()).unwrap_or_default();
    text.into_values().collect::<Vec<_>>().join("\n")
}
