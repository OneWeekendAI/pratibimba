//! The edge snapshot (edge-cache.md, phase EC1): a versioned per-site JSON document holding
//! everything `/decide` reads that is the same for every visitor, pushed to an edge store
//! (Cloudflare Workers KV) whenever it changes, so a Worker in front of Core can read it.
//!
//! **Pluggable and off by default.** With no store configured ([`Edge::off`]) nothing here runs:
//! no snapshot is built, no row is written, so a self-hosted Core is unaffected. Configure the
//! Cloudflare KV store with all three of `PRB_EDGE_KV_ACCOUNT`, `PRB_EDGE_KV_NAMESPACE` and
//! `PRB_EDGE_KV_TOKEN` (`PRB_EDGE_KV_API` overrides the API base, for tests). Setting only some
//! of them is an error, not a silent "off".
//!
//! **What is in it.** The slot part of each live slot is [`crate::http::decide`]'s own read
//! (`shared_inputs`, which runs decide's `SNAPSHOT` statement), so the snapshot cannot hold
//! anything decide does not read: approved arms with payloads and shares, audience gates,
//! the running experiment, the model state and version, cold-start counts, the site's
//! privacy mode, holdout fraction, kill switch and night window. Plus the manifest
//! (`/manifest/:site`) and the installed plugins (name and kind only, never their config).
//! G5: `erased`, a Bloom filter of the site's erasure hashes ([`erased_filter`]), so the Worker
//! skips the `gone/` read for a subject it rules out; an erase republishes the snapshot.
//! Left out on purpose: the per-visitor rows (sticky decision, latest picks), the clock, and the
//! cookieless salt (whether a salt may leave Core is EC2's decision).
//!
//! **When it is pushed.** Every handler that changes one of those inputs calls [`changed`]
//! after its commit (approve, retire, kill switch, privacy settings, audiences, experiments,
//! plugins, model rollback, `apply`), and every worker pass calls [`publish_all`], which covers
//! what the worker itself changes (learned model versions, cold-start counts, guard pauses) and
//! anything a clock changes (an experiment's start or end). A push happens only when the
//! content's hash differs from the last one published, and `version` rises by one each time.
//! A failed push changes nothing in the database, so the next trigger or worker pass retries it.

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};

/// The snapshot's format; bump when its shape changes incompatibly.
/// 2 (EC2): adds `settings.simulated_clock` and, per slot, `decision_ttl_secs`, the running
/// experiment's `ends_at` and `next_experiment_at`, which the edge needs to decide on its own clock.
pub const FORMAT: i64 = 2;

/// The key a site's snapshot is stored under.
pub fn key(site: &str) -> String {
    format!("site/{site}")
}

/// Where snapshots go. Implementations must be idempotent: the same `(key, body)` may be put twice.
pub trait Store: Send + Sync {
    fn put<'a>(&'a self, key: &'a str, body: Vec<u8>) -> futures_util::future::BoxFuture<'a, Result<()>>;
}

/// The edge publisher Core runs with: off, or a store; and, for EC2's edge decide, the token the
/// Worker presents.
#[derive(Clone, Default)]
pub struct Edge {
    store: Option<Arc<dyn Store>>,
    /// `PRB_EDGE_TOKEN`: the shared secret between Core and the edge Worker. With it Core accepts
    /// edge-made decisions (`POST /edge/decisions`) and tells the Worker the stored subject of the
    /// decisions it makes (see [`crate::http::decide`]). `None` turns both off.
    token: Option<String>,
    /// EC4: where erasures' tombstone hashes go (KV only: the beacon holds snapshots, not keys).
    tombstones: Option<Arc<dyn Store>>,
}

impl Edge {
    /// No edge: nothing is built or written.
    pub fn off() -> Self {
        Edge { store: None, token: None, tombstones: None }
    }

    /// Snapshots and tombstones both to `store` (tests use one in-memory store for both).
    pub fn with_store(store: Arc<dyn Store>) -> Self {
        Edge { store: Some(store.clone()), token: None, tombstones: Some(store) }
    }

    /// Whether erasures are pushed to the edge (EC4).
    pub fn has_tombstones(&self) -> bool {
        self.tombstones.is_some()
    }

