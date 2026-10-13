//! EC2, Core's side of edge decide (edge-cache.md): `/decide` tells the edge Worker, and only
//! it, the stored subject and record of each decision; `POST /edge/decisions` accepts the
//! decisions the Worker made, refusing any that Core would not have made for that visitor, and
//! stores them so that replay, explain, stickiness and erasure treat them like Core's own; and the
//! snapshot is also pushed to the Worker's beacon, which the 5 s freshness rule rests on.

mod common;
use common::*;
use pratibimba::edge::{Edge, MemoryStore};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;

const EDGE: &str = "edge-test-token";

fn edge_app(pool: &PgPool) -> axum::Router {
    let edge = Edge::off().with_token(EDGE);
    pratibimba::http::router(pratibimba::http::AppState::configured_with_edge(pool.clone(), Some(7), Some(TOKEN.into()), None, edge), &[])
}

async fn post_with(app: &axum::Router, path: &str, body: Value, headers: &[(&str, &str)]) -> Resp {
    let mut b = axum::http::Request::post(path).header("content-type", "application/json");
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    send(app, b.body(axum::body::Body::from(body.to_string())).unwrap()).await
}

fn unhex(h: &axum::http::HeaderValue) -> Value {
    let s = h.to_str().unwrap();
    let bytes: Vec<u8> = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect();
    serde_json::from_slice(&bytes).unwrap()
}

async fn setup(pool: &PgPool, mode: &str) {
    pratibimba::db::migrate(pool).await.unwrap();
    let mut f = demo_file();
    f.privacy_mode = Some(mode.into());
    pratibimba::site_file::apply(pool, &f).await.unwrap();
    set_holdout(pool, 0.0).await;
}

/// A decision as the edge Worker would send it for `stored` (the stored subject): the policy's
/// own pick for `seed` against the slot's current model, with the scores replay gives.
async fn edge_record(pool: &PgPool, stored: &str, seed: u64) -> Value {
    let (slot_id, version, state): (i64, i64, Value) =
        sqlx::query_as("SELECT m.slot_id, m.version, m.state FROM models m JOIN slots s ON s.id = m.slot_id WHERE s.key = 'hero'").fetch_one(pool).await.unwrap();
    let pool_ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM variants WHERE slot_id = $1 AND status = 'approved' ORDER BY id").bind(slot_id).fetch_all(pool).await.unwrap();
    let model: pratibimba::bandit::LinTs = serde_json::from_value(state).unwrap();
    let x = [1.0, 0.0, 1.0, 0.0];
    let features = pratibimba::http::decide::named(&x);
    let (pick, scores) = pratibimba::explain::replay(&model, "lints", &features, &x, &pool_ids, seed);
    json!({
        "id": uuid::Uuid::new_v4(), "slot": "hero", "subject": stored, "variant_id": pick, "pool": pool_ids,
        "features": features, "scores": scores.iter().map(|(id, s)| (id.to_string(), json!(s))).collect::<serde_json::Map<_, _>>(),
        "policy": "lints", "policy_version": version, "seed": seed.to_string(), "in_holdout": false,
        "created_at": chrono::Utc::now().to_rfc3339(), "created_at_ms": 0, "audiences": [], "cold_start": null,
        "experiment_id": null, "allocation": "bandit",
    })
}

async fn ingest(app: &axum::Router, subject: &str, consent: Option<bool>, decisions: Vec<Value>) -> Resp {
    post_with(
        app,
        "/edge/decisions",
        json!({ "site": "demo", "subject": subject, "consent": consent, "snapshot_version": 5, "decisions": decisions }),
        &[("authorization", &format!("Bearer {EDGE}"))],
    )
    .await
}

fn reason(r: &Resp) -> String {
    r.json()["results"][0]["reason"].as_str().unwrap_or_default().to_string()
}

fn status(r: &Resp) -> String {
    assert_eq!(r.status, 200, "{}", r.text);
    let v = r.json();
    v["results"][0]["status"].as_str().unwrap().to_string()
}

