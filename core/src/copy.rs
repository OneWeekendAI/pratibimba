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
use crate::template::{self, Template};
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

/// One field's value: a line of text, an image, or a template (P1, [`crate::template`]) that
/// decide fills into a line of text.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Text(String),
    Image(Image),
    Template(Template),
}

/// Text and images serialise as before; a template as `{"tpl": parts, "src": "..."}`, its parsed
/// form, so decide never parses.
impl Serialize for FieldValue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            FieldValue::Text(t) => t.serialize(s),
            FieldValue::Image(i) => i.serialize(s),
            FieldValue::Template(t) => serde_json::json!({ "tpl": t.parts, "src": t.src }).serialize(s),
        }
    }
}

/// The door form (site file, dashboard draft, MCP): every string is template language
/// ([`crate::template`]'s "Braces"): with no placeholder it is text, its `{{`/`}}` read as one
/// brace; with one it is a template; a lone brace that is no placeholder is refused, naming the
/// `{{` escape. An object with `tpl` is a template in its parsed form, re-parsed from its `src`
/// and refused if the parts disagree (so a hand-made `tpl` cannot smuggle what the parser
/// refuses); any other object is an image, read strictly so a misspelt or extra key (an
/// `onerror`) is named. A stored payload is read by [`read_text`] instead, never by this.
impl<'de> Deserialize<'de> for FieldValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        match Value::deserialize(d)? {
            Value::String(s) if s.contains(['{', '}']) => door_string(&s).map_err(D::Error::custom),
            Value::String(s) => Ok(FieldValue::Text(s)),
            v => object_value(v).map_err(D::Error::custom),
        }
    }
}

fn door_string(s: &str) -> Result<FieldValue, String> {
    let t = template::parse(s).map_err(|e| format!("a template: {e} (for a literal brace write {{{{ or }}}})"))?;
    if t.parts.iter().all(|p| matches!(p, template::Part::Lit(_))) {
        let text = t.parts.into_iter().map(|p| if let template::Part::Lit(l) = p { l } else { unreachable!() }).collect();
        return Ok(FieldValue::Text(text));
    }
    Ok(FieldValue::Template(t))
}

/// A non-string value, the same at the door and in storage: a parsed template or an image.
fn object_value(v: Value) -> Result<FieldValue, String> {
    match v {
        Value::Object(m) if m.contains_key("tpl") => stored_template(m).map(FieldValue::Template).map_err(|e| format!("a stored template: {e}")),
        v @ Value::Object(_) => serde_json::from_value(v).map(FieldValue::Image).map_err(|e| format!("an image field value: {e}")),
        other => Err(format!("a field value is text or an image {{asset or url, alt}}, not {other}")),
    }
}

/// A stored value: a string is plain text, verbatim (never template language, so copy stored
/// before migration 0300, or since as plain text with a brace, reads as written).
fn stored_value(v: Value) -> Result<FieldValue, String> {
    match v {
        Value::String(s) => Ok(FieldValue::Text(s)),
        v => object_value(v),
    }
}

fn stored_template(mut m: Map<String, Value>) -> Result<Template, String> {
    let src = match m.remove("src") {
        Some(Value::String(s)) => s,
        _ => return Err("needs its src string".into()),
    };
    let parts: Vec<template::Part> = serde_json::from_value(m.remove("tpl").expect("checked")).map_err(|e| e.to_string())?;
    if let Some(k) = m.keys().next() {
        return Err(format!("unknown key {k:?}"));
    }
    let t = template::parse(&src)?;
    if t.parts != parts {
        return Err("its parts do not match its src".into());
    }
    Ok(t)
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

/// A template's worst case within the field's `max`: draft time renders every placeholder at its
/// longest value (decide time truncates, never refuses).
fn fits(name: &str, t: &Template, max: u32) -> Result<(), String> {
    let n = template::longest(t);
    if n > max as usize {
        let path = template::widest(t).map(|p| p.as_str()).unwrap_or("value");
        return Err(format!("{name:?} can reach {n} characters with the longest {path}; its max is {max}. Add | truncate:N"));
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
            (FieldValue::Template(t), false) => fits(name, t, f.max)?,
            (FieldValue::Text(_) | FieldValue::Template(_), true) => return Err(format!("{name:?} is an image field: give {{asset or url, alt}}, not text")),
            (FieldValue::Image(_), false) => return Err(format!("{name:?} is a text field: give a string, not an image")),
        }
    }
    for f in spec.fields.iter().filter(|f| f.required) {
        let blank = match text.get(&f.name) {
            None => true,
            Some(FieldValue::Text(v)) => v.trim().is_empty(),
            Some(FieldValue::Image(_) | FieldValue::Template(_)) => false,
        };
        if blank {
            return Err(format!("{:?} is required", f.name));
        }
    }
    Ok(())
}

