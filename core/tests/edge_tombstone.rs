//! EC4, erased subjects at the edge (edge-cache.md, Decisions 1): on erase, Core pushes the
//! subject's tombstone hash to the edge store (KV key `gone/<site>/<subject_hash>`), so the
//! Worker refuses the subject; the erase answer says "effective at the edge within 60 s". A push
//! that fails, or an erasure made with no edge (the CLI, or before the edge was configured), is
//! pushed by the next worker pass. A Core with no edge pushes nothing and says so (`edge: null`).

mod common;
use common::*;
use pratibimba::edge::{Edge, MemoryStore, Store};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;

/// sha256("demo" 0x00 "alice"): pinned here and in the Worker's test (test/tombstone.test.mjs),
/// so Core's key and the key the Worker reads cannot drift apart.
const ALICE_HASH: &str = "4f5340822780b7f58a12a3cca2625041ac91589f3d336350d2f02154a2d583a1";

fn app_with(pool: &PgPool, edge: Edge) -> axum::Router {
    pratibimba::http::router(pratibimba::http::AppState::configured_with_edge(pool.clone(), Some(7), Some(TOKEN.into()), None, edge), &[])
}

async fn erase(app: &axum::Router, subject: &str) -> Value {
    let r = api_post(app, "/api/sites/demo/erase", json!({ "subject": subject, "actor": "owner" })).await;
    assert!(r.status.is_success(), "{} {}", r.status, r.text);
    r.json()
}

fn gone_puts(store: &MemoryStore) -> Vec<(String, Value)> {
    store.puts.lock().unwrap().iter().filter(|(k, _)| k.starts_with("gone/")).cloned().collect()
}

#[sqlx::test(migrations = false)]
async fn an_erase_pushes_the_tombstone_and_says_the_edge_window(pool: PgPool) {
    setup_demo(&pool).await;
    let store = Arc::new(MemoryStore::default());
    let app = app_with(&pool, Edge::with_store(store.clone()));
    let out = erase(&app, "alice").await;
    let puts = gone_puts(&store);
    assert_eq!(puts.len(), 1, "the erase pushed no tombstone (or more than one): {puts:?}");
    assert_eq!(puts[0].0, format!("gone/demo/{ALICE_HASH}"), "the key is not gone/<site>/sha256(site 0 subject)");
    assert_eq!(puts[0].1["site"], json!("demo"));
    assert_eq!(out["edge"]["tombstone"], json!("pushed"));
    assert_eq!(out["edge"]["effective_within_secs"], json!(60));
    assert_eq!(out["edge"]["note"], json!("effective at the edge within 60 s"));
    assert_eq!(count(&pool, "SELECT count(*) FROM erasures WHERE edge_pushed_at IS NOT NULL").await, 1, "the push was not recorded");
    // Nothing pending, so a worker pass pushes it no second time.
    pratibimba::worker::run_drain_edge(&pool, &Edge::with_store(store.clone())).await.unwrap();
    assert_eq!(gone_puts(&store).len(), 1, "an already-pushed tombstone was pushed again");
}

#[sqlx::test(migrations = false)]
async fn a_core_without_an_edge_pushes_nothing_and_says_so(pool: PgPool) {
    setup_demo(&pool).await;
    let out = erase(&admin_app(&pool), "alice").await;
    assert_eq!(out["edge"], Value::Null, "a Core with no edge claimed an edge window");
    assert_eq!(count(&pool, "SELECT count(*) FROM erasures WHERE edge_pushed_at IS NULL").await, 1);
}

/// A store that fails every put until `up` is set.
#[derive(Default)]
struct Down {
    up: std::sync::atomic::AtomicBool,
    inner: MemoryStore,
}

impl Store for Down {
    fn put<'a>(&'a self, key: &'a str, body: Vec<u8>) -> futures_util::future::BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            if !self.up.load(std::sync::atomic::Ordering::SeqCst) {
                anyhow::bail!("KV is down");
            }
            self.inner.put(key, body).await
        })
    }
}

