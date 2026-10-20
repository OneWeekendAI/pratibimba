//! S11: the asset store. Core keeps the images variants show, because variants change while the
//! site runs: an agent or a person uploads a banner, a variant names it, and the page shows it
//! without a deploy.
//!
//! - **Per site, content-addressed.** An asset's id is the SHA-256 of its bytes. It is stored
//!   under `<site>/<sha256>` and served publicly at `GET /assets/<site>/<sha256>` with a
//!   one-year `immutable` cache: the same name can never mean other bytes. A variant on one
//!   site cannot name another site's asset (every door checks [`check_refs`]), and the public
//!   route answers 404 for an id the site never uploaded, even when another site holds it.
//! - **Images only, judged by their bytes.** PNG, JPEG, WebP, AVIF, GIF and SVG, recognised
//!   by content sniffing ([`sniff`]); the file name and the declared content type are ignored.
//!   A size cap applies (`PRB_ASSET_MAX_BYTES`, default 5 MiB).
//! - **SVG cannot run script, twice over.** An upload is refused when it holds a script, an
//!   event handler, a `javascript:` URL, a foreign object, a DOCTYPE, or any reference outside
//!   the file ([`check_svg`]). And every asset is served with `Content-Security-Policy:
//!   default-src 'none'; sandbox` and `nosniff`, so even a file that slipped through could not
//!   run if a visitor opened it directly; an SVG is also served as an attachment.
//! - **Storage is a trait** ([`Store`]): a local directory (`PRB_ASSETS_DIR`, the default) or
//!   any S3-compatible bucket (AWS, Cloudflare R2, MinIO), signed with SigV4 ([`S3`]).
//! - **Garbage collection** ([`gc`], run by the worker): an asset that no live variant of its
//!   site references (draft, approved or paused, by asset id in an image field or by its URL in
//!   swap HTML) is marked; once it has stayed unreferenced for the grace period
//!   (`PRB_ASSET_GC_GRACE_SECS`, default 7 days) its row and bytes are deleted.

use crate::http::{ApiError, AppState};
use anyhow::{Context, Result, bail};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub const DEFAULT_MAX_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_MAX_BYTES: usize = 50 * 1024 * 1024;
pub const DEFAULT_GRACE_SECS: i64 = 7 * 86_400;
pub const MIMES: [&str; 6] = ["image/png", "image/jpeg", "image/webp", "image/avif", "image/gif", "image/svg+xml"];
/// Served on every asset: nothing it holds may run, load or frame anything.
pub const CSP: &str = "default-src 'none'; img-src data:; style-src 'unsafe-inline'; sandbox";
pub const CACHE: &str = "public, max-age=31536000, immutable";

/// A lowercase 64-character SHA-256 hex digest: the only shape an asset id has.
pub fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The public path an asset is served at, relative to Core (owners proxy it at `/_prb`).
pub fn path(site: &str, hash: &str) -> String {
    format!("/assets/{site}/{hash}")
}

// ---- what the bytes are ----

/// The image type the bytes are, by their content; `None` for anything else.
pub fn sniff(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if b.len() >= 16 && &b[4..8] == b"ftyp" {
        let size = (u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize).clamp(16, b.len());
        // The major brand, then the compatible brands, four bytes each after the minor version.
        let brands = std::iter::once(&b[8..12]).chain(b[16..size].chunks_exact(4));
        if brands.into_iter().any(|br| br == b"avif" || br == b"avis") {
            return Some("image/avif");
        }
        return None;
    }
    svg_root(b).then_some("image/svg+xml")
}