/// The most paths one variant may require.
pub const MAX_REQUIRES: usize = 10;

/// A variant's `requires` (P1): each entry a template path, none repeated, and only on a copy
/// slot's non-baseline variant (the baseline is offered to everyone; the database's CHECK says
/// the same). Returns the paths in the order given.
pub fn validate_requires(requires: &[String], is_copy: bool, baseline: bool) -> Result<Vec<template::Path>, String> {
    if requires.is_empty() {
        return Ok(vec![]);
    }
    if !is_copy {
        return Err("requires is only for a copy slot's variants".into());
    }
    if baseline {
        return Err("the baseline is the page's own content, offered to everyone: it has no requires".into());
    }
    if requires.len() > MAX_REQUIRES {
        return Err(format!("requires lists at most {MAX_REQUIRES} paths"));
    }
    let mut out = vec![];
    for r in requires {
        let p = template::Path::parse(r).map_err(|e| format!("requires: {e}"))?;
        if out.contains(&p) {
            return Err(format!("requires names {r:?} twice"));
        }
        out.push(p);
    }
    Ok(out)
}

/// The template paths whose presence `p` vouches for: `visitor.top_item`, its name and its count
/// are present or missing together (a top item is a counted order line).
fn vouches_for(r: template::Path, p: template::Path) -> bool {
    use template::Path::*;
    let top = |x| matches!(x, TopItem | TopItemName | TopItemCount);
    r == p || (top(r) && top(p))
}

/// A required text field must never fill to empty, so each placeholder in one must be safe: its
/// path in `requires` (the arm closes without it) or a non-empty `fallback`. Every path can be
/// missing (a visitor with no history, or none we may read, has none), so no path is exempt.
pub fn check_covered(spec: &CopySpec, text: &Text, requires: &[String]) -> Result<(), String> {
    let req: Vec<template::Path> = requires.iter().filter_map(|r| template::Path::parse(r).ok()).collect();
    for f in spec.fields.iter().filter(|f| f.required) {
        let Some(FieldValue::Template(t)) = text.get(&f.name) else { continue };
        for part in &t.parts {
            let template::Part::Ref { path, fmt } = part else { continue };
            let has_fallback = fmt.iter().any(|x| matches!(x, template::Fmt::Fallback(s) if !s.trim().is_empty()));
            if has_fallback || req.iter().any(|r| vouches_for(*r, *path)) {
                continue;
            }
            let p = path.as_str();
            return Err(format!(
                "{:?} is required, but {{{p}}} is missing for a visitor without that history and would leave it empty. \
                 Add \"{p}\" to requires (the variant is then shown only to visitors who have it), \
                 or give it a fallback: {{{p} | fallback: 'your favourite'}}",
                f.name
            ));
        }
    }
    Ok(())
}

/// [`check_covered`] against a slot's stored spec.
pub fn check_covered_stored(spec: &Value, text: &Text, requires: &[String]) -> Result<(), String> {
    check_covered(&read_spec(spec)?, text, requires)
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
    let spec = read_spec(spec)?;
    let text = read_text(payload)?;
    check(&spec, &text)
}

fn read_spec(spec: &Value) -> Result<CopySpec, String> {
    serde_json::from_value(spec.clone()).map_err(|e| format!("bad stored copy spec: {e}"))
}

const SHAPE: &str = "the draft's fields are not a {name: text or image} object (an image is {asset or url, alt, width?, height?})";

/// A stored payload as fields (see [`stored_value`]), with a reason when it is not one.
pub fn read_text(payload: Option<&str>) -> Result<Text, String> {
    let p = payload.ok_or("the draft carries no copy fields")?;
    let m: Map<String, Value> = serde_json::from_str(p).map_err(|_| SHAPE.to_string())?;
    m.into_iter().map(|(k, v)| stored_value(v).map(|v| (k, v))).collect()
}

/// A draft's fields as a person or agent sent them (the door form: strings are template
/// language), checked against the slot's stored spec. Store it with [`payload`].
pub fn read_draft(spec: &Value, payload: &str) -> Result<Text, String> {
    let text: Text = serde_json::from_str(payload).map_err(|e| {
        // A template's own refusal says what to fix; the shape message would hide it.
        let e = e.to_string();
        if e.contains("template") { e } else { SHAPE.to_string() }
    })?;
    check(&read_spec(spec)?, &text)?;
    Ok(text)
}

