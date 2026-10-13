//! EC3, Core's side of the write buffer (edge-cache.md): `POST /e/batch` takes what the edge
//! Worker's queue consumer drains (browser events and edge decision records) in one call. It is
//! idempotent on the client's UUIDs, checks the erase tombstone per event under the subject's
//! shared erase lock, takes an auth token, and says per id what happened: accepted, duplicate or
//! refused. Learning and attribution must not change: an event keeps the time the edge received
//! it, and an event that arrives before the decision it names waits for it in the outbox.

mod common;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

const EDGE: &str = "edge-test-token";

fn edge_app(pool: &PgPool) -> axum::Router {
    let edge = pratibimba::edge::Edge::off().with_token(EDGE);
    pratibimba::http::router(pratibimba::http::AppState::configured_with_edge(pool.clone(), Some(7), Some(TOKEN.into()), None, edge), &[])
}

async fn post_with(app: &axum::Router, path: &str, body: Value, headers: &[(&str, &str)]) -> Resp {
    let mut b = axum::http::Request::post(path).header("content-type", "application/json");
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    send(app, b.body(axum::body::Body::from(body.to_string())).unwrap()).await
}

async fn batch(app: &axum::Router, items: Vec<Value>) -> Resp {
    post_with(app, "/e/batch", json!({ "items": items }), &[("authorization", &format!("Bearer {EDGE}"))]).await
}

fn ev(name: &str) -> Value {
    json!({ "id": Uuid::new_v4(), "name": name, "slot": "hero" })
}

fn ev_for(name: &str, decision: &Value) -> Value {
    json!({ "id": Uuid::new_v4(), "name": name, "slot": "hero", "decision_id": decision })
}

fn events_item(subject: &str, events: Vec<Value>, received_at: chrono::DateTime<chrono::Utc>) -> Value {
    json!({ "kind": "events", "received_at": received_at.to_rfc3339(), "request": { "site": "demo", "subject": subject, "events": events } })
}

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}

