//! The insights pipeline: local time, rollups, discovery, retention, journeys, and the rule that
//! the insight routes read rollups only.
//!
//! The corpus has a planted pattern: on the demo hero slot, visitors whose LOCAL hour is in the
//! night window (22:00-05:00) convert at 40% on `gift` and 10% on the baseline; everyone else
//! converts at 10% on both. Visitors come from four UTC offsets, so the pattern is sharp on the
//! local axis and smeared on the UTC one.

mod common;
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use common::*;
use pratibimba::hash::unit;
use pratibimba::insights::{
    self, discover,
    localtime::{LocalTime, NightWindow},
    rollup,
    segments::Predicate,
};
use serde_json::{Value, json};
use sqlx::PgPool;

const OFFSETS: [i16; 4] = [330, -300, 60, 540];

fn u(tag: &str, i: i64) -> f64 {
    unit(&[tag.as_bytes(), &i.to_le_bytes()])
}

fn base() -> DateTime<Utc> {
    let d = (Utc::now() - Duration::days(16)).date_naive();
    Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
}

/// `n` exposed, settled decisions on demo/hero, each with an exposure event carrying its offset.
async fn seed_corpus(pool: &PgPool, n: i64, planted: bool) {
    let (mut ids, mut subjects, mut variants, mut ret, mut mob, mut ad, mut at, mut outcome, mut tz) =
        (vec![], vec![], vec![], vec![], vec![], vec![], vec![], vec![], vec![]);
    let baseline = variant_id(pool, "baseline").await;
    let gift = variant_id(pool, "gift").await;
    for i in 0..n {
        let off = OFFSETS[(u("tz", i) * 4.0) as usize];
        let h = (u("h", i) * 24.0) as i64;
        let k = (u("d", i) * 14.0) as i64;
        let local = base() + Duration::days(k) + Duration::hours(h) + Duration::minutes(7);
        let utc = local - Duration::minutes(off as i64);
        let is_gift = u("v", i) < 0.5;
        let night = h >= 22 || h < 5;
        let p = if planted && night && is_gift { 0.40 } else { 0.10 };
        ids.push(uuid::Uuid::new_v4());
        subjects.push(format!("v{i}"));
        variants.push(if is_gift { gift } else { baseline });
        ret.push(u("r", i) < 0.4);
        mob.push(u("m", i) < 0.5);
        ad.push(u("a", i) < 0.2);
        at.push(utc);
        outcome.push(if u("c", i) < p { 1i16 } else { 0 });
        tz.push(off);
    }
    sqlx::query(
        "INSERT INTO decisions (id, site_id, slot_id, variant_id, subject, pool, features, scores, policy, policy_version, seed,
                                created_at, exposed_at, outcome, outcome_at, settled_at)
         SELECT id, 'demo', (SELECT id FROM slots WHERE key = 'hero'), v, s, '{}',
                jsonb_build_object('bias', 1, 'returning', r::int, 'mobile', m::int, 'from_ad', a::int), '{}', 'lints', 0, 0,
                t, t, o, t + interval '1 hour', t + interval '1 hour'
           FROM UNNEST($1::uuid[], $2::text[], $3::int8[], $4::bool[], $5::bool[], $6::bool[], $7::timestamptz[], $8::int2[])
                AS x(id, s, v, r, m, a, t, o)",
    )
    .bind(&ids)
    .bind(&subjects)
    .bind(&variants)
    .bind(&ret)
    .bind(&mob)
    .bind(&ad)
    .bind(&at)
    .bind(&outcome)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO events (id, site_id, subject, name, slot_id, decision_id, received_at, tz_offset_min)
         SELECT gen_random_uuid(), 'demo', s, 'exposure', (SELECT id FROM slots WHERE key = 'hero'), d, t, z
           FROM UNNEST($1::uuid[], $2::text[], $3::timestamptz[], $4::int2[]) AS x(d, s, t, z)",
    )
    .bind(&ids)
    .bind(&subjects)
    .bind(&at)
    .bind(&tz)
    .execute(pool)
    .await
    .unwrap();
}

