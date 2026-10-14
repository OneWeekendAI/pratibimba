//! /decide's round trips (tracker, "Latency: Seoul move"): every statement is a round trip to a
//! database that may sit a few milliseconds away, so the count is the latency. These tests count
//! them at the wire, through a TCP proxy that sees what the Postgres client sends, and pin the
//! decision semantics the one-statement read must keep: approvals and retirements apply at once,
//! the cookieless salt is still the database's, and an erase that commits while a decide waits
//! for the subject's lock is still honoured.

mod common;
use common::*;
use pratibimba::http::{AppState, Ctx, DecideRequest, decide::decide};
use pratibimba::privacy;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A TCP proxy in front of the test server that counts round trips: each time the client sends
/// after the server last spoke (or first), that is one more trip. Segmenting cannot inflate it.
///
/// A flight that is only a Sync message is a ping (sqlx pings a connection when the pool hands
/// it back, and before handing it out unless told not to); those are counted apart, in `pings`.
async fn counting_proxy() -> (u16, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let url = std::env::var("DATABASE_URL").unwrap();
    let up = PgConnectOptions::from_str(&url).unwrap();
    let upstream = format!("{}:{}", up.get_host(), up.get_port());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let trips = Arc::new(AtomicUsize::new(0));
    let pings = Arc::new(AtomicUsize::new(0));
    let (t, p) = (trips.clone(), pings.clone());
    tokio::spawn(async move {
        loop {
            let (client, _) = listener.accept().await.unwrap();
            let server = TcpStream::connect(&upstream).await.unwrap();
            let (mut cr, mut cw) = client.into_split();
            let (mut sr, mut sw) = server.into_split();
            // 0 = nobody yet, 1 = the client spoke last, 2 = the server did.
            let last = Arc::new(AtomicU8::new(0));
            let (l1, t1, p1) = (last.clone(), t.clone(), p.clone());
            tokio::spawn(async move {
                let mut buf = vec![0u8; 64 * 1024];
                while let Ok(n) = cr.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    if l1.swap(1, Ordering::SeqCst) != 1 {
                        let ping = &buf[..n] == b"S\0\0\0\x04";
                        (if ping { &p1 } else { &t1 }).fetch_add(1, Ordering::SeqCst);
                        if std::env::var("PRB_TRACE_TRIPS").is_ok() {
                            eprintln!("trip: {:?}", String::from_utf8_lossy(&buf[..n.min(90)]));
                        }
                    }
                    if sw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
            tokio::spawn(async move {
                let mut buf = vec![0u8; 64 * 1024];
                while let Ok(n) = sr.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    last.store(2, Ordering::SeqCst);
                    if cw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    (port, trips, pings)
}

/// A pool on `db` through the proxy, with production's options and one connection, so every
/// statement reuses the warmed connection (and a decide that needed two at once would hang).
async fn proxied_pool(db: &FreshDb, port: u16) -> PgPool {
    let url = std::env::var("DATABASE_URL").unwrap();
    let opts = PgConnectOptions::from_str(&url).unwrap().host("127.0.0.1").port(port).database(&db.name);
    pratibimba::db::pool_options().max_connections(1).connect_with(opts).await.unwrap()
}

fn req(slot: &str, subject: &str, ctx: Value) -> DecideRequest {
    DecideRequest { site: "demo".into(), slot: slot.into(), subject: subject.into(), ctx: serde_json::from_value::<Ctx>(ctx).unwrap() }
}

/// `(statement round trips, pings)` one decide makes on a warm connection, including the ping
/// sqlx makes when decide hands its connection back (after the response is built).
async fn trips_of(state: &AppState, trips: &AtomicUsize, pings: &AtomicUsize, r: DecideRequest) -> (usize, usize) {
    settle(&state.pool).await;
    trips.store(0, Ordering::SeqCst);
    pings.store(0, Ordering::SeqCst);
    decide(state, &r).await.unwrap_or_else(|e| panic!("decide failed: {e:?}"));
    settle(&state.pool).await;
    (trips.load(Ordering::SeqCst), pings.load(Ordering::SeqCst))
}

/// Wait until the pool's one connection is idle again: sqlx hands a connection back (with its
/// ping) in a spawned task, so it lands after the call that released it returns.
async fn settle(pool: &PgPool) {
    for _ in 0..500 {
        if pool.num_idle() == 1 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    panic!("the pool's connection never came back");
}

/// The richest decide: cookieless (a pseudonym and its salt), a picker on the site (stated
/// features and the subject's picks), a declared feature, and a slot still in cold start.
fn rich_file() -> Value {
    json!({
        "site": "demo", "origin": "http://localhost:5173", "privacy_mode": "cookieless",
        "features": [ { "key": "vip", "pii": false } ],
        "slots": [
            { "key": "taste", "selector": "#taste", "kind": "picker", "metric": "click",
              "picker": { "max_picks": 3, "options": [ { "key": "kitchen", "label": "Kitchen" }, { "key": "garden", "label": "Garden" } ] },
              "variants": [ { "key": "none", "baseline": true }, { "key": "picker", "html": "<p>Pick</p>" } ] },
            { "key": "shelf", "selector": "#shelf", "metric": "click", "cold_start_exposures": 100,
              "variants": [ { "key": "baseline", "baseline": true }, { "key": "kitchen", "html": "<p>K</p>" } ] }
        ]
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_warm_decide_makes_four_round_trips_and_a_sticky_one_makes_one() {
    let db = FreshDb::new().await;
    setup_demo(&db.pool).await;
    let (port, trips, pings) = counting_proxy().await;
    let state = AppState::with_seed(proxied_pool(&db, port).await, 7);
    for s in ["warm-1", "warm-2"] {
        decide(&state, &req("hero", s, json!({}))).await.unwrap();
    }
    let fresh = trips_of(&state, &trips, &pings, req("hero", "fresh", json!({ "returning": true }))).await;
    let sticky = trips_of(&state, &trips, &pings, req("hero", "fresh", json!({ "returning": true }))).await;
    let n = count(&db.pool, "SELECT count(*) FROM decisions WHERE subject = 'fresh'").await;
    db.drop().await;
    assert_eq!(n, 1, "the second decide reused the first");
    assert!(fresh.0 <= 4, "a warm decide that writes made {} statement round trips; the budget is 4 (read, BEGIN+lock, check+insert, COMMIT)", fresh.0);
    assert!(sticky.0 <= 1, "a decide that reuses its sticky decision made {} statement round trips; the budget is 1", sticky.0);
    assert!(fresh.1 <= 1 && sticky.1 <= 1, "pings: {} and {}; only the one on hand-back is allowed", fresh.1, sticky.1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_warm_cookieless_decide_with_picks_features_and_cold_start_makes_four_round_trips() {
    let db = FreshDb::new().await;
    pratibimba::db::migrate(&db.pool).await.unwrap();
    pratibimba::site_file::apply(&db.pool, &file_from(rich_file())).await.unwrap();
    set_holdout(&db.pool, 0.0).await;
    let (port, trips, pings) = counting_proxy().await;
    let state = AppState::with_seed(proxied_pool(&db, port).await, 7);
    for s in ["warm-1", "warm-2"] {
        decide(&state, &req("shelf", s, json!({ "stated": ["kitchen"], "features": { "vip": true } }))).await.unwrap();
    }
    let r = req("shelf", "fresh", json!({ "stated": ["garden"], "features": { "vip": true } }));
    let fresh = trips_of(&state, &trips, &pings, r).await;
    let d: (Value, Option<Value>) =
        sqlx::query_as("SELECT features, cold_start FROM decisions ORDER BY created_at DESC LIMIT 1").fetch_one(&db.pool).await.unwrap();
    db.drop().await;
    // The fixture really exercised what it claims to: stated and declared features, cold start.
    assert_eq!(d.0["stated:garden"], json!(1.0), "{}", d.0);
    assert_eq!(d.0["feature:vip"], json!(1.0), "{}", d.0);
    assert!(d.1.is_some(), "cold start chose");
    assert!(fresh.0 <= 4, "a warm cookieless decide made {} statement round trips; the budget is 4", fresh.0);
    assert!(fresh.1 <= 1, "{} pings; only the one on hand-back is allowed", fresh.1);
}

/// The demo slot with no holdout, decided once so the process is warm.
async fn warm_demo(pool: &PgPool) -> Arc<AppState> {
    setup_learning(pool).await;
    let state = AppState::with_seed(pool.clone(), 7);
    decide(&state, &req("hero", "warm", json!({}))).await.unwrap();
    state
}

async fn variants_served(state: &AppState, prefix: &str, n: usize) -> Vec<String> {
    let mut out = vec![];
    for i in 0..n {
        out.push(decide(state, &req("hero", &format!("{prefix}-{i}"), json!({}))).await.unwrap().variant);
    }
    out
}

#[sqlx::test(migrations = false)]
async fn a_retired_variant_stops_serving_on_the_next_decide(pool: PgPool) {
    let state = warm_demo(&pool).await;
    assert!(variants_served(&state, "before", 20).await.contains(&"gift".to_string()), "gift serves before the retirement");
    sqlx::query("UPDATE variants SET status = 'retired' WHERE key = 'gift'").execute(&pool).await.unwrap();
    let after = variants_served(&state, "after", 40).await;
    assert!(!after.contains(&"gift".to_string()), "{after:?}");
}

#[sqlx::test(migrations = false)]
async fn a_newly_approved_variant_serves_on_the_next_decide(pool: PgPool) {
    let state = warm_demo(&pool).await;
    insert_variant(&pool, "fresh", "approved").await;
    let after = variants_served(&state, "after", 40).await;
    assert!(after.contains(&"fresh".to_string()), "{after:?}");
}

#[sqlx::test(migrations = false)]
async fn a_pause_applies_on_the_next_decide(pool: PgPool) {
    let state = warm_demo(&pool).await;
    sqlx::query("UPDATE sites SET paused_at = now(), paused_by = 'test'").execute(&pool).await.unwrap();
    let after = variants_served(&state, "after", 20).await;
    assert!(after.iter().all(|v| v == "baseline"), "{after:?}");
}

async fn cookieless(pool: &PgPool) -> Arc<AppState> {
    pratibimba::db::migrate(pool).await.unwrap();
    let mut f = demo_file();
    f.privacy_mode = Some("cookieless".into());
    pratibimba::site_file::apply(pool, &f).await.unwrap();
    AppState::with_seed(pool.clone(), 7)
}

async fn stored(pool: &PgPool, id: uuid::Uuid) -> String {
    sqlx::query_scalar("SELECT subject FROM decisions WHERE id = $1").bind(id).fetch_one(pool).await.unwrap()
}

async fn salts(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT salt FROM privacy_salts ORDER BY day").fetch_all(pool).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn a_cookieless_decide_uses_the_databases_salt_not_the_one_it_remembers(pool: PgPool) {
    let state = cookieless(&pool).await;
    let first = decide(&state, &req("hero", "alice", json!({}))).await.unwrap();
    let old = salts(&pool).await;
    assert_eq!(stored(&pool, first.decision_id).await, privacy::pseudonym(&old[0], "demo", "alice"));
    // Another process replaced today's salt (as a restore does: salts are never backed up).
    let new = "f".repeat(64);
    sqlx::query("UPDATE privacy_salts SET salt = $1").bind(&new).execute(&pool).await.unwrap();
    let second = decide(&state, &req("hero", "bob", json!({}))).await.unwrap();
    assert_eq!(stored(&pool, second.decision_id).await, privacy::pseudonym(&new, "demo", "bob"));
}

#[sqlx::test(migrations = false)]
async fn a_cookieless_decide_still_deletes_an_older_days_salt(pool: PgPool) {
    let state = cookieless(&pool).await;
    decide(&state, &req("hero", "alice", json!({}))).await.unwrap();
    // A late request made yesterday's salt beside today's (the race `privacy::salt` tolerates).
    sqlx::query("INSERT INTO privacy_salts (site_id, day, salt) SELECT site_id, day - 1, $1 FROM privacy_salts").bind("0".repeat(64)).execute(&pool).await.unwrap();
    assert_eq!(salts(&pool).await.len(), 2);
    decide(&state, &req("hero", "bob", json!({}))).await.unwrap();
    assert_eq!(salts(&pool).await.len(), 1, "the older day's salt is deleted, as every decide did before");
}

#[sqlx::test(migrations = false)]
async fn a_cookieless_decide_on_a_new_day_makes_that_days_salt(pool: PgPool) {
    let state = cookieless(&pool).await;
    decide(&state, &req("hero", "alice", json!({}))).await.unwrap();
    let today = salts(&pool).await;
    // The day turns: today's salt is now yesterday's.
    sqlx::query("UPDATE privacy_salts SET day = day - 1").execute(&pool).await.unwrap();
    let d = decide(&state, &req("hero", "alice", json!({}))).await.unwrap();
    let tomorrow = salts(&pool).await;
    assert_eq!(tomorrow.len(), 1);
    assert_ne!(tomorrow, today);
    assert_eq!(stored(&pool, d.decision_id).await, privacy::pseudonym(&tomorrow[0], "demo", "alice"));
}

/// An erase that commits while decide waits on the subject's lock must still refuse it: the
/// tombstone check has to read after the lock is held, not in a snapshot taken before.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_erase_that_commits_while_decide_waits_is_honoured() {
    let db = FreshDb::new().await;
    setup_learning(&db.pool).await;
    let state = AppState::with_seed(db.pool.clone(), 7);
    decide(&state, &req("hero", "warm", json!({}))).await.unwrap();
    // Stand in for an erase: the exclusive lock, then the tombstone, committed later.
    let mut erase = db.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock($1)").bind(privacy::lock_key("demo", "alice")).execute(&mut *erase).await.unwrap();
    sqlx::query("INSERT INTO erasures (site_id, subject_hash, actor, purge_after) VALUES ('demo', $1, 'test', now())")
        .bind(privacy::subject_hash("demo", "alice"))
        .execute(&mut *erase)
        .await
        .unwrap();
    let s = state.clone();
    let pending = tokio::spawn(async move { decide(&s, &req("hero", "alice", json!({}))).await });
    // Wait until decide is blocked on the lock (bounded: without a lock it never blocks).
    let mut waited = false;
    for _ in 0..100 {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND NOT granted").fetch_one(&db.pool).await.unwrap();
        if n > 0 {
            waited = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    erase.commit().await.unwrap();
    let got = pending.await.unwrap();
    let n = count(&db.pool, "SELECT count(*) FROM decisions WHERE subject = 'alice'").await;
    db.drop().await;
    assert!(waited, "decide never waited for the subject's lock");
    assert!(matches!(got, Err(pratibimba::http::ApiError::Gone(_))), "{:?}", got.map(|d| d.variant));
    assert_eq!(n, 0, "no decision for an erased subject");
}

/// A full-mode decide that reads the visitor's history (F3) for both the learner (a `history:`
/// block) and a gate (an audience on "last order 30+ days ago") still makes four round trips:
/// the history is a column of the one snapshot statement.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_warm_decide_that_reads_history_makes_four_round_trips() {
    let db = FreshDb::new().await;
    pratibimba::db::migrate(&db.pool).await.unwrap();
    let file = json!({
        "site": "demo", "origin": "http://localhost:5173", "privacy_mode": "full",
        "history": { "views": [ { "key": "filters", "category": "filters", "days": 30 } ] },
        "slots": [ { "key": "hero", "selector": "#hero",
                     "variants": [ { "key": "baseline", "baseline": true }, { "key": "gift", "html": "<h1>Back</h1>" } ] } ]
    });
    pratibimba::site_file::apply(&db.pool, &file_from(file)).await.unwrap();
    set_holdout(&db.pool, 0.0).await;
    let admin = admin_app(&db.pool);
    let aud = api_post(&admin, "/api/sites/demo/segments", json!({ "predicate": { "days_since_last_purchase": { "gte": 30 } }, "actor": "owner" })).await.json()["id"]
        .as_i64()
        .unwrap();
    api_post(&admin, &format!("/api/segments/{aud}/promote"), json!({ "actor": "owner" })).await;
    let gift = variant_id(&db.pool, "gift").await;
    let r = api_post(&admin, &format!("/api/variants/{gift}/audience"), json!({ "audience": aud, "actor": "owner" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    sqlx::query(
        "INSERT INTO events (id, site_id, subject, name, received_at, order_id, value, currency, items) VALUES
           (gen_random_uuid(), 'demo', 'fresh', 'purchase', now() - interval '41 days', 'o1', 600, 'USD', NULL),
           (gen_random_uuid(), 'demo', 'fresh', 'click', now() - interval '1 day', NULL, NULL, NULL, '[{\"id\": \"f\", \"category\": \"filters\"}]')",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let (port, trips, pings) = counting_proxy().await;
    let state = AppState::with_seed(proxied_pool(&db, port).await, 7);
    for s in ["warm-1", "warm-2"] {
        decide(&state, &req("hero", s, json!({}))).await.unwrap();
    }
    let fresh = trips_of(&state, &trips, &pings, req("hero", "fresh", json!({}))).await;
    let d: (Value, Value) = sqlx::query_as("SELECT features, audiences FROM decisions WHERE subject = 'fresh'").fetch_one(&db.pool).await.unwrap();
    db.drop().await;
    // The fixture really read history, for the learner and for the gate.
    assert_eq!(d.0["feature:history.viewed.filters"], json!(1.0), "{}", d.0);
    assert_eq!(d.1[0]["matched"], json!(true), "{}", d.1);
    assert!(fresh.0 <= 4, "a warm decide that reads history made {} statement round trips; the budget is 4", fresh.0);
    assert!(fresh.1 <= 1, "{} pings; only the one on hand-back is allowed", fresh.1);
}