    /// This edge with the Worker's shared token set.
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into()).filter(|t: &String| !t.is_empty());
        self
    }

    pub fn is_on(&self) -> bool {
        self.store.is_some()
    }

    /// Whether `presented` is the edge token. Constant-time in the token's bytes; false when no
    /// token is configured.
    pub fn token_matches(&self, presented: Option<&str>) -> bool {
        match (&self.token, presented) {
            (Some(t), Some(p)) if t.len() == p.len() => t.bytes().zip(p.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0,
            _ => false,
        }
    }

    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    /// Read the configuration from the process environment.
    pub fn from_env() -> Result<Self> {
        Self::from_vars(|k| std::env::var(k).ok())
    }

    /// Read the configuration through `get` (the environment, or a map in tests). All three of
    /// account, namespace and token, or none of them.
    ///
    /// EC2 adds two more: `PRB_EDGE_TOKEN` (see [`Edge::token`]) and `PRB_EDGE_BEACON_URL`, the
    /// Worker's strongly consistent version beacon, which then gets every snapshot too (see
    /// [`Beacon`]). The beacon needs the token.
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let val = |k: &str| get(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let (account, namespace, token) = (val("PRB_EDGE_KV_ACCOUNT"), val("PRB_EDGE_KV_NAMESPACE"), val("PRB_EDGE_KV_TOKEN"));
        let mut stores: Vec<Arc<dyn Store>> = Vec::new();
        let mut tombstones: Option<Arc<dyn Store>> = None;
        match (account, namespace, token) {
            (None, None, None) => {}
            (Some(account), Some(namespace), Some(token)) => {
                let api = val("PRB_EDGE_KV_API").unwrap_or_else(|| "https://api.cloudflare.com/client/v4".into());
                let kv: Arc<dyn Store> = Arc::new(CloudflareKv { api, account, namespace, token, http: reqwest::Client::new() });
                tombstones = Some(kv.clone());
                stores.push(kv);
            }
            _ => bail!("set all of PRB_EDGE_KV_ACCOUNT, PRB_EDGE_KV_NAMESPACE and PRB_EDGE_KV_TOKEN, or none of them"),
        }
        let edge_token = val("PRB_EDGE_TOKEN");
        if let Some(url) = val("PRB_EDGE_BEACON_URL") {
            let Some(token) = edge_token.clone() else {
                bail!("PRB_EDGE_BEACON_URL needs PRB_EDGE_TOKEN");
            };
            stores.push(Arc::new(Beacon { url, token, http: reqwest::Client::new() }));
        }
        let store: Option<Arc<dyn Store>> = match stores.len() {
            0 => None,
            1 => stores.pop(),
            _ => Some(Arc::new(Fanout(stores))),
        };
        Ok(Edge { store, token: edge_token, tombstones })
    }
}

/// Several stores, each put in turn; the put fails when any of them fails, so the next pass
/// retries all of them (each put is idempotent).
pub struct Fanout(pub Vec<Arc<dyn Store>>);

impl Store for Fanout {
    fn put<'a>(&'a self, key: &'a str, body: Vec<u8>) -> futures_util::future::BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            for s in &self.0 {
                s.put(key, body.clone()).await?;
            }
            Ok(())
        })
    }
}

/// EC2's version beacon: the edge Worker's `POST /_prb/edge/snapshot`, which keeps each site's
/// newest snapshot in a Durable Object. KV can take a minute to show a new value at a PoP
/// (EC1 measured 15 s and 51 s); a Durable Object is read where it lives, so it is current the
/// moment the put returns. The Worker checks the beacon's version every few seconds and decides
/// at the edge only from a snapshot of exactly that version (edge-cache.md, Decisions 6).
pub struct Beacon {
    url: String,
    token: String,
    http: reqwest::Client,
}

impl Store for Beacon {
    fn put<'a>(&'a self, _key: &'a str, body: Vec<u8>) -> futures_util::future::BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let res = self
                .http
                .post(&self.url)
                .bearer_auth(&self.token)
                .header("content-type", "application/json")
                .body(body)
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await
                .context("edge beacon unreachable")?;
            let status = res.status();
            if !status.is_success() {
                let text = res.text().await.unwrap_or_default();
                bail!("edge beacon answered {status}: {}", text.chars().take(300).collect::<String>());
            }
            Ok(())
        })
    }
}