async fn corpus(pool: &PgPool, planted: bool) {
    setup_demo(pool).await;
    seed_corpus(pool, 8000, planted).await;
    insights::nightly(pool, Utc::now()).await.unwrap();
}

fn cell<'a>(h: &'a Value, hour: i64, variant: &str) -> &'a Value {
    h["cells"].as_array().unwrap().iter().find(|c| c["hour"] == hour && c["variant"] == variant).unwrap_or_else(|| panic!("no cell {hour}/{variant}"))
}

/// Pooled rate over a set of hours.
fn rate(h: &Value, hours: &[i64], variant: &str) -> f64 {
    let (mut n, mut c) = (0.0, 0.0);
    for hr in hours {
        let x = cell(h, *hr, variant);
        n += x["exposures"].as_f64().unwrap();
        c += x["conversions"].as_f64().unwrap();
    }
    c / n
}

const NIGHT: [i64; 7] = [22, 23, 0, 1, 2, 3, 4];
const DAY: [i64; 8] = [9, 10, 11, 12, 13, 14, 15, 16];

// ---- local time -----------------------------------------------------------------------------

#[test]
fn local_time_applies_the_offset_and_wraps_the_day() {
    let night = NightWindow::default();
    // 20:00 UTC on Saturday 2026-10-03 is 01:30 Sunday in India.
    let t = Utc.with_ymd_and_hms(2026, 10, 3, 20, 0, 0).unwrap();
    let ist = LocalTime::at(t, Some(330), night);
    assert_eq!((ist.hour_local, ist.dow_local), (1, 0));
    assert!(ist.is_night_local && ist.is_weekend_local && !ist.is_business_hours_local);
    // The same instant in New York is 16:00 Saturday.
    let ny = LocalTime::at(t, Some(-240), night);
    assert_eq!((ny.hour_local, ny.dow_local, ny.is_night_local), (16, 6, false));
    // No offset (or a nonsense one) is UTC.
    assert_eq!(LocalTime::at(t, None, night).hour_local, 20);
    assert_eq!(LocalTime::at(t, Some(5000), night).hour_local, 20);
    // Business hours: a weekday, 09-17.
    let tue = Utc.with_ymd_and_hms(2026, 10, 6, 10, 0, 0).unwrap();
    assert!(LocalTime::at(tue, Some(0), night).is_business_hours_local);
}

#[test]
fn night_window_wraps_midnight_and_is_configurable() {
    let n = NightWindow::default();
    let inside: Vec<u8> = (0..24).filter(|h| n.contains(*h)).collect();
    assert_eq!(inside, vec![0, 1, 2, 3, 4, 22, 23]);
    let early = NightWindow { start: 1, end: 4 };
    assert_eq!((0..24).filter(|h| early.contains(*h)).collect::<Vec<u8>>(), vec![1, 2, 3]);
}

#[test]
fn cyclic_hour_puts_midnight_next_to_23_and_1() {
    let f = |h| LocalTime::from_parts(h, 2, NightWindow::default());
    let dist = |a: LocalTime, b: LocalTime| ((a.sin_hour - b.sin_hour).powi(2) + (a.cos_hour - b.cos_hour).powi(2)).sqrt();
    assert!((dist(f(0), f(23)) - dist(f(0), f(1))).abs() < 1e-9);
    assert!(dist(f(0), f(23)) < dist(f(0), f(12)) / 3.0);
    assert!((f(6).sin_hour - 1.0).abs() < 1e-9 && (f(0).cos_hour - 1.0).abs() < 1e-9);
}

// ---- predicates -----------------------------------------------------------------------------

#[test]
fn predicates_refuse_demographic_and_unknown_features() {
    for bad in ["gender", "age", "income", "ethnicity", "religion"] {
        let e = Predicate::parse(&json!({ bad: "x" })).unwrap_err();
        assert!(e.contains("demographic"), "{bad}: {e}");
    }
    assert!(Predicate::parse(&json!({ "favourite_colour": true })).unwrap_err().contains("unknown feature"));
    assert!(Predicate::parse(&json!({})).is_err());
    assert!(Predicate::parse(&json!({ "hour_local": [22, 24] })).is_err());
    let a = Predicate::parse(&json!({ "returning": true, "is_night_local": true })).unwrap();
    let b = Predicate::parse(&json!({ "is_night_local": true, "returning": true })).unwrap();
    assert_eq!(a.key(), b.key(), "one predicate, one segment");
    assert_eq!(a.describe(), "Night-time, returning visitors");
}