/// A decision's `copy`: the stored fields with every template filled from the visitor's facts
/// (`None`: no history may be read) within its field's `max`, so only plain strings and images
/// leave Core; and whether any field was a template (the decision then stores the result as
/// `filled`). A payload that does not read keeps its strings and images and drops anything else.
pub fn render(payload: Option<&str>, spec: Option<&Value>, f: Option<&crate::history::Facts>) -> (Value, bool) {
    let Some(p) = payload else { return (Value::Null, false) };
    let spec = spec.and_then(|s| read_spec(s).ok());
    let text = match read_text(Some(p)) {
        Ok(t) => t,
        Err(_) => {
            let m: Map<String, Value> = serde_json::from_str(p).unwrap_or_default();
            let keep = m.into_iter().filter(|(_, v)| v.is_string() || (v.is_object() && v.get("tpl").is_none())).collect();
            return (Value::Object(keep), false);
        }
    };
    let templated = text.values().any(|v| matches!(v, FieldValue::Template(_)));
    (fill_text(spec.as_ref(), &text, f), templated)
}

/// `text` as `{field: string | image}`, each template filled within its field's `max` (no spec:
/// [`MAX_LEN`]).
pub fn fill_text(spec: Option<&CopySpec>, text: &Text, f: Option<&crate::history::Facts>) -> Value {
    let max = |name: &str| spec.and_then(|s| s.fields.iter().find(|x| x.name == name)).map_or(MAX_LEN, |x| x.max.clamp(1, MAX_LEN));
    Value::Object(
        text.iter()
            .map(|(k, v)| {
                let v = match v {
                    FieldValue::Text(t) => Value::String(t.clone()),
                    FieldValue::Image(i) => serde_json::to_value(i).expect("an image serialises"),
                    FieldValue::Template(t) => Value::String(template::fill(t, f, max(k))),
                };
                (k.clone(), v)
            })
            .collect(),
    )
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
            (false, Some(t)) => check(spec, t)
                .and_then(|_| check_covered(spec, t, &v.requires))
                .map_err(|e| anyhow::anyhow!("slot {k:?}: variant {:?}: {e}", v.key))?,
            _ => {}
        }
    }
    Ok(())
}

