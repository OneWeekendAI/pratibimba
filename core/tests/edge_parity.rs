//! EC2 parity: the edge Worker's decide (deploy/proxy-worker/src/kernel.js) against Core's own
//! `/decide`, over thousands of decisions Core really made.
//!
//! Each scenario is a generated site: a privacy mode, a holdout, a field allowlist, declared
//! features (PII and not), a picker, 1-5 arms with traffic shares, audience gates (open, shut,
//! archived, local-time), a running, future or finished experiment, cold start part-way, a
//! learned model with the stated and time extension, sometimes the kill switch. Each request
//! is a generated visitor: fresh or returning (so sticky decisions occur), consent or not, a
//! named zone, an offset or neither, stated picks, declared features, and a share of requests
//! Core refuses (bad zone, undeclared feature, bad offset, unknown slot).
//!
//! For every request the test takes the edge snapshot exactly as Core publishes it
//! (`edge::content`) just before deciding, asks Core through its router with the edge token
//! (so the response carries what the Worker learns: the stored subject and the record), and
//! writes `{snapshot, request, seed, at, stored, sticky, core}`. Then it runs
//! deploy/proxy-worker/test/parity.mjs over the file, which runs the kernel on each case and
//! compares: same variant, payload, holdout flag, allocation, experiment, cold-start turn,
//! audiences, features and policy; scores within 1e-12. A request Core refused must be one the
//! kernel declines to decide (it then asks Core). The node script also reports how many cases
//! took each branch, and the test requires every branch to have been taken, so a generator that
//! stopped producing, say, experiments cannot pass by agreeing on nothing.
//!
//! `PRB_PARITY_SCENARIOS` / `PRB_PARITY_REQUESTS` scale it (default 60 x 60 = 3,600 decisions).
//! `PRB_WRITE_VECTORS=1` also refreshes deploy/proxy-worker/test/vectors.jsonl, the subset the
//! Worker's own `npm test` runs without a database.

mod common;
use common::*;
use pratibimba::bandit::{LinTs, Vector};
use pratibimba::edge::Edge;
use rand::{Rng, SeedableRng, seq::IndexedRandom};
use rand_chacha::ChaCha8Rng;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::io::Write;

const EDGE_TOKEN: &str = "parity-edge-token";
const ZONES: [&str; 12] = [
    "Asia/Kolkata", "Europe/London", "America/New_York", "America/Los_Angeles", "Australia/Sydney", "Asia/Tokyo",
    "Pacific/Chatham", "America/St_Johns", "Asia/Kathmandu", "UTC", "Africa/Cairo", "Europe/Berlin",
];
const PREDICATES: [&str; 9] = [
    r#"{"returning":true}"#,
    r#"{"mobile":false}"#,
    r#"{"from_ad":true,"returning":false}"#,
    r#"{"is_night_local":true}"#,
    r#"{"hour_local":[9,17]}"#,
    r#"{"hour_local":[22,3]}"#,
    r#"{"is_weekend_local":false,"mobile":true}"#,
    r#"{"is_business_hours_local":true}"#,
    r#"{"mobile":true}"#,
];