// ---- /decide tells the Worker what it needs, and nobody else ----

#[sqlx::test(migrations = false)]
async fn decide_reports_the_stored_subject_to_the_token_holder_only(pool: PgPool) {
    setup(&pool, "cookieless").await;
    let app = edge_app(&pool);
    let body = json!({ "site": "demo", "slot": "hero", "subject": "raw-1" });
    let with = post_with(&app, "/decide", body.clone(), &[("x-prb-edge-token", EDGE)]).await;
    assert_eq!(with.status, 200, "{}", with.text);
    let info = unhex(with.headers.get("x-prb-edge").expect("no x-prb-edge for the token holder"));
    let stored: String = sqlx::query_scalar("SELECT subject FROM decisions").fetch_one(&pool).await.unwrap();
    assert!(stored.starts_with("p1_"));
    assert_eq!(info["subject"], json!(stored));
    assert_eq!(info["records"][0]["id"], with.json()["decision_id"]);
    let rec = &info["records"][0];
    let row: (i64, i64) = sqlx::query_as("SELECT variant_id, seed FROM decisions").fetch_one(&pool).await.unwrap();
    assert_eq!(rec["variant_id"], json!(row.0));
    assert_eq!(rec["seed"], json!((row.1 as u64).to_string()));
    for h in [vec![], vec![("x-prb-edge-token", "wrong-token-val")]] {
        let r = post_with(&app, "/decide", body.clone(), &h).await;
        assert!(r.headers.get("x-prb-edge").is_none(), "x-prb-edge without the token");
    }
    // A Core with no edge token tells nobody.
    let plain = pratibimba::http::router(pratibimba::http::AppState::with_seed(pool.clone(), 1), &[]);
    let r = post_with(&plain, "/decide", body, &[("x-prb-edge-token", "")]).await;
    assert!(r.headers.get("x-prb-edge").is_none());
}

#[sqlx::test(migrations = false)]
async fn a_sticky_decision_reports_its_own_record(pool: PgPool) {
    setup(&pool, "full").await;
    let app = edge_app(&pool);
    let body = json!({ "site": "demo", "slot": "hero", "subject": "s" });
    let a = post_with(&app, "/decide", body.clone(), &[("x-prb-edge-token", EDGE)]).await;
    let b = post_with(&app, "/decide", body, &[("x-prb-edge-token", EDGE)]).await;
    let (ia, ib) = (unhex(a.headers.get("x-prb-edge").unwrap()), unhex(b.headers.get("x-prb-edge").unwrap()));
    assert_eq!(b.json()["decision_id"], a.json()["decision_id"], "not sticky");
    assert_eq!(ib["records"][0], ia["records"][0], "the sticky decision's record changed");
    assert_ne!(ib["at"], ia["at"], "`at` is the decision's clock, not the record's");
}

// ---- POST /edge/decisions ----

#[sqlx::test(migrations = false)]
async fn ingest_is_off_without_a_token_and_needs_it(pool: PgPool) {
    setup(&pool, "full").await;
    let off = admin_app(&pool);
    let rec = edge_record(&pool, "s", 1).await;
    let r = post_with(&off, "/edge/decisions", json!({ "site": "demo", "subject": "s", "snapshot_version": 1, "decisions": [rec.clone()] }), &[("authorization", "Bearer x")]).await;
    assert_eq!(r.status, 404);
    let app = edge_app(&pool);
    for auth in ["Bearer nope-nope-nope-no", "", "edge-test-token"] {
        let r = post_with(&app, "/edge/decisions", json!({ "site": "demo", "subject": "s", "snapshot_version": 1, "decisions": [rec.clone()] }), &[("authorization", auth)]).await;
        assert_eq!(r.status, 401, "authorization {auth:?}");
    }
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions").await, 0);
}