/// The document's first element is `<svg`, after an optional BOM, XML declaration, comments and
/// whitespace. A DOCTYPE is not skipped, so a file with one is not an SVG here (entity tricks).
fn svg_root(b: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(b) else { return false };
    let mut s = text.strip_prefix('\u{feff}').unwrap_or(text);
    loop {
        s = s.trim_start();
        if let Some(r) = s.strip_prefix("<?xml") {
            match r.find("?>") {
                Some(i) => s = &r[i + 2..],
                None => return false,
            }
        } else if let Some(r) = s.strip_prefix("<!--") {
            match r.find("-->") {
                Some(i) => s = &r[i + 3..],
                None => return false,
            }
        } else {
            break;
        }
    }
    s.strip_prefix("<svg").and_then(|r| r.chars().next()).is_some_and(|c| c.is_whitespace() || c == '>' || c == '/')
}

const SVG_BANNED: [&str; 12] = [
    "<script", "<foreignobject", "<iframe", "<embed", "<object", "<handler", "<listener", "<!doctype", "<!entity", "javascript:", "vbscript:", "@import",
];

/// Why an SVG upload is refused, or `Ok`. Errs towards refusing: an SVG a design tool exports
/// (shapes, gradients, `<style>`, internal `#` references) passes; anything that could run code,
/// load another resource or be decoded into either does not.
pub fn check_svg(b: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(b).map_err(|_| "the SVG is not UTF-8 text".to_string())?;
    let lower = text.to_ascii_lowercase();
    if let Some(bad) = SVG_BANNED.iter().find(|t| lower.contains(*t)) {
        return Err(format!("the SVG contains {bad:?}: an image may not carry script or load anything"));
    }
    if lower.contains('\\') {
        return Err("the SVG holds a backslash escape, which could spell a link past these checks; write it plainly".into());
    }
    if lower.contains("data:") {
        return Err("the SVG embeds a data: URL; upload the embedded image on its own".into());
    }
    // Every url(...) in a style must point inside the file.
    let mut rest = lower.as_str();
    while let Some(i) = rest.find("url(") {
        let inner = rest[i + 4..].trim_start().trim_start_matches(['"', '\'']);
        if !inner.starts_with('#') {
            return Err("the SVG has a url(...) outside the file; only #references are allowed".into());
        }
        rest = &rest[i + 4..];
    }
    for (name, value) in attributes(&lower) {
        if name.starts_with("on") {
            return Err(format!("the SVG has an event handler ({name}); an image may not carry script"));
        }
        let local = name.rsplit(':').next().unwrap_or(&name);
        if matches!(local, "href" | "src" | "action" | "formaction") && !value.trim().starts_with('#') && !value.trim().is_empty() {
            return Err(format!("the SVG links outside itself ({name}={value:?}); only #references are allowed"));
        }
        if local == "attributename" && value.trim().ends_with("href") {
            return Err("the SVG animates a link; an image may not change where it points".into());
        }
        // A numeric reference could spell any of the above past the plain-text checks.
        if value.contains("&#") {
            return Err(format!("the SVG has an encoded character in {name}; write it plainly"));
        }
    }
    Ok(())
}

/// Every attribute in a (lowercased) markup string as (name, value), quoted or not. A small
/// scanner, like the checks': it does not need to render the file, only to see what is in it.
fn attributes(s: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    let b = s.as_bytes();
    let mut i = 0;
    while let Some(off) = s[i..].find('<') {
        i += off + 1;
        if s[i..].starts_with("!--") {
            i = s[i..].find("-->").map_or(s.len(), |e| i + e + 3);
            continue;
        }
        // Skip the tag name.
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
            i += 1;
        }
        loop {
            while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
                i += 1;
            }
            if i >= b.len() || b[i] == b'>' {
                break;
            }
            let start = i;
            while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>' | b'/') {
                i += 1;
            }
            let name = s[start..i].to_string();
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            let mut value = String::new();
            if i < b.len() && b[i] == b'=' {
                i += 1;
                while i < b.len() && b[i].is_ascii_whitespace() {
                    i += 1;
                }
                if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                    let q = b[i];
                    let vs = i + 1;
                    i = vs + s[vs..].bytes().position(|c| c == q).unwrap_or(s.len() - vs);
                    value = s[vs..i.min(s.len())].to_string();
                    i += 1;
                } else {
                    let vs = i;
                    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                        i += 1;
                    }
                    value = s[vs..i].to_string();
                }
            }
            if name.is_empty() {
                i += 1;
                continue;
            }
            out.push((name, value));
        }
    }
    out
}