/// Cloudflare Workers KV through the REST API: `PUT .../storage/kv/namespaces/:ns/values/:key`.
pub struct CloudflareKv {
    api: String,
    account: String,
    namespace: String,
    token: String,
    http: reqwest::Client,
}

impl Store for CloudflareKv {
    fn put<'a>(&'a self, key: &'a str, body: Vec<u8>) -> futures_util::future::BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // The key is `site/<id>`: escape the slash, since the key is one path segment.
            let url = format!(
                "{}/accounts/{}/storage/kv/namespaces/{}/values/{}",
                self.api.trim_end_matches('/'),
                self.account,
                self.namespace,
                key.replace('%', "%25").replace('/', "%2F")
            );
            let res = self
                .http
                .put(url)
                .bearer_auth(&self.token)
                .header("content-type", "application/json")
                .body(body)
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await
                .context("edge store unreachable")?;
            let status = res.status();
            if !status.is_success() {
                let text = res.text().await.unwrap_or_default();
                bail!("edge store answered {status}: {}", text.chars().take(300).collect::<String>());
            }
            Ok(())
        })
    }
}

/// An in-memory store, for tests: every put, in order.
#[derive(Default)]
pub struct MemoryStore {
    pub puts: Mutex<Vec<(String, Value)>>,
}

impl MemoryStore {
    /// The last snapshot put for `site`.
    pub fn last(&self, site: &str) -> Option<Value> {
        self.puts.lock().unwrap().iter().rev().find(|(k, _)| *k == key(site)).map(|(_, v)| v.clone())
    }

    pub fn count(&self, site: &str) -> usize {
        self.puts.lock().unwrap().iter().filter(|(k, _)| *k == key(site)).count()
    }
}

impl Store for MemoryStore {
    fn put<'a>(&'a self, key: &'a str, body: Vec<u8>) -> futures_util::future::BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let v: Value = serde_json::from_slice(&body)?;
            self.puts.lock().unwrap().push((key.to_string(), v));
            Ok(())
        })
    }
}