fn env_n(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

struct Scenario {
    site: String,
    slots: Vec<String>,
    picker_keys: Vec<String>,
    features: Vec<(String, bool)>,
}

fn x_of(rng: &mut ChaCha8Rng) -> Vector {
    [1.0, rng.random_bool(0.5) as u8 as f64, rng.random_bool(0.5) as u8 as f64, rng.random_bool(0.3) as u8 as f64]
}

/// A learned model over `arms`: random observations, some carrying stated, declared and time
/// features, so the extension model exists with a few of its keys (the rest join at decide time).
fn trained(rng: &mut ChaCha8Rng, arms: &[i64], picker: &[String], features: &[(String, bool)]) -> LinTs {
    let mut m = LinTs::default();
    let n = rng.random_range(0..60);
    for _ in 0..n {
        let arm = *arms.choose(rng).unwrap();
        let x = x_of(rng);
        let mut f = pratibimba::http::decide::named(&x);
        if rng.random_bool(0.5) {
            for k in picker {
                f[format!("stated:{k}")] = json!(rng.random_bool(0.4) as u8 as f64);
            }
            for (k, _) in features {
                f[format!("feature:{k}")] = json!(rng.random_bool(0.5) as u8 as f64);
            }
            if rng.random_bool(0.5) {
                let h = rng.random_range(0..24u8);
                let t = pratibimba::insights::localtime::LocalTime::from_parts(h, rng.random_range(0..7), Default::default());
                for (k, v) in pratibimba::stated::time_values(&t) {
                    f[k.trim_start_matches("time:")] = json!(v);
                }
            }
        }
        pratibimba::stated::learn(&mut m, arm, &x, &f, rng.random_bool(0.3));
    }
    m
}

async fn scenario(pool: &PgPool, rng: &mut ChaCha8Rng, n: usize) -> Scenario {
    let site = format!("p{n}");
    let mode = *["full", "cookieless", "consent_aware"].choose(rng).unwrap();
    let holdout = *[0.0f32, 0.1, 0.3, 0.5].choose(rng).unwrap();
    let mut file = json!({ "site": site, "origin": "http://localhost:5173", "holdout_fraction": holdout, "privacy_mode": mode, "slots": [] });
    if rng.random_bool(0.25) {
        let all = ["returning", "mobile", "from_ad", "stated", "tz_offset", "slot", "decision_id"];
        let some: Vec<&str> = all.iter().copied().filter(|_| rng.random_bool(0.6)).collect();
        file["fields_allowed"] = json!(some);
    }
    let mut features = vec![];
    if rng.random_bool(0.4) {
        for (i, k) in ["vip", "email_known", "app_user"].iter().enumerate() {
            if rng.random_bool(0.6) {
                features.push((k.to_string(), i == 1 || rng.random_bool(0.3)));
            }
        }
        file["features"] = json!(features.iter().map(|(k, pii)| json!({ "key": k, "pii": pii })).collect::<Vec<_>>());
    }
    let mut picker_keys = vec![];
    let mut slots = vec![];
    if rng.random_bool(0.25) {
        picker_keys = vec!["books".to_string(), "garden".to_string(), "kitchen".to_string()];
        file["slots"].as_array_mut().unwrap().push(json!({
            "key": "taste", "selector": "#taste", "kind": "picker", "metric": "click",
            "picker": { "max_picks": 3, "options": picker_keys.iter().map(|k| json!({ "key": k, "label": k })).collect::<Vec<_>>() },
            "variants": [ { "key": "none", "baseline": true }, { "key": "picker", "html": "<p>Pick</p>" } ] }));
        slots.push("taste".to_string());
    }
    for s in 0..rng.random_range(1..=2) {
        let key = format!("s{s}");
        let arms = rng.random_range(1..=5);
        let mut variants = vec![json!({ "key": "base", "baseline": true })];
        for a in 1..arms {
            variants.push(json!({ "key": format!("v{a}"), "html": format!("<p>{key} v{a}</p>") }));
        }
        file["slots"].as_array_mut().unwrap().push(json!({
            "key": key, "selector": format!("#{key}"), "metric": "click",
            "decision_ttl_secs": *[0, 60, 600].choose(rng).unwrap(),
            "cold_start_exposures": *[0, 0, 3, 40].choose(rng).unwrap(),
            "variants": variants }));
        slots.push(key);
    }
    let f: pratibimba::site_file::SiteFile = serde_json::from_value(file).unwrap();
    pratibimba::site_file::apply(pool, &f).await.unwrap();
    sqlx::query("UPDATE sites SET night_start_hour = $2, night_end_hour = $3 WHERE id = $1")
        .bind(&site)
        .bind(rng.random_range(18..24i16))
        .bind(rng.random_range(0..8i16))
        .execute(pool)
        .await
        .unwrap();
    if rng.random_bool(0.1) {
        sqlx::query("UPDATE sites SET paused_at = now(), paused_by = 'parity' WHERE id = $1").bind(&site).execute(pool).await.unwrap();
    }
    // Audiences: one or two segments, promoted or archived.
    let mut segs = vec![];
    for i in 0..rng.random_range(0..=2) {
        let p: Value = serde_json::from_str(PREDICATES.choose(rng).unwrap()).unwrap();
        let status = if rng.random_bool(0.8) { "audience" } else { "archived" };
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO segments (site_id, name, predicate, predicate_key, origin, status, promoted_by, promoted_at)
             VALUES ($1, $2, $3, $4, 'authored', $5, 'parity', now()) RETURNING id",
        )
        .bind(&site)
        .bind(format!("seg {i}"))
        .bind(&p)
        .bind(format!("{p}-{i}"))
        .bind(status)
        .fetch_one(pool)
        .await
        .unwrap();
        segs.push(id);
    }
    for slot in &slots {
        let ids: Vec<(i64, bool)> = sqlx::query_as(
            "SELECT v.id, v.is_baseline FROM variants v JOIN slots s ON s.id = v.slot_id WHERE s.site_id = $1 AND s.key = $2 ORDER BY v.id",
        )
        .bind(&site)
        .bind(slot)
        .fetch_all(pool)
        .await
        .unwrap();
        let arm_ids: Vec<i64> = ids.iter().map(|v| v.0).collect();
        for (id, base) in &ids {
            if *base {
                continue;
            }
            if rng.random_bool(0.35) {
                let share = *[0.05f32, 0.1, 0.3, 0.5].choose(rng).unwrap();
                sqlx::query("UPDATE variants SET traffic_share = $2 WHERE id = $1").bind(id).bind(share).execute(pool).await.unwrap();
            }
            if !segs.is_empty() && rng.random_bool(0.4) {
                sqlx::query("UPDATE variants SET audience_id = $2 WHERE id = $1").bind(id).bind(segs.choose(rng).unwrap()).execute(pool).await.unwrap();
            }
        }
        let model = trained(rng, &arm_ids, &picker_keys, &features);
        let version = rng.random_range(1..500i64);
        sqlx::query("UPDATE models m SET state = $3, version = $4 FROM slots s WHERE m.slot_id = s.id AND s.site_id = $1 AND s.key = $2")
            .bind(&site)
            .bind(slot)
            .bind(serde_json::to_value(&model).unwrap())
            .bind(version)
            .execute(pool)
            .await
            .unwrap();
        // An experiment: running (now), finished, or due to start.
        if arm_ids.len() >= 2 && rng.random_bool(0.45) {
            let mut arms: Vec<i64> = arm_ids.iter().copied().filter(|_| rng.random_bool(0.7)).collect();
            if arms.len() < 2 {
                arms = arm_ids[..2].to_vec();
            }
            let weights: Vec<Value> = arms.iter().map(|a| json!({ "variant_id": a, "weight": *[1.0, 2.0, 0.5].choose(rng).unwrap() })).collect();
            let (start, stop) = *[("-1 hour", "+1 hour"), ("-1 hour", "+1 hour"), ("-2 hours", "-1 hour"), ("+1 hour", "+2 hours")].choose(rng).unwrap();
            sqlx::query(&format!(
                "INSERT INTO experiments (site_id, slot_id, name, metric, arms, starts_at, stops_at, created_by)
                 SELECT $1, s.id, 'exp', 'click', $3, now() + interval '{start}', now() + interval '{stop}', 'parity' FROM slots s WHERE s.site_id = $1 AND s.key = $2"
            ))
            .bind(&site)
            .bind(slot)
            .bind(json!(weights))
            .execute(pool)
            .await
            .unwrap();
        }
    }
    Scenario { site, slots, picker_keys, features }
}