/// Pixel size where the header gives it cheaply (PNG, GIF, JPEG); `None` otherwise.
pub fn dimensions(mime: &str, b: &[u8]) -> Option<(u32, u32)> {
    match mime {
        "image/png" if b.len() >= 24 => Some((u32::from_be_bytes(b[16..20].try_into().ok()?), u32::from_be_bytes(b[20..24].try_into().ok()?))),
        "image/gif" if b.len() >= 10 => Some((u16::from_le_bytes([b[6], b[7]]) as u32, u16::from_le_bytes([b[8], b[9]]) as u32)),
        "image/jpeg" => {
            let mut i = 2;
            while i + 9 < b.len() {
                if b[i] != 0xFF {
                    return None;
                }
                let m = b[i + 1];
                let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
                if (0xC0..=0xCF).contains(&m) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
                    return Some((u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32, u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32));
                }
                i += 2 + len;
            }
            None
        }
        _ => None,
    }
    .filter(|(w, h)| *w > 0 && *h > 0)
}

// ---- storage ----

/// Where asset bytes live. Keys are `<site>/<sha256>`, checked by [`check_key`] before any
/// backend sees them.
pub trait Store: Send + Sync {
    fn put<'a>(&'a self, key: &'a str, bytes: &'a [u8], mime: &'a str) -> BoxFuture<'a, Result<()>>;
    fn get<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Vec<u8>>>>;
    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<()>>;
    fn describe(&self) -> String;
    /// The directory, for a local store (backups copy it).
    fn local_dir(&self) -> Option<PathBuf> {
        None
    }
}

/// A storage key: exactly `<site>/<sha256>`, the site made of id characters. Nothing that could
/// climb out of a directory or name another bucket path gets through.
pub fn check_key(key: &str) -> Result<()> {
    let Some((site, hash)) = key.split_once('/') else { bail!("bad asset key {key:?}") };
    let site_ok = !site.is_empty() && site.len() <= 128 && site != "." && site != ".." && site.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'~' | b'.'));
    if !site_ok || !is_hash(hash) {
        bail!("bad asset key {key:?}");
    }
    Ok(())
}

pub fn key(site: &str, hash: &str) -> String {
    format!("{site}/{hash}")
}

/// A local directory: `<dir>/<site>/<sha256>`.
pub struct Local {
    pub dir: PathBuf,
}

impl Local {
    fn file(&self, key: &str) -> Result<PathBuf> {
        check_key(key)?;
        Ok(key.split('/').fold(self.dir.clone(), |p, part| p.join(part)))
    }
}

impl Store for Local {
    fn put<'a>(&'a self, key: &'a str, bytes: &'a [u8], _mime: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let file = self.file(key)?;
            let parent = file.parent().context("an asset file has a directory")?;
            tokio::fs::create_dir_all(parent).await.with_context(|| format!("cannot create {}", parent.display()))?;
            // Write beside, then rename: a reader never sees half a file.
            let tmp = parent.join(format!(".{}.{}", uuid::Uuid::new_v4().simple(), "part"));
            tokio::fs::write(&tmp, bytes).await.with_context(|| format!("cannot write {}", tmp.display()))?;
            tokio::fs::rename(&tmp, &file).await?;
            Ok(())
        })
    }
    fn get<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Vec<u8>>>> {
        Box::pin(async move {
            match tokio::fs::read(self.file(key)?).await {
                Ok(b) => Ok(Some(b)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.into()),
            }
        })
    }
    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            match tokio::fs::remove_file(self.file(key)?).await {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            }
        })
    }
    fn describe(&self) -> String {
        format!("local directory {}", self.dir.display())
    }
    fn local_dir(&self) -> Option<PathBuf> {
        Some(self.dir.clone())
    }
}