/// The snapshot's content for `site`: everything but `version` and `generated_at`. `None` for an
/// unknown site. Deterministic (serde_json maps are sorted), so its hash says whether it changed.
pub async fn content(pool: &PgPool, site: &str) -> Result<Option<Value>> {
    let mut conn = pool.acquire().await?;
    // The live slots: those with an approved arm, as in the manifest. A draft slot decides nothing.
    let slots: Vec<String> = sqlx::query_scalar(
        "SELECT s.key FROM slots s WHERE s.site_id = $1 AND s.kind NOT IN ('sorted', 'copy') AND EXISTS (SELECT 1 FROM variants v WHERE v.slot_id = s.id AND v.status = 'approved') ORDER BY s.key",
    )
    .bind(site)
    .fetch_all(&mut *conn)
    .await?;
    // Read with no slot first, so a site with no live slot still has its settings.
    let Some((settings, _)) = crate::http::decide::shared_inputs(&mut conn, site, "").await.map_err(api_err)? else {
        return Ok(None);
    };
    let mut settings = settings;
    let (simulated, history): (bool, Option<Value>) =
        sqlx::query_as("SELECT sim_now IS NOT NULL, history FROM sites WHERE id = $1").bind(site).fetch_one(&mut *conn).await?;
    settings["simulated_clock"] = json!(simulated);
    // F3: the site's `history:` block (null without one). The Worker reads no history, so it sends
    // an identified request on such a site to Core (deploy/proxy-worker/src/index.js readsHistory).
    settings["history"] = history.unwrap_or(Value::Null);
    let mut by_key = Map::new();
    for slot in slots {
        if let Some((_, mut s)) = crate::http::decide::shared_inputs(&mut conn, site, &slot).await.map_err(api_err)?
            && !s.is_null()
        {
            // What the edge needs to decide on its own clock, which decide reads from the clock
            // it shares with the database: how long a decision stays sticky, when the running
            // experiment ends, and when the next one starts (the edge asks Core from then on).
            let (ttl, ends, next): (i32, Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
                "SELECT s.decision_ttl_secs,
                        (SELECT least(x.stops_at, coalesce(x.stopped_at, 'infinity')) FROM experiments x WHERE x.id = $3),
                        (SELECT min(x.starts_at) FROM experiments x, sites t
                          WHERE x.slot_id = s.id AND t.id = s.site_id AND x.starts_at > COALESCE(t.sim_now, now()) AND x.stopped_at IS NULL)
                   FROM slots s WHERE s.site_id = $1 AND s.key = $2",
            )
            .bind(site)
            .bind(&slot)
            .bind(s["experiment"]["id"].as_i64())
            .fetch_one(&mut *conn)
            .await?;
            let stamp = |t: Option<chrono::DateTime<chrono::Utc>>| t.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
            s["decision_ttl_secs"] = json!(ttl);
            s["next_experiment_at"] = json!(stamp(next));
            if s["experiment"].is_object() {
                s["experiment"]["ends_at"] = json!(stamp(ends));
            }
            by_key.insert(slot, s);
        }
    }
    drop(conn);
    let plugins: Vec<(String, String, bool, Option<String>)> =
        sqlx::query_as("SELECT name, kind, deanonymiser, attributor FROM plugins WHERE site_id = $1 ORDER BY name").bind(site).fetch_all(pool).await?;
    let plugins: Vec<Value> = plugins
        .into_iter()
        .map(|(name, kind, deanonymiser, attributor)| json!({ "name": name, "kind": kind, "deanonymiser": deanonymiser, "attributor": attributor }))
        .collect();
    // G5 (edge-cache.md, Decision 22): every erasure of the site, as a Bloom filter, so the
    // Worker needs no `gone/` read for a subject it says is not erased.
    let hashes: Vec<String> =
        sqlx::query_scalar("SELECT subject_hash FROM erasures WHERE site_id = $1 ORDER BY subject_hash").bind(site).fetch_all(pool).await?;
    let manifest = match crate::http::manifest::load(pool, site).await {
        Ok(m) => serde_json::to_value(m)?,
        Err(e) => return Err(api_err(e)),
    };
    Ok(Some(json!({
        "format": FORMAT,
        "site": site,
        "settings": settings,
        "slots": by_key,
        "plugins": plugins,
        "manifest": manifest,
        "erased": erased_filter(&hashes),
    })))
}

/// Bits per erasure in [`erased_filter`]: with [`ERASED_FILTER_K`] = 7, about 0.8 % false positives.
pub const ERASED_FILTER_BITS_PER: usize = 10;
pub const ERASED_FILTER_K: u64 = 7;
/// The filter never grows past 64 KiB (about 52,000 erasures at 1 %); past that its
/// false-positive rate climbs, which only costs the Worker a `gone/` read. Never a false negative.
pub const ERASED_FILTER_MAX_BITS: usize = 64 * 1024 * 8;

/// A Bloom filter over `hashes` ([`crate::privacy::subject_hash`] hex, as the `gone/` keys):
/// `{n, m, k, bits}`, `bits` base64 with bit `i` at byte `i/8`, bit `i%8` (least significant
/// first). Index `j < k` of a hash is `(h1 + j*h2) mod m`, where `h1`, `h2` are the hash's first two
/// big-endian 32-bit words, `h2` with its low bit set. The Worker's `inErased` reads the same.
pub fn erased_filter(hashes: &[String]) -> Value {
    use base64::Engine;
    let n = hashes.len();
    let m = if n == 0 { 0 } else { (n * ERASED_FILTER_BITS_PER).div_ceil(8).max(8).min(ERASED_FILTER_MAX_BITS / 8) * 8 };
    let mut bits = vec![0u8; m / 8];
    for h in hashes {
        let Some((h1, h2)) = filter_words(h) else {
            continue;
        };
        for j in 0..ERASED_FILTER_K {
            let idx = ((h1 + j * h2) % m as u64) as usize;
            bits[idx >> 3] |= 1 << (idx & 7);
        }
    }
    if n * ERASED_FILTER_BITS_PER > ERASED_FILTER_MAX_BITS {
        tracing::warn!(erasures = n, "edge erased filter is at its cap; its false-positive rate is rising");
    }
    json!({ "n": n, "m": m, "k": ERASED_FILTER_K, "bits": base64::engine::general_purpose::STANDARD.encode(&bits) })
}