#[sqlx::test(migrations = false)]
async fn a_failed_push_still_erases_and_the_worker_pass_retries_it(pool: PgPool) {
    setup_demo(&pool).await;
    let down = Arc::new(Down::default());
    let app = app_with(&pool, Edge::with_store(down.clone()));
    let out = erase(&app, "alice").await;
    assert_eq!(out["edge"]["tombstone"], json!("pending"), "a failed push was reported as pushed");
    assert_eq!(count(&pool, "SELECT count(*) FROM erasures").await, 1, "the erase itself must not fail over the edge");
    assert_eq!(count(&pool, "SELECT count(*) FROM erasures WHERE edge_pushed_at IS NOT NULL").await, 0);
    down.up.store(true, std::sync::atomic::Ordering::SeqCst);
    pratibimba::worker::run_drain_edge(&pool, &Edge::with_store(down.clone())).await.unwrap();
    assert_eq!(gone_puts(&down.inner).iter().map(|p| p.0.clone()).collect::<Vec<_>>(), [format!("gone/demo/{ALICE_HASH}")], "the worker pass did not retry");
    assert_eq!(count(&pool, "SELECT count(*) FROM erasures WHERE edge_pushed_at IS NOT NULL").await, 1);
}

/// An erasure made with no edge at all (the CLI, or before the edge was configured) reaches the
/// edge at the first worker pass that has one.
#[sqlx::test(migrations = false)]
async fn erasures_made_without_an_edge_are_pushed_by_the_next_pass(pool: PgPool) {
    setup_demo(&pool).await;
    pratibimba::privacy::erase(&pool, "demo", "alice", "owner", None).await.unwrap();
    pratibimba::privacy::erase(&pool, "demo", "bob", "owner", None).await.unwrap();
    let store = Arc::new(MemoryStore::default());
    pratibimba::worker::run_drain_edge(&pool, &Edge::with_store(store.clone())).await.unwrap();
    let mut keys: Vec<String> = gone_puts(&store).into_iter().map(|p| p.0).collect();
    keys.sort();
    let mut want = vec![format!("gone/demo/{ALICE_HASH}"), format!("gone/demo/{}", pratibimba::privacy::subject_hash("demo", "bob"))];
    want.sort();
    assert_eq!(keys, want);
}