/// In memory: the default for tests and for an `AppState` nobody configured.
#[derive(Default)]
pub struct Memory {
    pub blobs: Mutex<HashMap<String, Vec<u8>>>,
}

impl Store for Memory {
    fn put<'a>(&'a self, key: &'a str, bytes: &'a [u8], _mime: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            check_key(key)?;
            self.blobs.lock().unwrap().insert(key.into(), bytes.to_vec());
            Ok(())
        })
    }
    fn get<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Vec<u8>>>> {
        Box::pin(async move {
            check_key(key)?;
            Ok(self.blobs.lock().unwrap().get(key).cloned())
        })
    }
    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            check_key(key)?;
            self.blobs.lock().unwrap().remove(key);
            Ok(())
        })
    }
    fn describe(&self) -> String {
        "memory".into()
    }
}

/// An S3-compatible bucket (AWS S3, Cloudflare R2, MinIO), path-style, signed with SigV4.
pub struct S3 {
    pub endpoint: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
    pub http: reqwest::Client,
}

type HmacSha256 = Hmac<Sha256>;

fn hmac(key: &[u8], msg: &str) -> Vec<u8> {
    let mut m = HmacSha256::new_from_slice(key).expect("hmac takes any key");
    m.update(msg.as_bytes());
    m.finalize().into_bytes().to_vec()
}

impl S3 {
    /// The URL and SigV4 headers for one request on `key`.
    pub fn signed(&self, method: &str, key: &str, payload: &[u8], at: DateTime<Utc>) -> Result<(String, Vec<(String, String)>)> {
        check_key(key)?;
        let url = reqwest::Url::parse(&self.endpoint).context("PRB_ASSETS_S3_ENDPOINT is not a URL")?;
        let host = match url.port() {
            Some(p) => format!("{}:{p}", url.host_str().context("the endpoint has no host")?),
            None => url.host_str().context("the endpoint has no host")?.to_string(),
        };
        // Keys are made of unreserved characters only (check_key), so they need no encoding.
        let uri = format!("{}/{}/{key}", url.path().trim_end_matches('/'), self.bucket);
        let payload_hash = sha256_hex(payload);
        let amz = at.format("%Y%m%dT%H%M%SZ").to_string();
        let date = at.format("%Y%m%d").to_string();
        let signed_headers = "host;x-amz-content-sha256;x-amz-date";
        let canonical = format!("{method}\n{uri}\n\nhost:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz}\n\n{signed_headers}\n{payload_hash}");
        let scope = format!("{date}/{}/s3/aws4_request", self.region);
        let to_sign = format!("AWS4-HMAC-SHA256\n{amz}\n{scope}\n{}", sha256_hex(canonical.as_bytes()));
        let k = hmac(format!("AWS4{}", self.secret_key).as_bytes(), &date);
        let k = hmac(&k, &self.region);
        let k = hmac(&k, "s3");
        let k = hmac(&k, "aws4_request");
        let sig: String = hmac(&k, &to_sign).iter().map(|b| format!("{b:02x}")).collect();
        let auth = format!("AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={sig}", self.access_key);
        let base = format!("{}://{host}", url.scheme());
        Ok((
            format!("{base}{uri}"),
            vec![("authorization".into(), auth), ("x-amz-content-sha256".into(), payload_hash), ("x-amz-date".into(), amz)],
        ))
    }

    async fn send(&self, method: reqwest::Method, key: &str, body: Option<(&[u8], &str)>) -> Result<reqwest::Response> {
        let payload = body.map(|b| b.0).unwrap_or(&[]);
        let (url, headers) = self.signed(method.as_str(), key, payload, Utc::now())?;
        let mut req = self.http.request(method, url);
        for (k, v) in headers {
            req = req.header(k, v);
        }
        if let Some((bytes, mime)) = body {
            req = req.header("content-type", mime).body(bytes.to_vec());
        }
        Ok(req.send().await?)
    }
}