#[sqlx::test(migrations = false)]
async fn creating_a_demographic_segment_is_refused(pool: PgPool) {
    setup_demo(&pool).await;
    let app = admin_app(&pool);
    let r = api_post(&app, "/api/sites/demo/segments", json!({ "predicate": { "gender": "female" }, "actor": "ritwik" })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    assert_eq!(count(&pool, "SELECT count(*) FROM segments").await, 0);
}

// ---- ingest ---------------------------------------------------------------------------------

#[sqlx::test(migrations = false)]
async fn events_keep_the_reported_offset(pool: PgPool) {
    setup_demo(&pool).await;
    let app = app(&pool);
    let ev = |tz: Value| json!({ "site": "demo", "subject": "s1", "tz_offset": tz, "events": [{ "id": uuid::Uuid::new_v4(), "name": "click" }] });
    assert_eq!(post(&app, "/e", ev(json!(330))).await.status, StatusCode::ACCEPTED);
    assert_eq!(post(&app, "/e", ev(Value::Null)).await.status, StatusCode::ACCEPTED);
    assert_eq!(post(&app, "/e", ev(json!(900))).await.status, StatusCode::BAD_REQUEST);
    let got: Vec<Option<i16>> = sqlx::query_scalar("SELECT tz_offset_min FROM events ORDER BY tz_offset_min NULLS LAST").fetch_all(&pool).await.unwrap();
    assert_eq!(got, vec![Some(330), None]);
}

// ---- rollups and the heatmap ----------------------------------------------------------------

#[sqlx::test(migrations = false)]
async fn local_heatmap_surfaces_the_night_gift_pattern(pool: PgPool) {
    corpus(&pool, true).await;
    let app = admin_app(&pool);
    let h = api_get(&app, "/api/sites/demo/insights/heatmap?axis=local").await.json();
    let total: i64 = h["cells"].as_array().unwrap().iter().map(|c| c["exposures"].as_i64().unwrap()).sum();
    assert_eq!(total, 8000, "every exposure lands in exactly one segment-0 cell");
    assert_eq!(h["tz_known_share"], 1.0);
    let (gn, bn, gd, bd) = (rate(&h, &NIGHT, "gift"), rate(&h, &NIGHT, "baseline"), rate(&h, &DAY, "gift"), rate(&h, &DAY, "baseline"));
    assert!(gn > 0.33 && gn < 0.47, "gift at local night: {gn}");
    assert!(bn < 0.15 && gd < 0.15 && bd < 0.15, "{bn} {gd} {bd}");
    // On the UTC axis the four offsets smear it: no seven-hour block reaches the local figure.
    let utc = api_get(&app, "/api/sites/demo/insights/heatmap?axis=utc").await.json();
    let best_block = (0..24).map(|s| rate(&utc, &(0..7).map(|k| (s + k) % 24).collect::<Vec<_>>(), "gift")).fold(0.0, f64::max);
    assert!(best_block < gn - 0.08, "UTC axis looked as sharp as local: {best_block} vs {gn}");
    // Daily rollups agree with hourly.
    assert_eq!(count(&pool, "SELECT sum(n)::int8 FROM rollup_daily WHERE segment_id = 0").await, 8000);
    // Rollups are partitioned by month.
    assert!(count(&pool, "SELECT count(*) FROM pg_inherits WHERE inhparent = 'rollup_hourly'::regclass").await >= 1);
}

#[sqlx::test(migrations = false)]
async fn dow_hour_heatmap_and_bad_params(pool: PgPool) {
    corpus(&pool, true).await;
    let app = admin_app(&pool);
    let h = api_get(&app, "/api/sites/demo/insights/heatmap?by=dow_hour&slot=hero").await.json();
    assert!(h["cells"].as_array().unwrap().iter().all(|c| c["dow"].as_i64().is_some_and(|d| (0..7).contains(&d))));
    assert_eq!(api_get(&app, "/api/sites/demo/insights/heatmap?axis=mars").await.status, StatusCode::BAD_REQUEST);
    assert_eq!(api_get(&app, "/api/sites/nope/insights/heatmap").await.status, StatusCode::NOT_FOUND);
    assert_eq!(api_get(&app, "/api/sites/demo/insights/heatmap?slot=nope").await.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = false)]
async fn rollups_rebuild_idempotently_and_follow_late_settlement(pool: PgPool) {
    setup_demo(&pool).await;
    seed_corpus(&pool, 400, false).await;
    // 50 of them still inside their window.
    sqlx::query("UPDATE decisions SET outcome = NULL, outcome_at = NULL, settled_at = NULL WHERE subject IN (SELECT subject FROM decisions ORDER BY subject LIMIT 50)")
        .execute(&pool)
        .await
        .unwrap();
    let t0 = Utc::now();
    rollup::run(&pool, t0).await.unwrap();
    let sum = |o: &'static str| {
        let pool = pool.clone();
        async move { count(&pool, &format!("SELECT coalesce(sum(n), 0)::int8 FROM rollup_hourly WHERE segment_id = 0 AND outcome = '{o}'")).await }
    };
    assert_eq!(sum("pending").await, 50);
    rollup::run(&pool, t0 + Duration::seconds(1)).await.unwrap();
    assert_eq!(count(&pool, "SELECT sum(n)::int8 FROM rollup_hourly WHERE segment_id = 0").await, 400, "a second run double-counted");
    // They settle later: 50 conversions arrive after the first run.
    let t1 = t0 + Duration::hours(2);
    sqlx::query("UPDATE decisions SET outcome = 1, outcome_at = $1, settled_at = $1 WHERE outcome IS NULL AND settled_at IS NULL").bind(t1).execute(&pool).await.unwrap();
    rollup::run(&pool, t1 + Duration::minutes(1)).await.unwrap();
    assert_eq!(sum("pending").await, 0);
    assert_eq!(count(&pool, "SELECT sum(n)::int8 FROM rollup_hourly WHERE segment_id = 0").await, 400);
    assert_eq!(count(&pool, "SELECT sum(n)::int8 FROM rollup_daily WHERE segment_id = 0 AND outcome = 'converted'").await, sum("converted").await);
}