/// With KV and the beacon both configured (production), the tombstone goes to KV, never to the
/// beacon (which holds snapshots: a tombstone there would be refused, failing the push forever).
#[sqlx::test(migrations = false)]
async fn the_tombstone_goes_to_kv_not_the_beacon(pool: PgPool) {
    setup_demo(&pool).await;
    type Seen = Arc<std::sync::Mutex<Vec<(String, String)>>>;
    let seen: Seen = Default::default();
    let s2 = seen.clone();
    let mock = axum::Router::new().fallback(move |req: axum::extract::Request| {
        let seen = s2.clone();
        async move {
            seen.lock().unwrap().push((req.method().to_string(), req.uri().path().to_string()));
            axum::Json(json!({ "success": true }))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let vars = [
        ("PRB_EDGE_KV_ACCOUNT", "acct".to_string()),
        ("PRB_EDGE_KV_NAMESPACE", "ns".to_string()),
        ("PRB_EDGE_KV_TOKEN", "tok".to_string()),
        ("PRB_EDGE_KV_API", format!("http://{addr}/client/v4")),
        ("PRB_EDGE_TOKEN", "edge".to_string()),
        ("PRB_EDGE_BEACON_URL", format!("http://{addr}/_prb/edge/snapshot")),
    ];
    let edge = Edge::from_vars(|k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone())).unwrap();
    let out = erase(&app_with(&pool, edge), "alice").await;
    assert_eq!(out["edge"]["tombstone"], json!("pushed"), "{out}");
    let seen = seen.lock().unwrap().clone();
    let gone: Vec<_> = seen.iter().filter(|(_, p)| p.contains("gone")).cloned().collect();
    assert_eq!(gone, [("PUT".to_string(), format!("/client/v4/accounts/acct/storage/kv/namespaces/ns/values/gone%2Fdemo%2F{ALICE_HASH}"))]);
    // G5: the erase also republishes the snapshot (KV and beacon both); the beacon gets nothing else.
    let kv_snapshots = seen.iter().filter(|(_, p)| p.ends_with("/values/site%2Fdemo")).count();
    let beacon: Vec<_> = seen.iter().filter(|(_, p)| p == "/_prb/edge/snapshot").collect();
    assert_eq!(seen.len(), 1 + kv_snapshots + beacon.len(), "an unexpected request: {seen:?}");
    assert_eq!(beacon.len(), kv_snapshots, "the beacon got something other than the snapshot: {seen:?}");
}

// ---- G5 (edge-cache.md, Decision 22): the erased set rides in the snapshot ----

/// The Bloom filter of `demo`'s erasures with alice alone (pinned in the unit test and in the
/// Worker's test/erased.test.mjs), and with alice and bob.
const ALICE_BITS: &str = "BQhAAAIQgAA=";
const ALICE_BOB_BITS: &str = "BQhAAAIwyZI=";

#[sqlx::test(migrations = false)]
async fn an_erase_republishes_the_snapshot_with_the_subject_in_its_filter(pool: PgPool) {
    setup_demo(&pool).await;
    let store = Arc::new(MemoryStore::default());
    let edge = Edge::with_store(store.clone());
    pratibimba::edge::publish(&pool, &edge, "demo").await.unwrap();
    let before = store.last("demo").expect("a first snapshot");
    assert_eq!(before["erased"], json!({ "n": 0, "m": 0, "k": 7, "bits": "" }), "{}", before["erased"]);
    erase(&app_with(&pool, edge), "alice").await;
    let after = store.last("demo").expect("a snapshot");
    assert_eq!(after["version"], json!(before["version"].as_i64().unwrap() + 1), "the erase did not republish the snapshot");
    assert_eq!(after["erased"], json!({ "n": 1, "m": 64, "k": 7, "bits": ALICE_BITS }));
}

/// An erasure made with no edge (the CLI before the edge was configured) is in the filter after
/// the next worker pass, since the content hash moved.
#[sqlx::test(migrations = false)]
async fn erasures_made_without_an_edge_reach_the_filter_at_the_next_pass(pool: PgPool) {
    setup_demo(&pool).await;
    let store = Arc::new(MemoryStore::default());
    let edge = Edge::with_store(store.clone());
    pratibimba::edge::publish(&pool, &edge, "demo").await.unwrap();
    pratibimba::privacy::erase(&pool, "demo", "alice", "owner", None).await.unwrap();
    pratibimba::privacy::erase(&pool, "demo", "bob", "owner", None).await.unwrap();
    pratibimba::worker::run_drain_edge(&pool, &edge).await.unwrap();
    assert_eq!(store.last("demo").unwrap()["erased"], json!({ "n": 2, "m": 64, "k": 7, "bits": ALICE_BOB_BITS }));
}

/// The filter is per site: another site's erasure is not in demo's filter.
#[sqlx::test(migrations = false)]
async fn another_sites_erasure_is_not_in_the_filter(pool: PgPool) {
    setup_demo(&pool).await;
    sqlx::query("INSERT INTO sites (id, origin) VALUES ('other', 'https://other.test')").execute(&pool).await.unwrap();
    pratibimba::privacy::erase(&pool, "demo", "alice", "owner", None).await.unwrap();
    pratibimba::privacy::erase(&pool, "other", "bob", "owner", None).await.unwrap();
    let store = Arc::new(MemoryStore::default());
    pratibimba::edge::publish(&pool, &Edge::with_store(store.clone()), "demo").await.unwrap();
    assert_eq!(store.last("demo").unwrap()["erased"], json!({ "n": 1, "m": 64, "k": 7, "bits": ALICE_BITS }));
}