fn filter_words(hash: &str) -> Option<(u64, u64)> {
    let word = |at: usize| hash.get(at..at + 8).and_then(|w| u32::from_str_radix(w, 16).ok());
    Some((word(0)? as u64, (word(8)? | 1) as u64))
}

fn api_err(e: crate::http::ApiError) -> anyhow::Error {
    anyhow::anyhow!("{e:?}")
}

/// One publish: the version written and the body's size.
#[derive(Debug, Clone, PartialEq)]
pub struct Published {
    pub version: i64,
    pub bytes: usize,
}

/// Build `site`'s snapshot and put it when its content changed since the last publish.
/// `Ok(None)` when the edge is off, the site is unknown, or nothing changed. Publishes for one
/// site are serialised by an advisory lock, so two concurrent publishes cannot land out of order.
pub async fn publish(pool: &PgPool, edge: &Edge, site: &str) -> Result<Option<Published>> {
    let Some(store) = &edge.store else {
        return Ok(None);
    };
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('prb-edge:' || $1, 0))").bind(site).execute(&mut *tx).await?;
    let Some(mut body) = content(pool, site).await? else {
        return Ok(None);
    };
    let hash = hex(&Sha256::digest(serde_json::to_vec(&body)?));
    let last: Option<(i64, String)> =
        sqlx::query_as("SELECT version, content_hash FROM edge_snapshots WHERE site_id = $1").bind(site).fetch_optional(&mut *tx).await?;
    if last.as_ref().is_some_and(|(_, h)| *h == hash) {
        return Ok(None);
    }
    let version = last.map_or(1, |(v, _)| v + 1);
    let now: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("SELECT now()").fetch_one(&mut *tx).await?;
    let o = body.as_object_mut().expect("content is an object");
    o.insert("version".into(), json!(version));
    o.insert("generated_at".into(), json!(now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)));
    o.insert("content_hash".into(), json!(hash));
    let bytes = serde_json::to_vec(&body)?;
    let size = bytes.len();
    store.put(&key(site), bytes).await.with_context(|| format!("edge snapshot for {site:?}"))?;
    sqlx::query(
        "INSERT INTO edge_snapshots (site_id, version, content_hash, bytes, published_at) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (site_id) DO UPDATE SET version = EXCLUDED.version, content_hash = EXCLUDED.content_hash,
                                             bytes = EXCLUDED.bytes, published_at = EXCLUDED.published_at",
    )
    .bind(site)
    .bind(version)
    .bind(&hash)
    .bind(size as i32)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::info!(site, version, bytes = size, "edge snapshot published");
    Ok(Some(Published { version, bytes: size }))
}

/// After a handler's commit: publish `site`, and never fail the request over it. A failure is
/// logged; the next worker pass retries, since the stored hash did not move.
pub async fn changed(pool: &PgPool, edge: &Edge, site: &str) {
    if let Err(e) = publish(pool, edge, site).await {
        tracing::error!(site, error = %format!("{e:#}"), "edge snapshot publish failed");
    }
}

/// [`changed`] for the site a row belongs to: `sql` takes the id as `$1` and returns the site id.
pub async fn changed_by(pool: &PgPool, edge: &Edge, sql: &str, id: i64) {
    if !edge.is_on() {
        return;
    }
    match sqlx::query_scalar::<_, String>(sql).bind(id).fetch_optional(pool).await {
        Ok(Some(site)) => changed(pool, edge, &site).await,
        Ok(None) => {}
        Err(e) => tracing::error!(error = %e, "edge: finding the site failed"),
    }
}