// ---- discovery and promotion ----------------------------------------------------------------

#[sqlx::test(migrations = false)]
async fn discovery_names_the_planted_night_segment(pool: PgPool) {
    corpus(&pool, true).await;
    let app = admin_app(&pool);
    let segs = api_get(&app, "/api/sites/demo/segments").await.json();
    let segs = segs["segments"].as_array().unwrap();
    assert!(!segs.is_empty(), "nothing discovered");
    let top = segs.iter().max_by(|a, b| a["evidence"]["score"].as_f64().unwrap().total_cmp(&b["evidence"]["score"].as_f64().unwrap())).unwrap();
    assert_eq!(top["predicate"], json!({ "is_night_local": true }), "{top:#}");
    assert_eq!((top["status"].as_str(), top["origin"].as_str()), (Some("draft"), Some("discovered")));
    assert_eq!(top["name"], "Night-time visitors");
    assert_eq!(top["evidence"]["kind"], "variant_lift");
    assert_eq!(top["evidence"]["variant"], variant_id(&pool, "gift").await);
    // Durable: a second night refreshes the drafts instead of adding copies.
    let before = count(&pool, "SELECT count(*) FROM segments").await;
    discover::run(&pool, Utc::now()).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM segments").await, before);
    // Nothing context-shaped that was not planted came up.
    for s in segs {
        let p = s["predicate"].as_object().unwrap();
        assert!(!p.contains_key("returning") && !p.contains_key("mobile") && !p.contains_key("from_ad"), "spurious: {s}");
    }
}

#[sqlx::test(migrations = false)]
async fn discovery_finds_nothing_without_a_pattern(pool: PgPool) {
    corpus(&pool, false).await;
    assert_eq!(count(&pool, "SELECT count(*) FROM segments").await, 0);
}