#[sqlx::test(migrations = false)]
async fn an_edge_decision_is_stored_as_core_would_and_replays(pool: PgPool) {
    setup(&pool, "full").await;
    let app = edge_app(&pool);
    sqlx::query("INSERT INTO webhook_subscriptions (site_id, url, events, secret) VALUES ('demo', 'https://hooks.example/x', '{decision}', 's')")
        .execute(&pool)
        .await
        .unwrap();
    let mut rec = edge_record(&pool, "alice", 99).await;
    // What the edge computed may differ from Core in the last bit (Math.exp); Core stores its own.
    let first = rec["scores"].as_object().unwrap().keys().next().unwrap().clone();
    let core_score = rec["scores"][&first].as_f64().unwrap();
    rec["scores"][&first] = json!(core_score * (1.0 + 1e-15));
    let id = rec["id"].clone();
    assert_eq!(status(&ingest(&app, "alice", None, vec![rec.clone()]).await), "stored");
    let (source, version, subject, stored): (String, Option<i64>, String, Value) =
        sqlx::query_as("SELECT source, edge_snapshot_version, subject, scores FROM decisions WHERE id = $1").bind(uuid::Uuid::parse_str(id.as_str().unwrap()).unwrap()).fetch_one(&pool).await.unwrap();
    assert_eq!((source.as_str(), version, subject.as_str()), ("edge", Some(5), "alice"));
    assert_eq!(stored[&first].as_f64().unwrap(), core_score, "Core stored the edge's score, not its own replay");
    assert_eq!(count(&pool, "SELECT count(*) FROM webhook_deliveries WHERE event = 'decision'").await, 1, "no decision webhook");
    // The decision page replays it exactly.
    let d = api_get(&app, &format!("/api/decisions/{}", id.as_str().unwrap())).await;
    assert_eq!(d.status, 200, "{}", d.text);
    assert_eq!(d.json()["replay_matches"], json!(true), "{}", d.text);
    // A retried send is a duplicate, not a second row.
    assert_eq!(status(&ingest(&app, "alice", None, vec![rec]).await), "duplicate");
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions").await, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM webhook_deliveries").await, 1);
}

#[sqlx::test(migrations = false)]
async fn core_keeps_an_edge_decision_sticky(pool: PgPool) {
    setup(&pool, "full").await;
    let app = edge_app(&pool);
    let rec = edge_record(&pool, "bob", 5).await;
    assert_eq!(status(&ingest(&app, "bob", None, vec![rec.clone()]).await), "stored");
    let r = post(&app, "/decide", json!({ "site": "demo", "slot": "hero", "subject": "bob", "ctx": { "mobile": true } })).await;
    assert_eq!(r.json()["decision_id"], rec["id"], "Core re-decided a visitor the edge decided a moment ago");
}

#[sqlx::test(migrations = false)]
async fn an_erased_visitors_edge_decision_is_dropped(pool: PgPool) {
    setup(&pool, "full").await;
    let app = edge_app(&pool);
    pratibimba::privacy::erase(&pool, "demo", "carol", "test", None).await.unwrap();
    let rec = edge_record(&pool, "carol", 5).await;
    assert_eq!(status(&ingest(&app, "carol", None, vec![rec]).await), "gone");
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions").await, 0);
}

#[sqlx::test(migrations = false)]
async fn cookieless_the_subject_must_be_todays_pseudonym(pool: PgPool) {
    setup(&pool, "cookieless").await;
    let app = edge_app(&pool);
    // Today's salt exists (a decide made it), so a refusal below is the subject rule's, not
    // "no salt yet".
    let salt = pratibimba::privacy::salt(&pool, "demo", pratibimba::privacy::day_of(chrono::Utc::now())).await.unwrap();
    // The raw id where a pseudonym belongs: refused.
    let raw = edge_record(&pool, "dave", 5).await;
    let r = ingest(&app, "dave", None, vec![raw]).await;
    assert_eq!(status(&r), "rejected");
    assert!(reason(&r).contains("subject"), "{}", r.text);
    // Today's pseudonym: stored.
    let p = pratibimba::privacy::pseudonym(&salt, "demo", "dave");
    let ok = edge_record(&pool, &p, 5).await;
    assert_eq!(status(&ingest(&app, "dave", None, vec![ok]).await), "stored");
    // A decision made just before midnight and sent just after: see
    // a_decision_from_before_midnight_keeps_its_own_days_salt (the age rule hides it here).
    // Consent on a consent_aware site makes the raw id right.
    sqlx::query("UPDATE sites SET privacy_mode = 'consent_aware'").execute(&pool).await.unwrap();
    let consented = edge_record(&pool, "dave", 7).await;
    assert_eq!(status(&ingest(&app, "dave", Some(true), vec![consented]).await), "stored");
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions").await, 2);
}