/// Every real site (never a sandbox): what a worker pass calls. Returns how many were pushed.
pub async fn publish_all(pool: &PgPool, edge: &Edge) -> Result<usize> {
    if !edge.is_on() {
        return Ok(0);
    }
    let sites: Vec<String> = sqlx::query_scalar("SELECT id FROM sites WHERE sandbox_of IS NULL ORDER BY id").fetch_all(pool).await?;
    let mut n = 0;
    for site in sites {
        match publish(pool, edge, &site).await {
            Ok(Some(_)) => n += 1,
            Ok(None) => {}
            Err(e) => tracing::error!(site, error = %format!("{e:#}"), "edge snapshot publish failed"),
        }
    }
    Ok(n)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ---- EC4: erased subjects at the edge ----

/// How long an erase takes to be refused at every edge PoP: the KV read the Worker makes is
/// cached for 30 s (`cacheTtl`), and KV propagates a write in well under that (measured live, see
/// the tracker's EC4), so 60 s is the bound the erase answer states (edge-cache.md, Decisions 1).
pub const ERASE_EDGE_WINDOW_SECS: i64 = 60;

/// The KV key a tombstone is pushed under: `gone/<site>/<subject_hash>`, where the hash is
/// [`crate::privacy::subject_hash`] of the client's id (the Worker computes the same from the
/// raw subject, whatever the privacy mode, so no salt is needed).
pub fn tombstone_key(site: &str, subject_hash: &str) -> String {
    format!("gone/{site}/{subject_hash}")
}

async fn push_rows(pool: &PgPool, store: &Arc<dyn Store>, rows: Vec<(i64, String, String, chrono::DateTime<chrono::Utc>)>) -> Result<usize> {
    let mut n = 0;
    let mut failed = None;
    for (id, site, hash, at) in rows {
        let body = serde_json::to_vec(&json!({ "site": site, "erased_at": at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true) }))?;
        match store.put(&tombstone_key(&site, &hash), body).await {
            Ok(()) => {
                sqlx::query("UPDATE erasures SET edge_pushed_at = now() WHERE id = $1").bind(id).execute(pool).await?;
                n += 1;
            }
            Err(e) => {
                tracing::error!(site, erasure = id, error = %format!("{e:#}"), "edge tombstone push failed");
                failed = Some(e);
            }
        }
    }
    match failed {
        Some(e) if n == 0 => Err(e),
        _ => Ok(n),
    }
}

/// Push one erasure's tombstone, right after the erase commits. `Ok(true)` when it is at the edge
/// store now; `Ok(false)` when there is no tombstone store. A failure leaves it pending, and the
/// next worker pass ([`push_tombstones`]) retries.
pub async fn push_tombstone(pool: &PgPool, edge: &Edge, erasure: i64) -> Result<bool> {
    let Some(store) = &edge.tombstones else { return Ok(false) };
    let rows = sqlx::query_as("SELECT id, site_id, subject_hash, created_at FROM erasures WHERE id = $1").bind(erasure).fetch_all(pool).await?;
    Ok(push_rows(pool, store, rows).await? == 1)
}

/// Every erasure not yet at the edge store (a failed push, or one made before the edge was
/// configured, or by the CLI), at most 500 per call: what each worker pass runs.
pub async fn push_tombstones(pool: &PgPool, edge: &Edge) -> Result<usize> {
    let Some(store) = &edge.tombstones else { return Ok(0) };
    let rows = sqlx::query_as("SELECT id, site_id, subject_hash, created_at FROM erasures WHERE edge_pushed_at IS NULL ORDER BY id LIMIT 500")
        .fetch_all(pool)
        .await?;
    push_rows(pool, store, rows).await
}

/// What an erase answer says about the edge: `null` with no edge configured (the erase is
/// complete when it returns), else whether the tombstone is at the edge store and the window.
pub async fn erased(pool: &PgPool, edge: &Edge, erasure: i64) -> Value {
    if !edge.is_on() && !edge.has_token() {
        return Value::Null;
    }
    let w = ERASE_EDGE_WINDOW_SECS;
    // G5: the snapshot's erased filter now holds the subject, so republish it (a failure is
    // logged and the worker pass retries, since the content hash moved).
    if edge.is_on() {
        changed_by(pool, edge, "SELECT site_id FROM erasures WHERE id = $1", erasure).await;
    }
    match push_tombstone(pool, edge, erasure).await {
        Ok(true) => json!({ "tombstone": "pushed", "effective_within_secs": w, "note": format!("effective at the edge within {w} s") }),
        Ok(false) => json!({ "tombstone": "unconfigured", "effective_within_secs": null,
                             "note": "no edge tombstone store is configured (PRB_EDGE_KV_*): the edge is not told; Core refuses every write" }),
        Err(e) => {
            tracing::error!(erasure, error = %format!("{e:#}"), "edge tombstone push failed; the worker retries");
            json!({ "tombstone": "pending", "effective_within_secs": w,
                    "note": format!("the edge push failed and the next worker pass retries it; effective at the edge within {w} s of that pass") })
        }
    }
}