/// The ids in one of the response's lists.
fn ids(r: &Resp, list: &str) -> Vec<String> {
    assert_eq!(r.status, 200, "{}", r.text);
    r.json()[list].as_array().unwrap_or_else(|| panic!("no {list} in {}", r.text)).iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

fn refused(r: &Resp) -> Vec<(String, String)> {
    assert_eq!(r.status, 200, "{}", r.text);
    r.json()["refused"].as_array().unwrap().iter().map(|v| (v["id"].as_str().unwrap_or("").to_string(), v["reason"].as_str().unwrap().to_string())).collect()
}

fn id_of(e: &Value) -> String {
    e["id"].as_str().unwrap().to_string()
}

async fn setup(pool: &PgPool, mode: &str) {
    pratibimba::db::migrate(pool).await.unwrap();
    let mut f = demo_file();
    f.privacy_mode = Some(mode.into());
    pratibimba::site_file::apply(pool, &f).await.unwrap();
    set_holdout(pool, 0.0).await;
}

// ---- the door ----

#[sqlx::test(migrations = false)]
async fn batch_is_off_without_a_token_and_needs_it(pool: PgPool) {
    setup(&pool, "full").await;
    let item = events_item("v", vec![ev("click")], now());
    let off = admin_app(&pool);
    let r = post_with(&off, "/e/batch", json!({ "items": [item.clone()] }), &[("authorization", "Bearer x")]).await;
    assert_eq!(r.status, 404, "{}", r.text);
    let app = edge_app(&pool);
    for auth in ["Bearer nope-nope-nope-no", "", EDGE] {
        let r = post_with(&app, "/e/batch", json!({ "items": [item.clone()] }), &[("authorization", auth)]).await;
        assert_eq!(r.status, 401, "authorization {auth:?}: {}", r.text);
    }
    assert_eq!(count(&pool, "SELECT count(*) FROM events").await, 0);
    // With the token it is written.
    assert_eq!(ids(&batch(&app, vec![item]).await, "accepted").len(), 1);
}

// ---- idempotent on the event UUID ----

#[sqlx::test(migrations = false)]
async fn a_redelivered_batch_is_all_duplicates_and_writes_nothing_twice(pool: PgPool) {
    setup(&pool, "full").await;
    let app = edge_app(&pool);
    let (a, b, c) = (ev("exposure"), ev("click"), ev("click"));
    let items = vec![events_item("alice", vec![a.clone(), b.clone()], now()), events_item("bob", vec![c.clone()], now())];
    let first = batch(&app, items.clone()).await;
    let mut got = ids(&first, "accepted");
    got.sort();
    let mut want = vec![id_of(&a), id_of(&b), id_of(&c)];
    want.sort();
    assert_eq!(got, want);
    assert!(ids(&first, "duplicate").is_empty());
    assert_eq!(count(&pool, "SELECT count(*) FROM events").await, 3);
    assert_eq!(count(&pool, "SELECT count(*) FROM pending_events").await, 3);
    assert_eq!(count(&pool, "SELECT events FROM visitors WHERE subject = 'alice'").await, 2);
    // The queue delivers at least once: the same batch again is absorbed.
    let again = batch(&app, items).await;
    assert!(ids(&again, "accepted").is_empty(), "{}", again.text);
    assert_eq!(ids(&again, "duplicate").len(), 3);
    assert_eq!(count(&pool, "SELECT count(*) FROM events").await, 3);
    assert_eq!(count(&pool, "SELECT count(*) FROM pending_events").await, 3);
    assert_eq!(count(&pool, "SELECT events FROM visitors WHERE subject = 'alice'").await, 2, "a redelivery counted the visit twice");
    assert_eq!(count(&pool, "SELECT events FROM visitors WHERE subject = 'bob'").await, 1);
    // A later batch adds to the visitor's summary rather than replacing it.
    let more = batch(&app, vec![events_item("alice", vec![ev("click")], now())]).await;
    assert_eq!(ids(&more, "accepted").len(), 1);
    assert_eq!(count(&pool, "SELECT events FROM visitors WHERE subject = 'alice'").await, 3);
    let recent: Vec<String> = sqlx::query_scalar("SELECT recent FROM visitors WHERE subject = 'alice'").fetch_one(&pool).await.unwrap();
    assert_eq!(recent, vec!["exposure", "click", "click"]);
}

#[sqlx::test(migrations = false)]
async fn the_same_event_twice_in_one_batch_is_written_once(pool: PgPool) {
    setup(&pool, "full").await;
    let app = edge_app(&pool);
    let e = ev("click");
    let r = batch(&app, vec![events_item("v", vec![e.clone()], now()), events_item("v", vec![e.clone()], now())]).await;
    assert_eq!(ids(&r, "accepted"), vec![id_of(&e)]);
    assert_eq!(ids(&r, "duplicate"), vec![id_of(&e)]);
    assert_eq!(count(&pool, "SELECT count(*) FROM events").await, 1);
    assert_eq!(count(&pool, "SELECT events FROM visitors").await, 1);
}

// ---- the tombstone, per event ----

#[sqlx::test(migrations = false)]
async fn an_erased_subjects_events_are_dropped_and_the_rest_written(pool: PgPool) {
    setup(&pool, "full").await;
    let app = edge_app(&pool);
    pratibimba::privacy::erase(&pool, "demo", "carol", "test", None).await.unwrap();
    let (live, gone1, gone2) = (ev("click"), ev("exposure"), ev("click"));
    // The erased subject is second, so a check of only the first item would let it through.
    let r = batch(&app, vec![events_item("dan", vec![live.clone()], now()), events_item("carol", vec![gone1.clone(), gone2.clone()], now())]).await;
    assert_eq!(ids(&r, "accepted"), vec![id_of(&live)]);
    let mut refusals = refused(&r);
    refusals.sort();
    let mut want = vec![(id_of(&gone1), "erased".to_string()), (id_of(&gone2), "erased".to_string())];
    want.sort();
    assert_eq!(refusals, want);
    let stored_for = |s: &'static str| format!("SELECT count(*) FROM events WHERE subject = '{s}'");
    assert_eq!(count(&pool, &stored_for("carol")).await, 0, "an erased subject's event was written");
    assert_eq!(count(&pool, "SELECT count(*) FROM visitors WHERE subject = 'carol'").await, 0);
    assert_eq!(count(&pool, &stored_for("dan")).await, 1);
}

/// An erase that holds the subject's lock when the batch arrives: the batch waits for it and
/// then sees the tombstone. The tombstone must be read after the lock is held, in its own
/// statement (READ COMMITTED takes a statement's snapshot before an in-statement lock waits).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_batch_racing_an_erase_waits_for_it_and_writes_nothing() {
    let db = FreshDb::new().await;
    let pool = db.pool.clone();
    setup(&pool, "full").await;
    let app = edge_app(&pool);
    let mut eraser = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock($1)").bind(pratibimba::privacy::lock_key("demo", "erin")).execute(&mut *eraser).await.unwrap();
    let e = ev("click");
    let item = events_item("erin", vec![e.clone()], now());
    let task = tokio::spawn({
        let app = app.clone();
        async move { batch(&app, vec![item]).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(!task.is_finished(), "the batch did not wait for the erase's lock");
    sqlx::query("INSERT INTO erasures (site_id, subject_hash, actor, purge_after) VALUES ('demo', $1, 'test', now())")
        .bind(pratibimba::privacy::subject_hash("demo", "erin"))
        .execute(&mut *eraser)
        .await
        .unwrap();
    eraser.commit().await.unwrap();
    let r = task.await.unwrap();
    assert_eq!(refused(&r), vec![(id_of(&e), "erased".to_string())]);
    assert_eq!(count(&pool, "SELECT count(*) FROM events").await, 0);
    db.drop().await;
}

// ---- what the batch refuses ----

#[sqlx::test(migrations = false)]
async fn picks_rescores_unknown_sites_and_bad_items_are_refused_alone(pool: PgPool) {
    setup(&pool, "full").await;
    let app = edge_app(&pool);
    let ok = ev("click");
    let pick = json!({ "id": Uuid::new_v4(), "name": "pick", "slot": "hero", "picks": ["a"] });
    let other = ev("click");
    let bad_name = json!({ "id": Uuid::new_v4(), "name": "Click!" });
    let r = batch(
        &app,
        vec![
            events_item("v", vec![ok.clone(), pick.clone()], now()),
            json!({ "kind": "events", "received_at": now().to_rfc3339(), "request": { "site": "nope", "subject": "v", "events": [other.clone()] } }),
            events_item("v", vec![bad_name.clone()], now()),
            json!({ "kind": "mystery" }),
        ],
    )
    .await;
    assert_eq!(ids(&r, "accepted"), vec![id_of(&ok)]);
    let refusals = refused(&r);
    let reason_for = |id: &str| refusals.iter().find(|(i, _)| i == id).map(|(_, r)| r.clone()).unwrap_or_else(|| panic!("{id} not refused: {}", r.text));
    assert!(reason_for(&id_of(&pick)).contains("/e"), "{}", r.text);
    assert!(reason_for(&id_of(&other)).contains("unknown site"), "{}", r.text);
    assert!(reason_for(&id_of(&bad_name)).contains("event name"), "{}", r.text);
    assert!(refusals.iter().any(|(id, reason)| id.is_empty() && reason.contains("kind")), "{}", r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM events").await, 1);
}

// ---- learning and attribution unchanged ----

/// A click the edge received inside the window, delivered after the window ended (but before it
/// was settled), is credited: the event keeps the edge's clock, not the drain's.
#[sqlx::test(migrations = false)]
async fn an_event_keeps_the_time_the_edge_received_it(pool: PgPool) {
    setup(&pool, "full").await;
    let app = edge_app(&pool);
    let d = decide(&app, "v", false).await.json()["decision_id"].clone();
    // The decision was made 31 minutes ago; the window is 30.
    sqlx::query("UPDATE decisions SET created_at = now() - interval '31 minutes'").execute(&pool).await.unwrap();
    let at = now() - chrono::Duration::minutes(2);
    let r = batch(&app, vec![events_item("v", vec![ev_for("exposure", &d), ev_for("click", &d)], at)]).await;
    assert_eq!(ids(&r, "accepted").len(), 2);
    let stored: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("SELECT received_at FROM events WHERE name = 'click'").fetch_one(&pool).await.unwrap();
    assert!((stored - at).num_milliseconds().abs() < 5, "stored {stored}, the edge received it at {at}");
    pratibimba::worker::run_once(&pool, now()).await.unwrap();
    let outcome: Option<i16> = sqlx::query_scalar("SELECT outcome FROM decisions").fetch_one(&pool).await.unwrap();
    assert_eq!(outcome, Some(1), "a click inside the window was not credited");
    // A clock ahead of Core's is held to Core's.
    let future = ev("click");
    batch(&app, vec![events_item("w", vec![future.clone()], now() + chrono::Duration::hours(1))]).await;
    let late: bool = sqlx::query_scalar("SELECT received_at <= now() FROM events WHERE id = $1").bind(Uuid::parse_str(&id_of(&future)).unwrap()).fetch_one(&pool).await.unwrap();
    assert!(late, "an event from the future was stored in the future");
}

/// The queue does not keep order: an exposure and a click can land before the decision they
/// name. They wait in the outbox for it, so the decision is still exposed and credited.
#[sqlx::test(migrations = false)]
async fn events_that_arrive_before_their_decision_wait_for_it(pool: PgPool) {
    setup(&pool, "full").await;
    let app = edge_app(&pool);
    let d = decide(&app, "v", false).await.json()["decision_id"].clone();
    // Take the decision out, as if its record were still in the queue.
    sqlx::query("CREATE TABLE held AS SELECT * FROM decisions").execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM decisions").execute(&pool).await.unwrap();
    let r = batch(&app, vec![events_item("v", vec![ev_for("exposure", &d), ev_for("click", &d)], now())]).await;
    assert_eq!(ids(&r, "accepted").len(), 2);
    pratibimba::worker::run_once(&pool, now()).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM pending_events").await, 2, "the events were drained before their decision existed");
    sqlx::query("INSERT INTO decisions SELECT * FROM held").execute(&pool).await.unwrap();
    pratibimba::worker::run_once(&pool, now()).await.unwrap();
    let (exposed, outcome): (bool, Option<i16>) = sqlx::query_as("SELECT exposed_at IS NOT NULL, outcome FROM decisions").fetch_one(&pool).await.unwrap();
    assert!(exposed, "the early exposure was lost");
    assert_eq!(outcome, Some(1), "the early click was lost");
    assert_eq!(count(&pool, "SELECT count(*) FROM pending_events").await, 0);
}

/// An event naming a decision that never comes does not wait forever.
#[sqlx::test(migrations = false)]
async fn an_orphan_event_is_drained_after_the_hold(pool: PgPool) {
    setup(&pool, "full").await;
    let app = edge_app(&pool);
    let ghost = json!(Uuid::new_v4());
    batch(&app, vec![events_item("v", vec![ev_for("click", &ghost)], now())]).await;
    let hold = chrono::Duration::seconds(pratibimba::worker::ORPHAN_HOLD_SECS);
    pratibimba::worker::run_once(&pool, now() + hold - chrono::Duration::seconds(30)).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM pending_events").await, 1, "drained before the hold ended");
    pratibimba::worker::run_once(&pool, now() + hold + chrono::Duration::seconds(5)).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM pending_events").await, 0, "an orphan stays in the outbox forever");
    // An event that names no decision is never held.
    batch(&app, vec![events_item("v", vec![ev("click")], now())]).await;
    pratibimba::worker::run_once(&pool, now()).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM pending_events").await, 0);
}

/// Cookieless: the event's subject is the pseudonym of the day the edge received it, which is
/// the subject its decision was stored under, so attribution can join them.
#[sqlx::test(migrations = false)]
async fn a_cookieless_event_takes_the_pseudonym_of_its_own_day(pool: PgPool) {
    setup(&pool, "cookieless").await;
    let app = edge_app(&pool);
    let yesterday = now() - chrono::Duration::days(1);
    let salt = pratibimba::privacy::salt(&pool, "demo", pratibimba::privacy::day_of(yesterday)).await.unwrap();
    let e = ev("click");
    let r = batch(&app, vec![events_item("raw-1", vec![e.clone()], yesterday)]).await;
    assert_eq!(ids(&r, "accepted").len(), 1);
    let stored: String = sqlx::query_scalar("SELECT subject FROM events").fetch_one(&pool).await.unwrap();
    assert_eq!(stored, pratibimba::privacy::pseudonym(&salt, "demo", "raw-1"));
}

// ---- edge decision records ride the same batch ----

async fn edge_record(pool: &PgPool, stored: &str, seed: u64, created_at: chrono::DateTime<chrono::Utc>) -> Value {
    let (slot_id, version, state): (i64, i64, Value) =
        sqlx::query_as("SELECT m.slot_id, m.version, m.state FROM models m JOIN slots s ON s.id = m.slot_id WHERE s.key = 'hero'").fetch_one(pool).await.unwrap();
    let pool_ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM variants WHERE slot_id = $1 AND status = 'approved' ORDER BY id").bind(slot_id).fetch_all(pool).await.unwrap();
    let model: pratibimba::bandit::LinTs = serde_json::from_value(state).unwrap();
    let x = [1.0, 0.0, 1.0, 0.0];
    let features = pratibimba::http::decide::named(&x);
    let (pick, scores) = pratibimba::explain::replay(&model, "lints", &features, &x, &pool_ids, seed);
    json!({
        "id": Uuid::new_v4(), "slot": "hero", "subject": stored, "variant_id": pick, "pool": pool_ids,
        "features": features, "scores": scores.iter().map(|(id, s)| (id.to_string(), json!(s))).collect::<serde_json::Map<_, _>>(),
        "policy": "lints", "policy_version": version, "seed": seed.to_string(), "in_holdout": false,
        "created_at": created_at.to_rfc3339(), "audiences": [], "cold_start": null, "experiment_id": null, "allocation": "bandit",
    })
}

fn decisions_item(subject: &str, decisions: Vec<Value>) -> Value {
    json!({ "kind": "decisions", "request": { "site": "demo", "subject": subject, "consent": null, "snapshot_version": 3, "decisions": decisions } })
}

#[sqlx::test(migrations = false)]
async fn edge_decisions_are_stored_checked_and_idempotent(pool: PgPool) {
    setup(&pool, "full").await;
    let app = edge_app(&pool);
    pratibimba::privacy::erase(&pool, "demo", "carol", "test", None).await.unwrap();
    let good = edge_record(&pool, "alice", 9, now()).await;
    let gone = edge_record(&pool, "carol", 9, now()).await;
    let mut wrong = edge_record(&pool, "bob", 9, now()).await;
    wrong["scores"] = json!({});
    // An hour old: past /edge/decisions' 10 minutes, but a queue that retried for a while (or a
    // replay from the dead-letter queue) still delivers it, and it is the record of what was shown.
    let old = edge_record(&pool, "dora", 9, now() - chrono::Duration::hours(1)).await;
    let click = ev_for("click", &good["id"]);
    let items = vec![
        // The event is first in the batch: decisions are written first all the same.
        events_item("alice", vec![click.clone()], now()),
        decisions_item("alice", vec![good.clone()]),
        decisions_item("carol", vec![gone.clone()]),
        decisions_item("bob", vec![wrong.clone()]),
        decisions_item("dora", vec![old.clone()]),
    ];
    let r = batch(&app, items.clone()).await;
    let mut acc = ids(&r, "accepted");
    acc.sort();
    let mut want = vec![id_of(&good), id_of(&click), id_of(&old)];
    want.sort();
    assert_eq!(acc, want, "{}", r.text);
    let refusals = refused(&r);
    assert!(refusals.contains(&(id_of(&gone), "erased".to_string())), "{}", r.text);
    assert!(refusals.iter().any(|(id, why)| id == &id_of(&wrong) && why.contains("replay")), "{}", r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions").await, 2);
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions WHERE source = 'edge' AND edge_snapshot_version = 3").await, 2);
    let again = batch(&app, items).await;
    let mut dup = ids(&again, "duplicate");
    dup.sort();
    assert_eq!(dup, want);
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions").await, 2);
}

/// Only the fields the site collects are kept, as at `/e`.
#[sqlx::test(migrations = false)]
async fn fields_the_site_does_not_collect_are_dropped(pool: PgPool) {
    setup(&pool, "full").await;
    sqlx::query("UPDATE sites SET fields_allowed = '{slot}'").execute(&pool).await.unwrap();
    let app = edge_app(&pool);
    let d = json!(Uuid::new_v4());
    let mut item = events_item("v", vec![ev_for("click", &d)], now());
    item["request"]["tz_offset"] = json!(330);
    assert_eq!(ids(&batch(&app, vec![item]).await, "accepted").len(), 1);
    let (decision, tz, slot): (Option<Uuid>, Option<i16>, Option<i64>) =
        sqlx::query_as("SELECT decision_id, tz_offset_min, slot_id FROM events").fetch_one(&pool).await.unwrap();
    assert_eq!((decision, tz), (None, None), "a field the site does not collect was stored");
    assert!(slot.is_some(), "the slot, which it collects, was dropped");
}