impl Store for S3 {
    fn put<'a>(&'a self, key: &'a str, bytes: &'a [u8], mime: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let r = self.send(reqwest::Method::PUT, key, Some((bytes, mime))).await?;
            if !r.status().is_success() {
                bail!("S3 PUT {key} answered {}: {}", r.status(), r.text().await.unwrap_or_default().chars().take(300).collect::<String>());
            }
            Ok(())
        })
    }
    fn get<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Vec<u8>>>> {
        Box::pin(async move {
            let r = self.send(reqwest::Method::GET, key, None).await?;
            match r.status() {
                StatusCode::NOT_FOUND => Ok(None),
                s if s.is_success() => Ok(Some(r.bytes().await?.to_vec())),
                s => bail!("S3 GET {key} answered {s}"),
            }
        })
    }
    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let r = self.send(reqwest::Method::DELETE, key, None).await?;
            if !(r.status().is_success() || r.status() == StatusCode::NOT_FOUND) {
                bail!("S3 DELETE {key} answered {}", r.status());
            }
            Ok(())
        })
    }
    fn describe(&self) -> String {
        format!("S3 bucket {} at {}", self.bucket, self.endpoint)
    }
}

/// The asset store and its limits.
#[derive(Clone)]
pub struct Assets {
    pub store: Arc<dyn Store>,
    pub max_bytes: usize,
    pub grace: chrono::Duration,
}

impl Assets {
    pub fn with_store(store: Arc<dyn Store>) -> Self {
        Assets { store, max_bytes: DEFAULT_MAX_BYTES, grace: chrono::Duration::seconds(DEFAULT_GRACE_SECS) }
    }

    pub fn memory() -> Self {
        Self::with_store(Arc::new(Memory::default()))
    }

    pub fn local(dir: impl Into<PathBuf>) -> Self {
        Self::with_store(Arc::new(Local { dir: dir.into() }))
    }

    /// From the environment: S3 when `PRB_ASSETS_S3_BUCKET` is set, else the local directory
    /// `PRB_ASSETS_DIR` (default `./prb-assets`).
    pub fn from_env() -> Result<Self> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let store: Arc<dyn Store> = match var("PRB_ASSETS_S3_BUCKET") {
            Some(bucket) => {
                let need = |k: &str| var(k).with_context(|| format!("PRB_ASSETS_S3_BUCKET is set, so {k} is required"));
                Arc::new(S3 {
                    endpoint: need("PRB_ASSETS_S3_ENDPOINT")?,
                    bucket,
                    access_key: need("PRB_ASSETS_S3_ACCESS_KEY")?,
                    secret_key: need("PRB_ASSETS_S3_SECRET_KEY")?,
                    region: var("PRB_ASSETS_S3_REGION").unwrap_or_else(|| "us-east-1".into()),
                    http: reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build()?,
                })
            }
            None => Arc::new(Local { dir: var("PRB_ASSETS_DIR").unwrap_or_else(|| "prb-assets".into()).into() }),
        };
        let mut a = Self::with_store(store);
        if let Some(v) = var("PRB_ASSET_MAX_BYTES") {
            let n: usize = v.parse().context("PRB_ASSET_MAX_BYTES is not a number of bytes")?;
            if n == 0 || n > MAX_MAX_BYTES {
                bail!("PRB_ASSET_MAX_BYTES must be 1 to {MAX_MAX_BYTES}");
            }
            a.max_bytes = n;
        }
        if let Some(v) = var("PRB_ASSET_GC_GRACE_SECS") {
            let n: i64 = v.parse().context("PRB_ASSET_GC_GRACE_SECS is not a number of seconds")?;
            if n < 3600 {
                bail!("PRB_ASSET_GC_GRACE_SECS must be at least 3600: a draft needs time to name an upload");
            }
            a.grace = chrono::Duration::seconds(n);
        }
        Ok(a)
    }
}

// ---- the doors ----