fn ctx(rng: &mut ChaCha8Rng, s: &Scenario) -> Value {
    let mut c = serde_json::Map::new();
    for k in ["returning", "mobile", "from_ad"] {
        if rng.random_bool(0.6) {
            c.insert(k.into(), json!(rng.random_bool(0.5)));
        }
    }
    match rng.random_range(0..10) {
        0..=3 => {
            c.insert("tz".into(), json!(ZONES.choose(rng).unwrap()));
        }
        4..=5 => {
            c.insert("tz_offset".into(), json!(*[-300i32, 0, 330, 345, 540, 600, -210].choose(rng).unwrap()));
        }
        6 if rng.random_bool(0.15) => {
            // Refused by Core.
            if rng.random_bool(0.5) {
                c.insert("tz".into(), json!("Mars/Olympus"));
            } else {
                c.insert("tz_offset".into(), json!(900));
            }
        }
        _ => {}
    }
    match rng.random_range(0..3) {
        0 => {}
        1 => {
            c.insert("consent".into(), json!(true));
        }
        _ => {
            c.insert("consent".into(), json!(false));
        }
    }
    if !s.picker_keys.is_empty() && rng.random_bool(0.6) {
        let mut picks: Vec<&String> = s.picker_keys.iter().filter(|_| rng.random_bool(0.4)).collect();
        let bad = "lamps".to_string();
        if rng.random_bool(0.05) {
            picks.push(&bad);
        }
        c.insert("stated".into(), json!(picks));
    }
    if !s.features.is_empty() && rng.random_bool(0.7) {
        let mut f = serde_json::Map::new();
        for (k, _) in &s.features {
            if rng.random_bool(0.6) {
                f.insert(k.clone(), json!(rng.random_bool(0.5)));
            }
        }
        if rng.random_bool(0.04) {
            f.insert("undeclared".into(), json!(true));
        }
        c.insert("features".into(), Value::Object(f));
    }
    Value::Object(c)
}