#[sqlx::test(migrations = false)]
async fn a_decision_core_would_not_have_made_is_refused(pool: PgPool) {
    setup(&pool, "full").await;
    let app = edge_app(&pool);
    // The pick must be the policy's argmax for a bandit allocation.
    let mut wrong = edge_record(&pool, "e", 11).await;
    let pick = wrong["variant_id"].as_i64().unwrap();
    let other = wrong["pool"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).find(|v| *v != pick).unwrap();
    wrong["variant_id"] = json!(other);
    // Scores the replay does not give.
    let mut forged = edge_record(&pool, "f", 12).await;
    let k = forged["scores"].as_object().unwrap().keys().next().unwrap().clone();
    forged["scores"][&k] = json!(forged["scores"][&k].as_f64().unwrap() + 0.01);
    // A holdout flag the hash does not give.
    let mut held = edge_record(&pool, "g", 13).await;
    held["in_holdout"] = json!(true);
    // A model version Core never had.
    let mut unknown = edge_record(&pool, "h", 14).await;
    unknown["policy_version"] = json!(99_999);
    // From too long ago.
    let mut old = edge_record(&pool, "i", 15).await;
    old["created_at"] = json!((chrono::Utc::now() - chrono::Duration::minutes(20)).to_rfc3339());
    for (name, rec, subject) in [("wrong pick", wrong, "e"), ("forged scores", forged, "f"), ("holdout", held, "g"), ("unknown version", unknown, "h"), ("old", old, "i")] {
        assert_eq!(status(&ingest(&app, subject, None, vec![rec]).await), "rejected", "{name}");
    }
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions").await, 0);
    // And the same shapes, made right, are accepted: the refusals are not a constant.
    assert_eq!(status(&ingest(&app, "e", None, vec![edge_record(&pool, "e", 11).await]).await), "stored");
}

#[sqlx::test(migrations = false)]
async fn holdout_decisions_are_accepted_when_the_hash_says_so(pool: PgPool) {
    setup(&pool, "full").await;
    sqlx::query("UPDATE sites SET holdout_fraction = 0.5").execute(&pool).await.unwrap();
    let app = edge_app(&pool);
    let subject = (0..200).map(|i| format!("hold-{i}")).find(|s| pratibimba::hash::in_holdout("demo", s, f64::from(0.5f32))).unwrap();
    let baseline = variant_id(&pool, "baseline").await;
    let mut rec = edge_record(&pool, &subject, 3).await;
    rec["in_holdout"] = json!(true);
    rec["variant_id"] = json!(baseline);
    assert_eq!(status(&ingest(&app, &subject, None, vec![rec]).await), "stored");
}

// ---- the beacon ----