fn clean_name(name: Option<&str>) -> Option<String> {
    name.map(|n| n.chars().filter(|c| !c.is_control()).take(128).collect::<String>()).filter(|n| !n.trim().is_empty())
}

/// Store an upload for `site`. Refuses an empty or oversized body, anything whose bytes are not
/// one of the accepted image types, and an unsafe SVG. Uploading the same bytes again returns
/// the same id and restarts its grace period.
pub async fn upload(pool: &PgPool, assets: &Assets, site: &str, bytes: &[u8], name: Option<&str>, by: &str, mcp_call: Option<i64>) -> Result<Value, ApiError> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sites WHERE id = $1)").bind(site).fetch_one(pool).await?;
    if !exists {
        return Err(ApiError::NotFound(format!("unknown site {site:?}")));
    }
    if bytes.is_empty() {
        return Err(ApiError::BadRequest("the upload is empty".into()));
    }
    if bytes.len() > assets.max_bytes {
        return Err(ApiError::TooLarge(format!("the upload is {} bytes; the limit is {} (PRB_ASSET_MAX_BYTES)", bytes.len(), assets.max_bytes)));
    }
    let mime = sniff(bytes).ok_or_else(|| {
        ApiError::BadRequest("not an image Pratibimba accepts: png, jpeg, webp, avif, gif or svg, judged by the file's bytes, not its name".into())
    })?;
    if mime == "image/svg+xml" {
        check_svg(bytes).map_err(ApiError::BadRequest)?;
    }
    let hash = sha256_hex(bytes);
    let (w, h) = dimensions(mime, bytes).unzip();
    assets.store.put(&key(site, &hash), bytes, mime).await.map_err(ApiError::Internal)?;
    let created: DateTime<Utc> = sqlx::query_scalar(
        "INSERT INTO assets (site_id, sha256, mime, bytes, width, height, name, uploaded_by, mcp_call_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (site_id, sha256) DO UPDATE SET unreferenced_since = CASE WHEN assets.unreferenced_since IS NULL THEN NULL ELSE now() END
         RETURNING created_at",
    )
    .bind(site)
    .bind(&hash)
    .bind(mime)
    .bind(bytes.len() as i32)
    .bind(w.map(|v| v as i32))
    .bind(h.map(|v| v as i32))
    .bind(clean_name(name))
    .bind(by.chars().take(128).collect::<String>())
    .bind(mcp_call)
    .fetch_one(pool)
    .await?;
    Ok(json!({
        "id": hash, "mime": mime, "bytes": bytes.len(), "width": w, "height": h, "created_at": created,
        "path": path(site, &hash),
        "use": { "image_field": { "asset": hash, "alt": "say what it shows" }, "swap_html_src": format!("/_prb{}", path(site, &hash)) },
    }))
}

/// The site's assets, newest first.
pub async fn list(pool: &PgPool, site: &str) -> Result<Value, ApiError> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, i32, Option<i32>, Option<i32>, Option<String>, String, DateTime<Utc>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT sha256, mime, bytes, width, height, name, uploaded_by, created_at, unreferenced_since FROM assets WHERE site_id = $1 ORDER BY created_at DESC, sha256",
    )
    .bind(site)
    .fetch_all(pool)
    .await?;
    Ok(json!({ "assets": rows.into_iter().map(|(id, mime, bytes, w, h, name, by, at, unref)| json!({
        "id": id, "mime": mime, "bytes": bytes, "width": w, "height": h, "name": name, "uploaded_by": by, "created_at": at,
        "in_use": unref.is_none(), "unreferenced_since": unref, "path": path(site, &id),
    })).collect::<Vec<_>>() }))
}

/// Refuse a payload naming an asset `site` does not hold.
pub async fn check_refs(pool: &PgPool, site: &str, refs: &[String]) -> Result<(), ApiError> {
    for r in refs {
        let found: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM assets WHERE site_id = $1 AND sha256 = $2)").bind(site).bind(r).fetch_one(pool).await?;
        if !found {
            return Err(ApiError::BadRequest(format!("asset {r} is not one of site {site:?}'s uploads; upload it first (upload_asset, or the dashboard)")));
        }
    }
    Ok(())
}

