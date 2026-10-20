//! F4: the `copy` slot kind. A copy slot's variants are named typed fields, not HTML
//! (backend.md, "Copy slots"). The payload is the field map as a JSON object; a decision returns
//! it as `copy` and a `null` payload.
//!
//! A field is `text` (the default: one line of plain text within `max` characters) or `image`
//! (S11): `{asset | url, alt, width?, height?}`. `asset` is the SHA-256 of an image uploaded to
//! this site's asset store ([`crate::assets`]); `url` is an `https://` or root-relative address.
//! `alt` is required and is plain text within the field's `max` (default 250). Renderers set the
//! image's `src` and `alt` attributes; nothing from a field ever becomes markup.

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
    /// Text: the value's maximum length in characters. Image: the alt text's (default 250).
    #[serde(default)]
    pub max: u32,
    #[serde(default)]
    pub required: bool,
    /// `text` (default) or `image`.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

pub const IMAGE: &str = "image";
pub const DEFAULT_ALT_MAX: u32 = 250;
pub const MAX_URL: usize = 2048;
pub const MAX_DIM: u32 = 10_000;

impl Field {
    pub fn is_image(&self) -> bool {
        self.kind.as_deref() == Some(IMAGE)
    }
    fn limit(&self) -> u32 {
        if self.is_image() && self.max == 0 { DEFAULT_ALT_MAX } else { self.max }
    }
}

/// An image field's value. Exactly one of `asset` and `url`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub alt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

/// One field's value: a line of text, or an image.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum FieldValue {
    Text(String),
    Image(Image),
}

/// A string is text; an object is an image, read strictly so a misspelt or extra key (an
/// `onerror`) is named in the refusal.
impl<'de> Deserialize<'de> for FieldValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        match Value::deserialize(d)? {
            Value::String(s) => Ok(FieldValue::Text(s)),
            v @ Value::Object(_) => serde_json::from_value(v).map(FieldValue::Image).map_err(|e| D::Error::custom(format!("an image field value: {e}"))),
            other => Err(D::Error::custom(format!("a field value is text or an image {{asset or url, alt}}, not {other}"))),
        }
    }
}

impl From<&str> for FieldValue {
    fn from(s: &str) -> Self {
        FieldValue::Text(s.to_string())
    }
}

/// Plain text a field may hold: one line, no markup characters, within `max` characters.
fn plain_line(name: &str, what: &str, value: &str, max: u32) -> Result<(), String> {
    if value.chars().count() > max as usize {
        return Err(format!("{name:?}{what} is {} characters; its max is {max}", value.chars().count()));
    }
    if value.contains(['<', '>']) {
        return Err(format!("{name:?}{what} contains < or >: copy fields are plain text, no HTML"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{name:?}{what} contains a control character or newline: a field is one line"));
    }
    Ok(())
}

/// An image's address: `https://host/...` or a root-relative `/path`, with nothing a renderer
/// would have to escape or that a browser would read as another scheme.
pub fn check_url(url: &str) -> Result<(), String> {
    let ok_chars = url.bytes().all(|b| b.is_ascii_graphic() && !matches!(b, b'"' | b'\'' | b'<' | b'>' | b'`' | b'\\'));
    let shape = url.strip_prefix("https://").is_some_and(|r| !r.is_empty() && !r.starts_with('/'))
        || (url.starts_with('/') && !url.starts_with("//"));
    if url.len() > MAX_URL || !ok_chars || !shape {
        return Err(format!("image url {url:?} must be https://... or a root-relative /path (at most {MAX_URL} characters, no spaces or quotes)"));
    }
    Ok(())
}

fn check_image(name: &str, f: &Field, img: &Image) -> Result<(), String> {
    match (&img.asset, &img.url) {
        (Some(a), None) => {
            if !crate::assets::is_hash(a) {
                return Err(format!("{name:?}: asset {a:?} is not an asset id (the 64-character SHA-256 an upload returns)"));
            }
        }
        (None, Some(u)) => check_url(u).map_err(|e| format!("{name:?}: {e}"))?,
        _ => return Err(format!("{name:?}: an image needs exactly one of asset or url")),
    }
    if img.alt.trim().is_empty() {
        return Err(format!("{name:?}: alt text is required: say what the image shows"));
    }
    plain_line(name, " alt", &img.alt, f.limit())?;
    for d in [img.width, img.height].into_iter().flatten() {
        if d == 0 || d > MAX_DIM {
            return Err(format!("{name:?}: width and height must be 1 to {MAX_DIM}"));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CopySpec {
    pub fields: Vec<Field>,
}

/// A variant's fields: field name to value.
pub type Text = BTreeMap<String, FieldValue>;

/// Why `text` is not a valid variant for `spec`, or `Ok`.
pub fn check(spec: &CopySpec, text: &Text) -> Result<(), String> {
    for (name, value) in text {
        let Some(f) = spec.fields.iter().find(|f| &f.name == name) else {
            return Err(format!("{name:?} is not a field of this slot"));
        };
        match (value, f.is_image()) {
            (FieldValue::Text(v), false) => plain_line(name, "", v, f.max)?,
            (FieldValue::Image(img), true) => check_image(name, f, img)?,
            (FieldValue::Text(_), true) => return Err(format!("{name:?} is an image field: give {{asset or url, alt}}, not text")),
            (FieldValue::Image(_), false) => return Err(format!("{name:?} is a text field: give a string, not an image")),
        }
    }
    for f in spec.fields.iter().filter(|f| f.required) {
        let blank = match text.get(&f.name) {
            None => true,
            Some(FieldValue::Text(v)) => v.trim().is_empty(),
            Some(FieldValue::Image(_)) => false,
        };
        if blank {
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
    let text = read_text(payload)?;
    check(&spec, &text)
}

/// A stored or proposed payload as fields, with a reason when it is not one.
pub fn read_text(payload: Option<&str>) -> Result<Text, String> {
    let p = payload.ok_or("the draft carries no copy fields")?;
    serde_json::from_str(p).map_err(|_| "the draft's fields are not a {name: text or image} object (an image is {asset or url, alt, width?, height?})".to_string())
}

/// The asset ids a payload's image fields name (S11), so a door can check they are this site's.
pub fn asset_refs(text: &Text) -> Vec<String> {
    text.values()
        .filter_map(|v| match v {
            FieldValue::Image(Image { asset: Some(a), .. }) => Some(a.clone()),
            _ => None,
        })
        .collect()
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
        match f.kind.as_deref() {
            None | Some("text") | Some(IMAGE) => {}
            Some(t) => bail!("slot {k:?}: field {:?} type {t:?} is not text or image", f.name),
        }
        if (f.max == 0 && !f.is_image()) || f.max > MAX_LEN {
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

/// A stored payload's words, one per line, for the constraint checks: each text field's value
/// and each image's alt text (an image's claim is in its alt as much as a headline's is in it).
pub fn plain(payload: Option<&str>) -> String {
    let text: Text = payload.and_then(|p| serde_json::from_str(p).ok()).unwrap_or_default();
    text.into_values()
        .map(|v| match v {
            FieldValue::Text(t) => t,
            FieldValue::Image(i) => i.alt,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