#[sqlx::test(migrations = false)]
async fn the_snapshot_is_pushed_to_the_beacon_and_kv_and_a_failure_is_retried(pool: PgPool) {
    setup(&pool, "full").await;
    type Seen = Arc<std::sync::Mutex<Vec<(String, String, Value)>>>;
    let seen: Seen = Default::default();
    let fail = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let (s2, f2) = (seen.clone(), fail.clone());
    let mock = axum::Router::new().fallback(move |req: axum::extract::Request| {
        let (seen, fail) = (s2.clone(), f2.clone());
        async move {
            let (parts, body) = req.into_parts();
            let bytes = http_body_util::BodyExt::collect(body).await.unwrap().to_bytes();
            let auth = parts.headers.get("authorization").map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
            seen.lock().unwrap().push((parts.uri.path().to_string(), auth, serde_json::from_slice(&bytes).unwrap_or(Value::Null)));
            let beacon = parts.uri.path() == "/_prb/edge/snapshot";
            if beacon && fail.load(std::sync::atomic::Ordering::SeqCst) {
                return (axum::http::StatusCode::BAD_GATEWAY, "down").into_response();
            }
            axum::Json(json!({ "success": true })).into_response()
        }
    });
    use axum::response::IntoResponse;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let vars: Vec<(String, String)> = [
        ("PRB_EDGE_KV_ACCOUNT", "acct".to_string()),
        ("PRB_EDGE_KV_NAMESPACE", "ns".to_string()),
        ("PRB_EDGE_KV_TOKEN", "kvtok".to_string()),
        ("PRB_EDGE_KV_API", format!("http://{addr}/client/v4")),
        ("PRB_EDGE_TOKEN", EDGE.to_string()),
        ("PRB_EDGE_BEACON_URL", format!("http://{addr}/_prb/edge/snapshot")),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let edge = Edge::from_vars(|k| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())).unwrap();
    assert!(pratibimba::edge::publish(&pool, &edge, "demo").await.is_err(), "a failed beacon push was not an error");
    assert_eq!(count(&pool, "SELECT count(*) FROM edge_snapshots").await, 0, "a failed push was recorded as published");
    fail.store(false, std::sync::atomic::Ordering::SeqCst);
    let p = pratibimba::edge::publish(&pool, &edge, "demo").await.unwrap().expect("published");
    assert_eq!(p.version, 1);
    let seen = seen.lock().unwrap();
    let beacon: Vec<_> = seen.iter().filter(|s| s.0 == "/_prb/edge/snapshot").collect();
    let kv: Vec<_> = seen.iter().filter(|s| s.0.contains("/storage/kv/")).collect();
    assert_eq!(beacon.len(), 2, "the beacon was not retried");
    assert!(!kv.is_empty(), "KV got nothing");
    let last = beacon.last().unwrap();
    assert_eq!(last.1, format!("Bearer {EDGE}"));
    assert_eq!(last.2["version"], json!(1));
    assert_eq!(last.2["format"], json!(2));
    assert_eq!(last.2["site"], json!("demo"));
}

#[test]
fn the_beacon_needs_the_edge_token() {
    let get = |k: &str| (k == "PRB_EDGE_BEACON_URL").then(|| "https://x.example/_prb/edge/snapshot".to_string());
    assert!(Edge::from_vars(get).is_err());
    let get = |k: &str| match k {
        "PRB_EDGE_BEACON_URL" => Some("https://x.example/_prb/edge/snapshot".to_string()),
        "PRB_EDGE_TOKEN" => Some("t".to_string()),
        _ => None,
    };
    let e = Edge::from_vars(get).unwrap();
    assert!(e.is_on() && e.has_token());
    let only_token = Edge::from_vars(|k| (k == "PRB_EDGE_TOKEN").then(|| "t".to_string())).unwrap();
    assert!(!only_token.is_on() && only_token.has_token(), "the token alone turns ingest on, not the publisher");
}