#[sqlx::test(migrations = false)]
async fn promoting_a_draft_makes_an_audience_and_backfills_it(pool: PgPool) {
    corpus(&pool, true).await;
    let app = admin_app(&pool);
    let id: i64 = sqlx::query_scalar("SELECT id FROM segments WHERE predicate = '{\"is_night_local\": true}'").fetch_one(&pool).await.unwrap();
    let path = format!("/api/segments/{id}/promote");
    assert_eq!(api_post(&app, &path, json!({})).await.status, StatusCode::BAD_REQUEST, "promotion needs a named actor");
    let r = api_post(&app, &path, json!({ "actor": "ritwik" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!((r.json()["status"].as_str(), r.json()["promoted_by"].as_str()), (Some("audience"), Some("ritwik")));
    assert_eq!(api_post(&app, &path, json!({ "actor": "ritwik" })).await.status, StatusCode::CONFLICT);
    // The status route shows the backfill pending; runs then fill it in day chunks.
    let st = api_get(&app, "/api/sites/demo/insights/status").await.json();
    assert_eq!(st["backfills"].as_array().unwrap().len(), 1);
    let mut now = Utc::now();
    for _ in 0..5 {
        now += Duration::seconds(1);
        rollup::run(&pool, now).await.unwrap();
    }
    let st = api_get(&app, "/api/sites/demo/insights/status").await.json();
    assert!(st["backfills"].as_array().unwrap().is_empty(), "{st}");
    let h = api_get(&app, &format!("/api/sites/demo/insights/heatmap?segment={id}")).await.json();
    let hours: std::collections::BTreeSet<i64> = h["cells"].as_array().unwrap().iter().map(|c| c["hour"].as_i64().unwrap()).collect();
    assert_eq!(hours, NIGHT.iter().copied().collect(), "the segment's rollup holds exactly the night hours");
    let n: i64 = h["cells"].as_array().unwrap().iter().map(|c| c["exposures"].as_i64().unwrap()).sum();
    let all = api_get(&app, "/api/sites/demo/insights/heatmap").await.json();
    let night_all: i64 = all["cells"].as_array().unwrap().iter().filter(|c| NIGHT.contains(&c["hour"].as_i64().unwrap())).map(|c| c["exposures"].as_i64().unwrap()).sum();
    assert_eq!(n, night_all);
    // Retention by segment: every corpus visitor is seen once, so the segment's cohorts hold
    // exactly its night visitors.
    insights::retention(&pool).await.unwrap();
    let r = api_get(&app, &format!("/api/sites/demo/insights/retention?segment={id}")).await.json();
    let size: i64 = r["cohorts"].as_array().unwrap().iter().map(|c| c["size"].as_i64().unwrap()).sum();
    assert_eq!(size, night_all, "{r}");
    // Archiving drops it from the list; it cannot be promoted again.
    api_post(&app, &format!("/api/segments/{id}/archive"), json!({ "actor": "ritwik" })).await;
    assert_eq!(api_post(&app, &path, json!({ "actor": "ritwik" })).await.status, StatusCode::CONFLICT);
}

#[sqlx::test(migrations = false)]
async fn authored_segment_is_active_and_counted(pool: PgPool) {
    corpus(&pool, true).await;
    let app = admin_app(&pool);
    let r = api_post(&app, "/api/sites/demo/segments", json!({ "predicate": { "hour_local": [22, 4] }, "actor": "ritwik", "name": "Late" })).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    assert_eq!(r.json()["status"], "active");
    let again = api_post(&app, "/api/sites/demo/segments", json!({ "predicate": { "hour_local": [22, 4] }, "actor": "ritwik" })).await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    for k in 1..=4 {
        rollup::run(&pool, Utc::now() + Duration::seconds(k)).await.unwrap();
    }
    let segs = api_get(&app, "/api/sites/demo/segments").await.json();
    let late = segs["segments"].as_array().unwrap().iter().find(|s| s["name"] == "Late").unwrap().clone();
    let night_all = count(&pool, "SELECT sum(n)::int8 FROM rollup_hourly WHERE segment_id = 0 AND hour_local IN (22,23,0,1,2,3,4) AND outcome <> 'pending'").await;
    assert_eq!(late["last_28_days"]["exposures"].as_i64(), Some(night_all));
}

// ---- retention and journeys -----------------------------------------------------------------

fn monday_weeks_ago(w: i64) -> NaiveDate {
    let d = Utc::now().date_naive() - Duration::weeks(w);
    d - Duration::days(chrono::Datelike::weekday(&d).num_days_from_monday() as i64)
}

#[sqlx::test(migrations = false)]
async fn retention_matrix_counts_cohorts_and_returns(pool: PgPool) {
    setup_demo(&pool).await;
    let w0 = monday_weeks_ago(4);
    let at = |week: i64| Utc.from_utc_datetime(&(w0 + Duration::weeks(week)).and_hms_opt(12, 0, 0).unwrap());
    let mut rows: Vec<(String, DateTime<Utc>)> = Vec::new();
    for i in 1..=10 {
        rows.push((format!("a{i}"), at(0)));
        if i <= 4 { rows.push((format!("a{i}"), at(1))); }
        if i <= 2 { rows.push((format!("a{i}"), at(2))); rows.push((format!("a{i}"), at(2) + Duration::hours(1))); }
    }
    for i in 1..=5 {
        rows.push((format!("b{i}"), at(1)));
    }
    for (s, t) in rows {
        sqlx::query("INSERT INTO events (id, site_id, subject, name, received_at) VALUES (gen_random_uuid(), 'demo', $1, 'view', $2)").bind(s).bind(t).execute(&pool).await.unwrap();
    }
    insights::nightly(&pool, Utc::now()).await.unwrap();
    let r = api_get(&admin_app(&pool), "/api/sites/demo/insights/retention").await.json();
    assert_eq!(
        r["cohorts"],
        json!([
            { "cohort_week": w0, "size": 10, "returned": [{ "week": 1, "visitors": 4 }, { "week": 2, "visitors": 2 }] },
            { "cohort_week": w0 + Duration::weeks(1), "size": 5, "returned": [] },
        ])
    );
}

#[sqlx::test(migrations = false)]
async fn journeys_group_paths_by_outcome(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    pratibimba::site_file::apply(&pool, &file_from(json!({ "site": "demo", "origin": "http://localhost:5173", "slots": [
        { "key": "hero", "selector": "#hero", "metric": "click", "variants": [{ "key": "baseline", "baseline": true }, { "key": "gift", "html": "<h1>G</h1>" }] },
        { "key": "cta", "selector": "#cta", "metric": "click", "variants": [{ "key": "plain", "baseline": true }, { "key": "bold", "html": "<b>B</b>" }] } ] })))
        .await
        .unwrap();
    let t = Utc::now() - Duration::days(2);
    // 6 visitors see hero:gift then cta:bold and convert; 3 see hero:baseline then cta:plain and do not.
    let mut k = 0;
    for (n, steps, conv) in [(6, [("hero", "gift"), ("cta", "bold")], 1i16), (3, [("hero", "baseline"), ("cta", "plain")], 0)] {
        for i in 0..n {
            for (j, (slot, var)) in steps.iter().enumerate() {
                k += 1;
                sqlx::query(
                    "INSERT INTO decisions (id, site_id, slot_id, variant_id, subject, pool, features, scores, policy, policy_version, seed,
                                            created_at, exposed_at, outcome, settled_at)
                     SELECT gen_random_uuid(), 'demo', s.id, v.id, $1, '{}', '{}', '{}', 'lints', 0, 0, $2, $2, $3, $2
                       FROM slots s JOIN variants v ON v.slot_id = s.id WHERE s.key = $4 AND v.key = $5",
                )
                .bind(format!("j{conv}-{i}"))
                .bind(t + Duration::minutes(10 * j as i64 + k % 3))
                .bind(if j == 1 { conv } else { 0 })
                .bind(slot)
                .bind(var)
                .execute(&pool)
                .await
                .unwrap();
            }
        }
    }
    insights::nightly(&pool, Utc::now()).await.unwrap();
    let app = admin_app(&pool);
    let conv = api_get(&app, "/api/sites/demo/insights/journeys").await.json();
    let p = &conv["paths"].as_array().unwrap()[0];
    assert_eq!(p["visitors"], 6);
    let steps: Vec<(String, String)> = p["steps"].as_array().unwrap().iter().map(|s| (s["slot"].as_str().unwrap().into(), s["variant"].as_str().unwrap().into())).collect();
    assert_eq!(steps, vec![("hero".into(), "gift".into()), ("cta".into(), "bold".into())]);
    let bounced = api_get(&app, "/api/sites/demo/insights/journeys?outcome=bounced").await.json();
    assert_eq!(bounced["paths"].as_array().unwrap().len(), 1);
    assert_eq!(bounced["paths"][0]["visitors"], 3);
}

// ---- the rule: insight routes read rollups only ---------------------------------------------

#[sqlx::test(migrations = false)]
async fn insight_routes_never_read_raw_events_or_decisions(pool: PgPool) {
    corpus(&pool, true).await;
    let app = admin_app(&pool);
    let draft: i64 = sqlx::query_scalar("SELECT min(id) FROM segments").fetch_one(&pool).await.unwrap();
    let gets = [
        "/api/sites/demo/insights/heatmap",
        "/api/sites/demo/insights/heatmap?axis=utc&by=dow_hour&slot=hero",
        "/api/sites/demo/insights/journeys",
        "/api/sites/demo/insights/journeys?outcome=bounced",
        "/api/sites/demo/insights/retention",
        "/api/sites/demo/insights/status",
        "/api/sites/demo/segments",
    ];
    let mut before = Vec::new();
    for g in gets {
        before.push(api_get(&app, g).await.json());
    }
    sqlx::query("ALTER TABLE events RENAME TO events_hidden").execute(&pool).await.unwrap();
    sqlx::query("ALTER TABLE decisions RENAME TO decisions_hidden").execute(&pool).await.unwrap();
    for (g, b) in gets.iter().zip(&before) {
        let r = api_get(&app, g).await;
        assert_eq!(r.status, StatusCode::OK, "{g} touched a raw table: {}", r.text);
        assert_eq!(&r.json(), b, "{g}");
    }
    let r = api_post(&app, "/api/sites/demo/segments", json!({ "predicate": { "mobile": true }, "actor": "ritwik" })).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    let r = api_post(&app, &format!("/api/segments/{draft}/promote"), json!({ "actor": "ritwik" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn insight_routes_need_the_admin_token(pool: PgPool) {
    setup_demo(&pool).await;
    let r = get(&admin_app(&pool), "/api/sites/demo/insights/heatmap").await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = false)]
async fn the_worker_hook_runs_once_per_period(pool: PgPool) {
    setup_demo(&pool).await;
    seed_corpus(&pool, 100, false).await;
    let t = Utc.timestamp_opt(Utc::now().timestamp(), 0).unwrap();
    insights::maybe_run(&pool, t).await.unwrap();
    assert_eq!(count(&pool, "SELECT sum(n)::int8 FROM rollup_hourly WHERE segment_id = 0").await, 100);
    let ran: Vec<String> = sqlx::query_scalar("SELECT name FROM insight_jobs ORDER BY name").fetch_all(&pool).await.unwrap();
    assert_eq!(ran, vec!["nightly", "rollup"]);
    insights::maybe_run(&pool, t + Duration::seconds(10)).await.unwrap();
    let at: DateTime<Utc> = sqlx::query_scalar("SELECT ran_at FROM insight_jobs WHERE name = 'rollup'").fetch_one(&pool).await.unwrap();
    assert_eq!(at, t, "a rollup ran again inside its period");
    insights::maybe_run(&pool, t + Duration::seconds(61)).await.unwrap();
    let at: DateTime<Utc> = sqlx::query_scalar("SELECT ran_at FROM insight_jobs WHERE name = 'rollup'").fetch_one(&pool).await.unwrap();
    assert_eq!(at, t + Duration::seconds(61));
}