/// `GET /assets/{site}/{hash}`: the bytes, cached for a year, unable to run anything.
pub async fn serve(State(state): State<Arc<AppState>>, Path((site, hash)): Path<(String, String)>, headers: HeaderMap) -> Response {
    let not_found = || (StatusCode::NOT_FOUND, "no such asset").into_response();
    if !is_hash(&hash) {
        return not_found();
    }
    let mime: Option<String> = match sqlx::query_scalar("SELECT mime FROM assets WHERE site_id = $1 AND sha256 = $2").bind(&site).bind(&hash).fetch_optional(&state.pool).await {
        Ok(m) => m,
        Err(e) => return ApiError::from(e).into_response(),
    };
    let Some(mime) = mime else { return not_found() };
    let etag = format!("\"{hash}\"");
    let common = [
        (header::CACHE_CONTROL, CACHE.to_string()),
        (header::ETAG, etag.clone()),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
        (header::CONTENT_SECURITY_POLICY, CSP.into()),
        (header::HeaderName::from_static("cross-origin-resource-policy"), "cross-origin".into()),
        (
            header::CONTENT_DISPOSITION,
            if mime == "image/svg+xml" { format!("attachment; filename=\"{hash}.svg\"") } else { "inline".into() },
        ),
    ];
    if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()).is_some_and(|v| v.split(',').any(|t| t.trim() == etag)) {
        return (StatusCode::NOT_MODIFIED, common).into_response();
    }
    match state.assets.store.get(&key(&site, &hash)).await {
        Ok(Some(bytes)) => (StatusCode::OK, common, [(header::CONTENT_TYPE, mime)], bytes).into_response(),
        Ok(None) => not_found(),
        Err(e) => ApiError::Internal(e).into_response(),
    }
}

/// What one garbage-collection pass did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Gc {
    pub marked: u64,
    pub revived: u64,
    pub deleted: Vec<(String, String)>,
}

/// A variant still in play (draft, approved or paused) of the asset's site names it: by id in
/// an image field, or by its URL (which holds the id) in swap HTML.
const REFERENCED: &str = "EXISTS (SELECT 1 FROM variants v JOIN slots s ON s.id = v.slot_id
     WHERE s.site_id = a.site_id AND v.status IN ('draft', 'approved', 'paused') AND strpos(v.payload, a.sha256) > 0)";

/// Mark assets no variant references, unmark those referenced again, and delete those
/// unreferenced for longer than the grace period (row first, in the statement that re-checks
/// the reference, then the bytes).
pub async fn gc(pool: &PgPool, assets: &Assets, now: DateTime<Utc>) -> Result<Gc> {
    let revived = sqlx::query(&format!("UPDATE assets a SET unreferenced_since = NULL WHERE unreferenced_since IS NOT NULL AND {REFERENCED}"))
        .execute(pool)
        .await?
        .rows_affected();
    let marked = sqlx::query(&format!("UPDATE assets a SET unreferenced_since = $1 WHERE unreferenced_since IS NULL AND NOT {REFERENCED}"))
        .bind(now)
        .execute(pool)
        .await?
        .rows_affected();
    let deleted: Vec<(String, String)> =
        sqlx::query_as(&format!("DELETE FROM assets a WHERE unreferenced_since <= $1 AND NOT {REFERENCED} RETURNING site_id, sha256"))
            .bind(now - assets.grace)
            .fetch_all(pool)
            .await?;
    for (site, hash) in &deleted {
        if let Err(e) = assets.store.delete(&key(site, hash)).await {
            tracing::error!(error = %e, site, hash, "asset bytes not deleted; the row is gone, so it is no longer served");
        }
    }
    Ok(Gc { marked, revived, deleted })
}