#[sqlx::test(migrations = false)]
async fn the_snapshot_carries_the_edge_clock_inputs(pool: PgPool) {
    setup(&pool, "full").await;
    let gift = variant_id(&pool, "gift").await;
    let base = variant_id(&pool, "baseline").await;
    sqlx::query(
        "INSERT INTO experiments (site_id, slot_id, name, metric, arms, starts_at, stops_at, created_by)
         SELECT 'demo', id, 'now', 'click', $1, now() - interval '1 hour', now() + interval '1 hour', 't' FROM slots WHERE key = 'hero'",
    )
    .bind(json!([{ "variant_id": base, "weight": 1 }, { "variant_id": gift, "weight": 1 }]))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO experiments (site_id, slot_id, name, metric, arms, starts_at, stops_at, created_by)
         SELECT 'demo', id, 'later', 'click', $1, now() + interval '3 hours', now() + interval '4 hours', 't' FROM slots WHERE key = 'hero'",
    )
    .bind(json!([{ "variant_id": base, "weight": 1 }, { "variant_id": gift, "weight": 1 }]))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE slots SET decision_ttl_secs = 123").execute(&pool).await.unwrap();
    let store = Arc::new(MemoryStore::default());
    pratibimba::edge::publish(&pool, &Edge::with_store(store.clone()), "demo").await.unwrap();
    let s = store.last("demo").unwrap();
    let hero = &s["slots"]["hero"];
    assert_eq!(s["format"], json!(2));
    assert_eq!(s["settings"]["simulated_clock"], json!(false));
    assert_eq!(hero["decision_ttl_secs"], json!(123));
    let ends = chrono::DateTime::parse_from_rfc3339(hero["experiment"]["ends_at"].as_str().unwrap()).unwrap();
    let next = chrono::DateTime::parse_from_rfc3339(hero["next_experiment_at"].as_str().unwrap()).unwrap();
    let now = chrono::Utc::now();
    assert!((ends.with_timezone(&chrono::Utc) - now - chrono::Duration::hours(1)).num_seconds().abs() < 60, "{ends}");
    assert!((next.with_timezone(&chrono::Utc) - now - chrono::Duration::hours(3)).num_seconds().abs() < 60, "{next}");
    // A sandbox's simulated clock is not one the edge can keep.
    sqlx::query("INSERT INTO sites (id, origin, sandbox_of, sim_now) VALUES ('demo-sim', 'http://localhost:5173', 'demo', now() - interval '1 day')")
        .execute(&pool)
        .await
        .unwrap();
    pratibimba::edge::publish(&pool, &Edge::with_store(store.clone()), "demo-sim").await.unwrap();
    assert_eq!(store.last("demo-sim").unwrap()["settings"]["simulated_clock"], json!(true));
}

/// Mutant c6: a decision made at 23:59:30 and ingested after midnight is filed under the salt of
/// ITS day, not today's. Through the route this is unreachable away from midnight (the age rule
/// refuses a decision from another day first), so the rule is held on `stored_subject` directly.
#[sqlx::test(migrations = false)]
async fn a_decision_from_before_midnight_keeps_its_own_days_salt(pool: PgPool) {
    use chrono::{Duration, Utc};
    use pratibimba::http::edge::stored_subject;
    use pratibimba::privacy::{day_of, pseudonym};
    setup(&pool, "cookieless").await;
    let today = day_of(Utc::now());
    let yesterday = today - Duration::days(1);
    let at = yesterday.and_hms_opt(23, 59, 30).unwrap().and_utc();
    for (day, salt) in [(yesterday, "y".repeat(64)), (today, "t".repeat(64))] {
        sqlx::query("INSERT INTO privacy_salts (site_id, day, salt) VALUES ('demo', $1, $2)").bind(day).bind(salt).execute(&pool).await.unwrap();
    }
    let got = stored_subject(&pool, "demo", "erin", false, at).await.unwrap();
    assert_eq!(got.as_deref(), Some(pseudonym(&"y".repeat(64), "demo", "erin").as_str()), "yesterday's decision takes yesterday's salt");
    let now = stored_subject(&pool, "demo", "erin", false, Utc::now()).await.unwrap();
    assert_eq!(now.as_deref(), Some(pseudonym(&"t".repeat(64), "demo", "erin").as_str()));
    // Identified: the raw id, whatever the day; a day with no salt: none.
    assert_eq!(stored_subject(&pool, "demo", "erin", true, at).await.unwrap().as_deref(), Some("erin"));
    let older = (yesterday - Duration::days(1)).and_hms_opt(12, 0, 0).unwrap().and_utc();
    assert_eq!(stored_subject(&pool, "demo", "erin", false, older).await.unwrap(), None);
}