fn hex_json(h: Option<&axum::http::HeaderValue>) -> Option<Value> {
    let s = h?.to_str().ok()?;
    let bytes: Vec<u8> = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect();
    serde_json::from_slice(&bytes).ok()
}

fn worker_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../deploy/proxy-worker")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_edge_kernel_decides_as_core_does() {
    let db = FreshDb::new().await;
    let pool = db.pool.clone();
    pratibimba::db::migrate(&pool).await.unwrap();
    let edge = Edge::off().with_token(EDGE_TOKEN);
    let state = pratibimba::http::AppState::configured_with_edge(pool.clone(), Some(2026), None, None, edge);
    let app = pratibimba::http::router(state, &[]);
    let mut rng = ChaCha8Rng::seed_from_u64(env_n("PRB_PARITY_SEED", 2026) as u64);
    let (scenarios, requests) = (env_n("PRB_PARITY_SCENARIOS", 60), env_n("PRB_PARITY_REQUESTS", 60));
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/edge-parity");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cases.jsonl");
    let mut out = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
    let zones: Vec<&str> = chrono_tz::TZ_VARIANTS.iter().map(|t| t.name()).collect();
    writeln!(out, "{}", json!({ "zones": zones })).unwrap();
    let mut written = 0;
    for n in 0..scenarios {
        let s = scenario(&pool, &mut rng, n).await;
        // What the Worker would have cached from Core's answers: (slot, stored subject) -> record.
        let mut sticky: std::collections::HashMap<(String, String), Value> = Default::default();
        let mut subjects: Vec<String> = vec![];
        for r in 0..requests {
            let subject = if !subjects.is_empty() && rng.random_bool(0.2) { subjects.choose(&mut rng).unwrap().clone() } else { format!("v-{n}-{r}") };
            subjects.push(subject.clone());
            let c = ctx(&mut rng, &s);
            let batch = s.slots.len() > 1 && rng.random_bool(0.2);
            let slots: Vec<String> = if batch {
                s.slots.clone()
            } else if rng.random_bool(0.03) {
                vec!["nope".to_string()]
            } else {
                vec![s.slots.choose(&mut rng).unwrap().clone()]
            };
            // An experiment stops early, or a new one starts: a sticky decision made under the
            // old one must not survive it.
            if rng.random_bool(0.04) {
                if rng.random_bool(0.5) {
                    sqlx::query("UPDATE experiments SET stopped_at = now(), stopped_by = 'parity' WHERE site_id = $1 AND stopped_at IS NULL AND starts_at <= now()")
                        .bind(&s.site)
                        .execute(&pool)
                        .await
                        .unwrap();
                } else {
                    sqlx::query(
                        "INSERT INTO experiments (site_id, slot_id, name, metric, arms, starts_at, stops_at, created_by)
                         SELECT s.site_id, s.id, 'late', 'click', (SELECT jsonb_agg(jsonb_build_object('variant_id', v.id, 'weight', 1) ORDER BY v.id) FROM variants v WHERE v.slot_id = s.id),
                                now() - interval '1 second', now() + interval '1 hour', 'parity'
                           FROM slots s WHERE s.site_id = $1 AND (SELECT count(*) FROM variants v WHERE v.slot_id = s.id) >= 2",
                    )
                    .bind(&s.site)
                    .execute(&pool)
                    .await
                    .unwrap();
                }
            }
            // Move cold start along: mark some decisions exposed.
            if rng.random_bool(0.3) {
                sqlx::query("UPDATE decisions SET exposed_at = now() WHERE id IN (SELECT id FROM decisions WHERE site_id = $1 AND exposed_at IS NULL LIMIT 2)")
                    .bind(&s.site)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            let mut snapshot = pratibimba::edge::content(&pool, &s.site).await.unwrap().unwrap();
            snapshot["version"] = json!(1);
            snapshot["generated_at"] = json!("2026-10-13T00:00:00.000Z");
            // The subject as Core stores it for this request (what the Worker learns on a miss).
            let mode = snapshot["settings"]["privacy_mode"].as_str().unwrap().to_string();
            let consent = c.get("consent").and_then(Value::as_bool);
            let identified = mode == "full" || (mode == "consent_aware" && consent == Some(true));
            let stored = if identified {
                subject.clone()
            } else {
                let salt = pratibimba::privacy::salt(&pool, &s.site, pratibimba::privacy::day_of(chrono::Utc::now())).await.unwrap();
                pratibimba::privacy::pseudonym(&salt, &s.site, &subject)
            };
            let stickies: Vec<Value> = slots.iter().map(|sl| sticky.get(&(sl.clone(), stored.clone())).cloned().unwrap_or(Value::Null)).collect();
            let (path, body) = if batch {
                ("/decide/batch", json!({ "site": s.site, "subject": subject, "slots": slots, "ctx": c }))
            } else {
                ("/decide", json!({ "site": s.site, "slot": slots[0], "subject": subject, "ctx": c }))
            };
            let res = send(
                &app,
                axum::http::Request::post(path)
                    .header("content-type", "application/json")
                    .header("x-prb-edge-token", EDGE_TOKEN)
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await;
            let learned = hex_json(res.headers.get("x-prb-edge"));
            if let Some(l) = &learned {
                assert_eq!(l["subject"], json!(stored), "Core told the edge a different stored subject");
                for rec in l["records"].as_array().unwrap() {
                    // Which slot: the response says, by decision id.
                    let body = res.json();
                    let all = if batch { body["decisions"].as_array().unwrap().clone() } else { vec![body.clone()] };
                    if let Some(d) = all.iter().find(|d| d["decision_id"] == rec["id"]) {
                        sticky.insert((d["slot"].as_str().unwrap().to_string(), stored.clone()), rec.clone());
                    }
                }
            }
            let at = learned.as_ref().map(|l| l["at"].clone()).unwrap_or(json!(chrono::Utc::now().to_rfc3339()));
            let case = json!({
                "snapshot": snapshot, "path": path, "request": body, "stored": stored, "identified": identified,
                "at": at, "sticky": stickies,
                "core": { "status": res.status.as_u16(), "body": serde_json::from_str::<Value>(&res.text).unwrap_or(Value::Null) },
            });
            writeln!(out, "{case}").unwrap();
            written += 1;
        }
    }
    out.flush().unwrap();
    drop(out);
    assert_eq!(written, scenarios * requests);

    let run = std::process::Command::new("node").arg(worker_dir().join("test/parity.mjs")).arg(&path).output().expect("node must be installed");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(run.status.success(), "parity.mjs failed:\n{stdout}\n{}", String::from_utf8_lossy(&run.stderr));
    let report: Value = serde_json::from_str(stdout.lines().last().unwrap()).unwrap();
    eprintln!("parity: {report}");
    assert_eq!(report["mismatches"], json!(0), "the edge decided differently from Core: {}", report["examples"]);
    assert_eq!(report["cases"].as_u64().unwrap() as usize, written);
    assert_eq!(report["order_ok"], json!(true), "the extended sampler's scores are off in the last bit more often than exp alone explains: {}", report["inexact_by_policy"]);
    assert_eq!(report["zones_match"], json!(true), "deploy/proxy-worker/src/tables.js ZONES differs from chrono-tz's list");
    // Every branch was taken, so agreement is not agreement on nothing.
    for (branch, min) in [
        ("decided", 1500),
        ("base_lints", 50),
        ("stated", 30),
        ("features", 30),
        ("time", 200),
        ("holdout", 100),
        ("experiment", 100),
        ("cold_start", 50),
        ("gate_shut", 50),
        ("gate_open", 50),
        ("share_excluded", 50),
        ("sticky", 50),
        ("paused", 20),
        ("cookieless", 300),
        ("identified", 300),
        ("batch", 50),
        ("core_refused", 50),
        ("zone", 200),
        ("experiment_clock", 20),
    ] {
        let got = report["coverage"][branch].as_u64().unwrap_or(0);
        assert!(got >= min, "only {got} cases took branch {branch} (need {min}): {}", report["coverage"]);
    }
    if std::env::var("PRB_WRITE_VECTORS").is_ok() {
        // Every 25th case, so the Worker's own suite holds a spread of them without a database.
        let all = std::fs::read_to_string(&path).unwrap();
        let mut lines = all.lines();
        let mut kept = vec![lines.next().unwrap().to_string()];
        // The kernel reads neither the manifest nor the plugins: dropped to keep the file small.
        kept.extend(lines.enumerate().filter(|(i, _)| i % 40 == 0).map(|(_, l)| {
            let mut v: Value = serde_json::from_str(l).unwrap();
            let o = v["snapshot"].as_object_mut().unwrap();
            o.remove("manifest");
            o.remove("plugins");
            v.to_string()
        }));
        std::fs::write(worker_dir().join("test/vectors.jsonl"), kept.join("\n") + "\n").unwrap();
    }
    db.drop().await;
}