#[cfg(test)]
mod erased_filter_tests {
    use super::*;

    fn h(site: &str, s: &str) -> String {
        crate::privacy::subject_hash(site, s)
    }

    fn member(f: &Value, hash: &str) -> bool {
        use base64::Engine;
        let m = f["m"].as_u64().unwrap();
        let k = f["k"].as_u64().unwrap();
        let bits = base64::engine::general_purpose::STANDARD.decode(f["bits"].as_str().unwrap()).unwrap();
        if m == 0 {
            return false;
        }
        let r: Vec<u8> = (0..8).map(|i| u8::from_str_radix(&hash[2 * i..2 * i + 2], 16).unwrap()).collect();
        let h1 = u32::from_be_bytes([r[0], r[1], r[2], r[3]]) as u64;
        let h2 = (u32::from_be_bytes([r[4], r[5], r[6], r[7]]) | 1) as u64;
        (0..k).all(|i| {
            let idx = (h1 + i * h2) % m;
            bits[(idx >> 3) as usize] >> (idx & 7) & 1 == 1
        })
    }

    #[test]
    fn an_empty_site_has_an_empty_filter() {
        assert_eq!(erased_filter(&[]), json!({ "n": 0, "m": 0, "k": 7, "bits": "" }));
    }

    #[test]
    fn one_erasure_sets_the_pinned_bits() {
        // Pinned in the Worker's test too (deploy/proxy-worker/test/erased.test.mjs).
        assert_eq!(erased_filter(&[h("demo", "alice")]), json!({ "n": 1, "m": 64, "k": 7, "bits": "BQhAAAIQgAA=" }));
    }

    #[test]
    fn forty_erasures_set_the_pinned_bits() {
        // demo's s0..s39: 16 of them have an even second word, so `h2 | 1` matters. Pinned in
        // the Worker's test too.
        let hs: Vec<String> = (0..40).map(|i| h("demo", &format!("s{i}"))).collect();
        assert_eq!(erased_filter(&hs), json!({ "n": 40, "m": 400, "k": 7, "bits": "9p5OHKP/35GFV+jWjkbmGIU9xMlstZ+iOaEt/GHbzxky93pxGzq3Un2Ex31lsGhEFCM=" }));
    }

    #[test]
    fn the_filter_is_sized_at_ten_bits_an_erasure_and_capped_at_64_kib() {
        let many = |n: usize| (0..n).map(|i| h("demo", &format!("s{i}"))).collect::<Vec<_>>();
        assert_eq!(erased_filter(&many(2))["m"], json!(64), "below the floor");
        assert_eq!(erased_filter(&many(1000))["m"], json!(10_000));
        assert_eq!(erased_filter(&many(1001))["m"], json!(10_016), "rounded up to whole bytes");
        let big = erased_filter(&many(60_000));
        assert_eq!(big["m"], json!(ERASED_FILTER_MAX_BITS), "not capped");
        assert_eq!(ERASED_FILTER_MAX_BITS, 64 * 1024 * 8);
        assert_eq!(big["n"], json!(60_000));
    }

    #[test]
    fn every_erasure_is_a_member_and_few_others_are() {
        let erased: Vec<String> = (0..5000).map(|i| h("demo", &format!("gone-{i}"))).collect();
        let f = erased_filter(&erased);
        for e in &erased {
            assert!(member(&f, e), "a false negative: {e}");
        }
        let fp = (0..20_000).filter(|i| member(&f, &h("demo", &format!("live-{i}")))).count();
        assert!(fp < 400, "{fp} false positives in 20000 (want about 0.8 %)");
        assert!(fp > 0, "no false positive at all: the filter is not a Bloom filter at 10 bits");
    }

    #[test]
    fn past_the_cap_there_are_still_no_false_negatives() {
        let erased: Vec<String> = (0..70_000).map(|i| h("demo", &format!("x{i}"))).collect();
        let f = erased_filter(&erased);
        assert!(erased.iter().all(|e| member(&f, e)));
    }
}