/// A stored payload's words, one per line, for the constraint checks: each text field's value
/// and each image's alt text (an image's claim is in its alt as much as a headline's is in it).
pub fn plain(payload: Option<&str>) -> String {
    let text: Text = read_text(payload).unwrap_or_default();
    text.into_values()
        .map(|v| match v {
            FieldValue::Text(t) => t,
            FieldValue::Image(i) => i.alt,
            FieldValue::Template(t) => t.src,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hero() -> CopySpec {
        serde_json::from_value(serde_json::json!({"fields": [
            {"name": "headline", "max": 40, "required": true},
            {"name": "note", "max": 40}
        ]})).unwrap()
    }

    fn door(v: serde_json::Value) -> Text {
        serde_json::from_value(v).unwrap()
    }

    // Mutation: return Ok(()) at the top of check_covered -> red.
    #[test]
    fn required_field_placeholder_without_requires_or_fallback_refused() {
        let t = door(serde_json::json!({"headline": "{visitor.top_item.name | truncate: 20}"}));
        let e = check_covered(&hero(), &t, &[]).unwrap_err();
        assert!(e.contains("\"headline\" is required") && e.contains("requires") && e.contains("fallback"), "{e}");
    }

    // Mutation: drop the `req.iter().any(..)` term (requires never covers) -> red.
    #[test]
    fn required_field_placeholder_covered_by_requires_passes() {
        let t = door(serde_json::json!({"headline": "Your usual {visitor.top_item.name | truncate: 20}"}));
        assert_eq!(check_covered(&hero(), &t, &["visitor.top_item.name".into()]), Ok(()));
    }

    // Mutation: vouches_for = `r == p` only (no top-item family) -> red.
    #[test]
    fn requires_top_item_covers_its_name() {
        let t = door(serde_json::json!({"headline": "{visitor.top_item.name | truncate: 20}"}));
        assert_eq!(check_covered(&hero(), &t, &["visitor.top_item".into()]), Ok(()));
    }

    // Mutation: vouches_for returns true always -> red.
    #[test]
    fn requires_another_path_does_not_cover() {
        let t = door(serde_json::json!({"headline": "{visitor.top_item.name | truncate: 20}"}));
        assert!(check_covered(&hero(), &t, &["visitor.visit_count".into()]).is_err());
    }

    // Mutation: drop the `has_fallback` term -> red.
    #[test]
    fn required_field_placeholder_with_fallback_passes() {
        let t = door(serde_json::json!({"headline": "{visitor.top_item.name | truncate: 20, fallback: 'thali'}"}));
        assert_eq!(check_covered(&hero(), &t, &[]), Ok(()));
    }

    // Mutation: drop `!s.trim().is_empty()` from the fallback test -> red.
    #[test]
    fn blank_fallback_does_not_cover() {
        let t = door(serde_json::json!({"headline": "{visitor.top_item.name | truncate: 20, fallback: ' '}"}));
        assert!(check_covered(&hero(), &t, &[]).is_err());
    }

    // Mutation: drop `.filter(|f| f.required)` (check every field) -> red.
    #[test]
    fn optional_field_placeholder_needs_no_cover() {
        let t = door(serde_json::json!({"headline": "Hello", "note": "{visitor.top_item.name | truncate: 20}"}));
        assert_eq!(check_covered(&hero(), &t, &[]), Ok(()));
    }

    // Mutation: check only the first placeholder of a template -> red.
    #[test]
    fn second_placeholder_is_checked_too() {
        let t = door(serde_json::json!({"headline": "{visitor.top_item.name | truncate: 10, fallback: 'thali'} x{visitor.visit_count}"}));
        let e = check_covered(&hero(), &t, &[]).unwrap_err();
        assert!(e.contains("visitor.visit_count"), "{e}");
    }

    // Mutation: drop the `tpl`-key guard so every object is read as a template first -> red.
    #[test]
    fn image_object_still_parses_as_image() {
        let v: FieldValue = serde_json::from_str(r#"{"url":"/a.png","alt":"A thali"}"#).unwrap();
        assert!(matches!(v, FieldValue::Image(_)), "{v:?}");
    }

    #[test]
    fn a_string_with_a_placeholder_parses_as_a_template() {
        let v: FieldValue = serde_json::from_str(r#""Hi {visitor.visit_count}""#).unwrap();
        assert!(matches!(v, FieldValue::Template(_)), "{v:?}");
    }

    // At the door, `{{`/`}}` are one brace each. Mutation: keep the text verbatim -> red.
    #[test]
    fn a_door_string_with_only_doubled_braces_is_text_with_single_braces() {
        let v: FieldValue = serde_json::from_str(r#""a {{b}}""#).unwrap();
        assert_eq!(v, FieldValue::Text("a {b}".into()));
    }

    #[test]
    fn a_door_string_with_a_lone_brace_is_refused_naming_the_escape() {
        let e = serde_json::from_str::<FieldValue>(r#""Use code {SAVE}""#).unwrap_err().to_string();
        assert!(e.contains("{{"), "{e}");
    }

    // The old-data path: a stored string is never template language.
    // Mutation: read stored strings with the door's parser -> red.
    #[test]
    fn a_stored_string_with_a_lone_brace_reads_as_plain_text() {
        let t = read_text(Some(r#"{"headline":"Use code {SAVE} today"}"#)).unwrap();
        assert_eq!(t["headline"], FieldValue::Text("Use code {SAVE} today".into()));
    }

    #[test]
    fn a_stored_string_with_a_lone_brace_renders_verbatim() {
        let (v, templated) = render(Some(r#"{"headline":"Use code {SAVE}"}"#), None, None);
        assert_eq!((v, templated), (serde_json::json!({ "headline": "Use code {SAVE}" }), false));
    }

    // The escape round-trips: the door's `{{` is stored as one brace and renders as one.
    #[test]
    fn an_escaped_brace_is_stored_and_rendered_as_one() {
        let text: Text = serde_json::from_str(r#"{"headline":"Code {{SAVE}} for {visitor.top_item | truncate: 9}"}"#).unwrap();
        let stored = payload(&text);
        let (v, _) = render(Some(&stored), None, None);
        assert_eq!(v["headline"], "Code {SAVE} for");
    }

    // Mutation: return a template's object as-is (no fill) -> red.
    #[test]
    fn render_never_returns_a_tpl_object() {
        let text: Text = serde_json::from_str(r#"{"headline":"Hi {visitor.top_item | truncate: 9, fallback: 'there'}"}"#).unwrap();
        let (v, templated) = render(Some(&payload(&text)), None, None);
        assert!(templated && !v.to_string().contains("tpl"), "{v}");
        assert_eq!(v["headline"], "Hi there");
    }

    // Mutation: skip the src/parts agreement check -> red.
    #[test]
    fn a_stored_template_whose_parts_disagree_with_its_src_is_refused() {
        let e = serde_json::from_str::<FieldValue>(r#"{"tpl":[{"lit":"<b>hi</b>"}],"src":"hi"}"#).unwrap_err();
        assert!(e.to_string().contains("do not match"), "{e}");
    }
}
