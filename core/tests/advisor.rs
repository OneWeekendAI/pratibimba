//! A1 (B6): every settled decision gets a propensity from the model version that made it.

mod common;
use common::*;
use pratibimba::bandit::LinTs;
use pratibimba::worker;
use serde_json::json;
use sqlx::PgPool;

async fn site(pool: &PgPool) -> (i64, i64, i64) {
    pratibimba::db::migrate(pool).await.unwrap();
    pratibimba::site_file::apply(pool, &file_from(json!({ "site": "demo", "origin": "http://localhost:5173", "slots": [
        { "key": "hero", "selector": "#hero", "variants": [{ "key": "baseline", "baseline": true }, { "key": "gift", "html": "<h1>G</h1>" }] } ] })))
        .await
        .unwrap();
    let slot: i64 = sqlx::query_scalar("SELECT id FROM slots WHERE key = 'hero'").fetch_one(pool).await.unwrap();
    (slot, variant_id(pool, "baseline").await, variant_id(pool, "gift").await)
}

/// A model where `good` converts every time and `bad` never.
fn dominant(good: i64, bad: i64) -> serde_json::Value {
    let mut m = LinTs::default();
    let x = [1.0, 0.0, 1.0, 0.0];
    for _ in 0..300 {
        m.update(good, &x, true);
        m.update(bad, &x, false);
    }
    serde_json::to_value(m).unwrap()
}

async fn version(pool: &PgPool, slot: i64, v: i64, state: serde_json::Value) {
    sqlx::query("INSERT INTO model_versions (slot_id, version, state, reason) VALUES ($1, $2, $3, 'test')")
        .bind(slot)
        .bind(v)
        .bind(&state)
        .execute(pool)
        .await
        .unwrap();
    // The live model is the newest version written.
    sqlx::query("UPDATE models SET version = $2, state = $3 WHERE slot_id = $1 AND version < $2").bind(slot).bind(v).bind(&state).execute(pool).await.unwrap();
}

struct Row {
    variant: i64,
    pool: Vec<i64>,
    version: i64,
    held: bool,
    settled: bool,
    edge: bool,
    days_ago: i64,
}

impl Row {
    fn bandit(variant: i64, pool: Vec<i64>, version: i64) -> Self {
        Row { variant, pool, version, held: false, settled: true, edge: false, days_ago: 1 }
    }
}

async fn decision(pool: &PgPool, slot: i64, r: Row) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO decisions (id, site_id, slot_id, variant_id, subject, pool, features, scores, policy, policy_version, seed,
                                in_holdout, created_at, exposed_at, outcome, settled_at, source, edge_snapshot_version)
         VALUES ($1, 'demo', $2, $3, $1::text, $4, '{\"bias\":1,\"returning\":0,\"mobile\":1,\"from_ad\":0}', '{}', 'lints', $5, 17,
                 $6, now() - make_interval(days => $7::int), now() - make_interval(days => $7::int), 0,
                 CASE WHEN $8 THEN now() - make_interval(days => $7::int) END,
                 CASE WHEN $9 THEN 'edge' ELSE 'server' END, CASE WHEN $9 THEN 1 END)",
    )
    .bind(id)
    .bind(slot)
    .bind(r.variant)
    .bind(&r.pool)
    .bind(r.version)
    .bind(r.held)
    .bind(r.days_ago as i32)
    .bind(r.settled)
    .bind(r.edge)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn propensity(pool: &PgPool, id: uuid::Uuid) -> (Option<f32>, Option<String>) {
    sqlx::query_as("SELECT propensity, propensity_kind FROM decisions WHERE id = $1").bind(id).fetch_one(pool).await.unwrap()
}

// Mutation: drop the fill_scoped loop from worker::run_scoped -> red.
#[sqlx::test(migrations = false)]
async fn settled_bandit_rows_get_replay_propensity(pool: PgPool) {
    let (slot, base, gift) = site(&pool).await;
    version(&pool, slot, 1, dominant(gift, base)).await;
    let won = decision(&pool, slot, Row::bandit(gift, vec![base, gift], 1)).await;
    let lost = decision(&pool, slot, Row::bandit(base, vec![base, gift], 1)).await;
    let open = decision(&pool, slot, Row { settled: false, days_ago: 0, ..Row::bandit(gift, vec![base, gift], 1) }).await;
    worker::run_once(&pool, chrono::Utc::now()).await.unwrap();
    let (p, k) = propensity(&pool, won).await;
    assert_eq!(k.as_deref(), Some("replay"));
    assert!(p.unwrap() > 0.95, "{p:?}");
    let (p, _) = propensity(&pool, lost).await;
    assert!(p.unwrap() < 0.05 && p.unwrap() > 0.0, "{p:?}");
    assert_eq!(propensity(&pool, open).await, (None, None), "an unsettled row waits");
}

// Mutation: kind_of checks holdout after single/replay (held row with two arms -> replay) -> red.
#[sqlx::test(migrations = false)]
async fn holdout_rows_get_holdout_kind(pool: PgPool) {
    let (slot, base, gift) = site(&pool).await;
    version(&pool, slot, 1, dominant(gift, base)).await;
    let id = decision(&pool, slot, Row { held: true, ..Row::bandit(base, vec![base, gift], 1) }).await;
    worker::fill_propensity(&pool, 500).await.unwrap();
    assert_eq!(propensity(&pool, id).await, (None, Some("holdout".into())));
}

// Mutation: load the current model (`models.state`) instead of the row's policy_version -> red.
#[sqlx::test(migrations = false)]
async fn edge_rows_filled_from_their_version(pool: PgPool) {
    let (slot, base, gift) = site(&pool).await;
    // Version 1 favoured gift; the live version 2 has turned to the baseline.
    version(&pool, slot, 1, dominant(gift, base)).await;
    version(&pool, slot, 2, dominant(base, gift)).await;
    let id = decision(&pool, slot, Row { edge: true, ..Row::bandit(gift, vec![base, gift], 1) }).await;
    worker::fill_propensity(&pool, 500).await.unwrap();
    let (p, k) = propensity(&pool, id).await;
    assert_eq!(k.as_deref(), Some("replay"));
    assert!(p.unwrap() > 0.95, "replayed from version 1, not the live model: {p:?}");
}

// Mutation: drop `AND d.propensity_kind IS NULL` from the UPDATE and the SELECT -> red.
#[sqlx::test(migrations = false)]
async fn fill_is_idempotent(pool: PgPool) {
    let (slot, base, gift) = site(&pool).await;
    version(&pool, slot, 1, dominant(gift, base)).await;
    decision(&pool, slot, Row::bandit(gift, vec![base, gift], 1)).await;
    decision(&pool, slot, Row { held: true, ..Row::bandit(base, vec![base, gift], 1) }).await;
    assert_eq!(worker::fill_propensity(&pool, 500).await.unwrap(), 2);
    assert_eq!(worker::fill_propensity(&pool, 500).await.unwrap(), 0);
}

// Mutation: fill rows whose version is gone as NULL kind (leave value None, kind unchanged) -> red.
#[sqlx::test(migrations = false)]
async fn row_without_its_model_version_is_none(pool: PgPool) {
    let (slot, base, gift) = site(&pool).await;
    let id = decision(&pool, slot, Row::bandit(gift, vec![base, gift], 9)).await;
    worker::fill_propensity(&pool, 500).await.unwrap();
    assert_eq!(propensity(&pool, id).await, (None, Some("none".into())));
}

// Mutation: store Kind::Single's value as NULL -> red.
#[sqlx::test(migrations = false)]
async fn single_arm_row_is_one(pool: PgPool) {
    let (slot, base, gift) = site(&pool).await;
    version(&pool, slot, 1, dominant(gift, base)).await;
    let id = decision(&pool, slot, Row::bandit(gift, vec![gift], 1)).await;
    worker::fill_propensity(&pool, 500).await.unwrap();
    assert_eq!(propensity(&pool, id).await, (Some(1.0), Some("single".into())));
}

// Mutation: leave the experiment weight out (Kind::Experiment -> None) -> red.
#[sqlx::test(migrations = false)]
async fn experiment_rows_get_their_weight(pool: PgPool) {
    let (slot, base, gift) = site(&pool).await;
    let e: i64 = sqlx::query_scalar(
        "INSERT INTO experiments (site_id, slot_id, name, metric, arms, starts_at, stops_at, created_by)
         VALUES ('demo', $1, 'e', 'click', $2, now() - interval '9 days', now() + interval '9 days', 'test') RETURNING id",
    )
    .bind(slot)
    .bind(json!([{ "variant_id": base, "weight": 1.0 }, { "variant_id": gift, "weight": 3.0 }]))
    .fetch_one(&pool)
    .await
    .unwrap();
    let id = decision(&pool, slot, Row::bandit(gift, vec![base, gift], 1)).await;
    sqlx::query("UPDATE decisions SET allocation = 'experiment', experiment_id = $2 WHERE id = $1").bind(id).bind(e).execute(&pool).await.unwrap();
    worker::fill_propensity(&pool, 500).await.unwrap();
    assert_eq!(propensity(&pool, id).await, (Some(0.75), Some("experiment".into())));
}

// Mutation: make the backfill subcommand run one batch instead of looping until 0 -> red.
#[sqlx::test(migrations = false)]
async fn backfill_fills_old_rows(pool: PgPool) {
    let (slot, base, gift) = site(&pool).await;
    version(&pool, slot, 1, serde_json::to_value(LinTs::default()).unwrap()).await;
    // More rows than one backfill batch (500), settled long before this column existed.
    let n = 520;
    sqlx::query(
        "INSERT INTO decisions (id, site_id, slot_id, variant_id, subject, pool, features, scores, policy, policy_version, seed,
                                created_at, outcome, settled_at)
         SELECT gen_random_uuid(), 'demo', $1, $2, 's' || g, $3, '{\"bias\":1,\"returning\":0,\"mobile\":0,\"from_ad\":0}', '{}',
                'lints', 1, g, now() - interval '300 days', 0, now() - interval '300 days'
           FROM generate_series(1, $4) g",
    )
    .bind(slot)
    .bind(gift)
    .bind(vec![base, gift])
    .bind(n)
    .execute(&pool)
    .await
    .unwrap();
    let db: String = sqlx::query_scalar("SELECT current_database()").fetch_one(&pool).await.unwrap();
    let base_url = std::env::var("DATABASE_URL").unwrap();
    let url = format!("{}/{db}", base_url.rsplit_once('/').unwrap().0);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_pratibimba"))
        .arg("backfill-propensity")
        .env("PRB_DB", &url)
        .env("DATABASE_URL", &url)
        .env("RUST_LOG", "warn")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains(&format!("filled for {n} decisions")), "{}", String::from_utf8_lossy(&out.stdout));
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions WHERE propensity_kind IS NULL").await, 0);
    let avg: f64 = sqlx::query_scalar("SELECT avg(propensity)::float8 FROM decisions WHERE propensity_kind = 'replay'").fetch_one(&pool).await.unwrap();
    assert!((avg - 0.5).abs() < 0.05, "two prior arms: {avg}");
}

// Mutation: loop fill_scoped in run_scoped until a short batch (no per-pass bound) -> red.
#[sqlx::test(migrations = false)]
async fn worker_pass_fills_one_bounded_batch(pool: PgPool) {
    let (slot, base, gift) = site(&pool).await;
    version(&pool, slot, 1, serde_json::to_value(LinTs::default()).unwrap()).await;
    for _ in 0..(worker::PROPENSITY_PER_PASS + 20) {
        decision(&pool, slot, Row::bandit(gift, vec![base, gift], 1)).await;
    }
    worker::run_once(&pool, chrono::Utc::now()).await.unwrap();
    let left = count(&pool, "SELECT count(*) FROM decisions WHERE propensity_kind IS NULL").await;
    assert_eq!(left, 20, "a pass must leave the rest of a backlog for the next pass");
}

// ---- B7: the nightly advisor run and its cards ----

use axum::http::StatusCode;
use rand::{Rng, SeedableRng};

/// `n` exposed, settled bandit rows between `days.0` and `days.1` days ago, half on each arm
/// (replay propensity 0.5), converting at `rates.0` (baseline) and `rates.1` (arm).
async fn plant(pool: &PgPool, slot: i64, base: i64, arm: i64, rates: (f64, f64), n: usize, days: (f64, f64), seed: u64) {
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
    let (mut variant, mut outcome, mut ago) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..n {
        let on_arm = rng.random_bool(0.5);
        variant.push(if on_arm { arm } else { base });
        outcome.push(i16::from(rng.random_bool(if on_arm { rates.1 } else { rates.0 })));
        ago.push(rng.random_range(days.0..days.1) * 86_400.0);
    }
    sqlx::query(
        "INSERT INTO decisions (id, site_id, slot_id, variant_id, subject, pool, features, scores, policy, policy_version, seed,
                                created_at, exposed_at, outcome, settled_at, propensity, propensity_kind)
         SELECT gen_random_uuid(), 'demo', $1, v, 's' || g, $2, '{\"bias\":1,\"returning\":0,\"mobile\":0,\"from_ad\":0}', '{}',
                'lints', 1, g, now() - make_interval(secs => a), now() - make_interval(secs => a), o::int,
                now() - make_interval(secs => a), 0.5, 'replay'
           FROM unnest($3::int8[], $4::int2[], $5::float8[]) WITH ORDINALITY AS t(v, o, a, g)",
    )
    .bind(slot)
    .bind(vec![base, arm])
    .bind(&variant)
    .bind(&outcome)
    .bind(&ago)
    .execute(pool)
    .await
    .unwrap();
}

/// A slot whose arm converts like the baseline: 8,000 training rows, 4,000 held-out rows.
async fn flat(pool: &PgPool) -> i64 {
    let (slot, base, gift) = site(pool).await;
    plant(pool, slot, base, gift, (0.10, 0.10), 8_000, (15.0, 60.0), 1).await;
    plant(pool, slot, base, gift, (0.10, 0.10), 4_000, (0.0, 13.9), 2).await;
    slot
}

/// The defaults' ±0.3 per 100 needs ~10^5 rows a side at a 10% rate; these tests widen it.
async fn margin(pool: &PgPool, m: f64) {
    sqlx::query("UPDATE sites SET advisor_settings = $1 WHERE id = 'demo'").bind(json!({ "margin": m })).execute(pool).await.unwrap();
}

async fn cards(pool: &PgPool) -> Vec<(i64, String, String, serde_json::Value)> {
    sqlx::query_as("SELECT slot_id, state, status, evidence FROM advisor_cards ORDER BY id").fetch_all(pool).await.unwrap()
}

// Mutation: drop the `advisor::run::run` call from insights::nightly -> red.
#[sqlx::test(migrations = false)]
async fn nightly_writes_pause_card_for_flat_slot(pool: PgPool) {
    let slot = flat(&pool).await;
    margin(&pool, 3.0).await;
    let s = pratibimba::insights::nightly(&pool, chrono::Utc::now()).await.unwrap();
    assert_eq!(s.cards, 1);
    let c = cards(&pool).await;
    assert_eq!(c.len(), 1);
    let (sid, state, status, ev) = &c[0];
    assert_eq!((*sid, state.as_str(), status.as_str()), (slot, "confident", "pending"));
    let claim = ev["claim"].as_str().unwrap();
    assert!(claim.starts_with("hero: its one version is within ±3 conversions per 100 of the page's own over 8,000 visits."), "{claim}");
    assert_eq!(ev["rows"], json!(8000));
    assert_eq!(ev["arms"][0]["key"], json!("gift"));
    assert!(ev["arms"][0]["uplift"].as_f64().unwrap().abs() < 3.0, "{ev}");
    assert_eq!(ev["visits_per_week"], json!(2000), "4,000 held-out rows over 14 days");
}

// Mutation: propose regardless of the interval (judge's `confident` = true) -> red.
#[sqlx::test(migrations = false)]
async fn nightly_writes_nothing_for_helping_arm(pool: PgPool) {
    let (slot, base, gift) = site(&pool).await;
    plant(&pool, slot, base, gift, (0.10, 0.20), 8_000, (15.0, 60.0), 3).await;
    plant(&pool, slot, base, gift, (0.10, 0.20), 4_000, (0.0, 13.9), 4).await;
    margin(&pool, 3.0).await;
    assert_eq!(pratibimba::insights::nightly(&pool, chrono::Utc::now()).await.unwrap().cards, 0);
    assert!(cards(&pool).await.is_empty());
    let a = pratibimba::advisor::run::assess(&pool, "demo", chrono::Utc::now()).await.unwrap();
    assert_eq!(a[0].candidate.state, pratibimba::advisor::pause::State::Early);
}

// Mutation: run_site reads Settings::default() instead of the site's advisor_settings -> red.
#[sqlx::test(migrations = false)]
async fn stored_margin_setting_reaches_the_run(pool: PgPool) {
    flat(&pool).await;
    assert_eq!(pratibimba::advisor::run::run(&pool, chrono::Utc::now()).await.unwrap(), 0, "±0.3 per 100 is not shown by 8,000 rows");
    let r = api_post(&admin_app(&pool), "/api/sites/demo/advisor-settings", json!({ "margin": 3.0 })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(pratibimba::advisor::run::run(&pool, chrono::Utc::now()).await.unwrap(), 1);
}

// Mutation: the run pauses the slot's arms itself (UPDATE variants SET status = 'paused') -> red.
#[sqlx::test(migrations = false)]
async fn nightly_never_changes_variant_status(pool: PgPool) {
    flat(&pool).await;
    margin(&pool, 3.0).await;
    let q = "SELECT id, status, paused_at, traffic_share::float8 FROM variants ORDER BY id";
    let before: Vec<(i64, String, Option<chrono::DateTime<chrono::Utc>>, Option<f64>)> = sqlx::query_as(q).fetch_all(&pool).await.unwrap();
    assert_eq!(pratibimba::insights::nightly(&pool, chrono::Utc::now()).await.unwrap().cards, 1);
    let after: Vec<(i64, String, Option<chrono::DateTime<chrono::Utc>>, Option<f64>)> = sqlx::query_as(q).fetch_all(&pool).await.unwrap();
    assert_eq!(before, after);
}

// Mutation: drop the open-card slots from `blocked` -> red (the second insert hits the index).
#[sqlx::test(migrations = false)]
async fn second_night_does_not_duplicate_card(pool: PgPool) {
    flat(&pool).await;
    margin(&pool, 3.0).await;
    let now = chrono::Utc::now();
    assert_eq!(pratibimba::advisor::run::run(&pool, now).await.unwrap(), 1);
    assert_eq!(pratibimba::advisor::run::run(&pool, now + chrono::Duration::days(1)).await.unwrap(), 0);
    assert_eq!(cards(&pool).await.len(), 1);
}

// Mutation: delete the unique index advisor_cards_one_open -> red.
#[sqlx::test(migrations = false)]
async fn one_open_card_per_slot(pool: PgPool) {
    let (slot, _, _) = site(&pool).await;
    let ins = "INSERT INTO advisor_cards (site_id, slot_id, kind, state, evidence) VALUES ('demo', $1, 'pause_slot', 'confident', '{}')";
    sqlx::query(ins).bind(slot).execute(&pool).await.unwrap();
    assert!(sqlx::query(ins).bind(slot).execute(&pool).await.is_err(), "a second open card for the slot");
    sqlx::query("UPDATE advisor_cards SET status = 'rejected', decided_by = 'asha', decided_at = now()").execute(&pool).await.unwrap();
    sqlx::query(ins).bind(slot).execute(&pool).await.unwrap();
}

// Mutation: drop the rejected cards from `blocked` -> red.
#[sqlx::test(migrations = false)]
async fn rejected_card_keeps_slot_quiet(pool: PgPool) {
    flat(&pool).await;
    margin(&pool, 3.0).await;
    let now = chrono::Utc::now();
    assert_eq!(pratibimba::advisor::run::run(&pool, now).await.unwrap(), 1);
    sqlx::query("UPDATE advisor_cards SET status = 'rejected', decided_by = 'asha', decided_at = $1").bind(now).execute(&pool).await.unwrap();
    assert_eq!(pratibimba::advisor::run::run(&pool, now + chrono::Duration::days(1)).await.unwrap(), 0);
}

// Mutation: count holdout-kind rows (drop the propensity_kind filter, NULL propensity read as 1) -> red.
#[sqlx::test(migrations = false)]
async fn holdout_rows_are_not_compared(pool: PgPool) {
    let slot = flat(&pool).await;
    margin(&pool, 3.0).await;
    // Held-out-by-hash baseline rows that all converted: not a policy draw, so not evidence.
    sqlx::query(
        "INSERT INTO decisions (id, site_id, slot_id, variant_id, subject, pool, features, scores, policy, policy_version, seed,
                                in_holdout, created_at, exposed_at, outcome, settled_at, propensity_kind)
         SELECT gen_random_uuid(), 'demo', $1, (SELECT id FROM variants WHERE key = 'baseline'), 'h' || g, (SELECT pool FROM decisions LIMIT 1),
                '{}', '{}', 'lints', 1, g, true, now() - interval '20 days', now() - interval '20 days', 1, now() - interval '20 days', 'holdout'
           FROM generate_series(1, 3000) g",
    )
    .bind(slot)
    .execute(&pool)
    .await
    .unwrap();
    let a = pratibimba::advisor::run::assess(&pool, "demo", chrono::Utc::now()).await.unwrap();
    assert_eq!(a[0].evidence["rows"], json!(8000), "{}", a[0].evidence);
}

// Mutation: write_advisor_settings skips validate -> red.
#[sqlx::test(migrations = false)]
async fn advisor_settings_api_defaults_and_bounds(pool: PgPool) {
    site(&pool).await;
    let app = admin_app(&pool);
    let j = api_get(&app, "/api/sites/demo/advisor-settings").await.json();
    assert_eq!((&j["min_rows"], &j["margin"], &j["held_out_days"]), (&json!(300), &json!(0.3), &json!(14)), "{j}");
    assert_eq!((&j["fdr_q"], &j["cards_per_week"], &j["quiet_days"]), (&json!(0.1), &json!(3), &json!(30)), "{j}");
    let r = api_post(&app, "/api/sites/demo/advisor-settings", json!({ "min_rows": 10 })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    assert!(r.text.contains("min_rows is 50 to 1000000"), "{}", r.text);
    let r = api_post(&app, "/api/sites/demo/advisor-settings", json!({ "quiet_days": 7, "cards_per_week": 1 })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let j = api_get(&app, "/api/sites/demo/advisor-settings").await.json();
    assert_eq!((&j["quiet_days"], &j["cards_per_week"], &j["margin"]), (&json!(7), &json!(1), &json!(0.3)), "{j}");
}

// Mutation: apply skips the advisor_settings UPDATE -> red.
#[sqlx::test(migrations = false)]
async fn site_file_sets_advisor_settings(pool: PgPool) {
    pratibimba::db::migrate(&pool).await.unwrap();
    pratibimba::site_file::apply(&pool, &file_from(json!({ "site": "demo", "origin": "http://localhost:5173", "advisor_settings": { "margin": 1.5, "min_rows": 500 },
        "slots": [{ "key": "hero", "selector": "#hero", "variants": [{ "key": "baseline", "baseline": true }] }] })))
        .await
        .unwrap();
    let s = pratibimba::advisor::run::settings(&pool, "demo").await.unwrap();
    assert_eq!((s.margin(), s.min_rows(), s.held_out_days()), (1.5, 500, 14));
}

// ---- B8: backfill reaches sandboxes too ----

// Mutation: backfill_propensity fills real sites only (drop the sandbox loop) -> red.
#[sqlx::test(migrations = false)]
async fn backfill_fills_sandbox_rows(pool: PgPool) {
    site(&pool).await;
    let sb = pratibimba::sim::reset(&pool, "demo", chrono::Utc::now()).await.unwrap();
    let (slot, base, gift): (i64, i64, i64) = sqlx::query_as(
        "SELECT s.id, b.id, g.id FROM slots s JOIN variants b ON b.slot_id = s.id AND b.key = 'baseline' JOIN variants g ON g.slot_id = s.id AND g.key = 'gift'
          WHERE s.site_id = $1",
    )
    .bind(&sb)
    .fetch_one(&pool)
    .await
    .unwrap();
    let version: i64 = sqlx::query_scalar("SELECT version FROM models WHERE slot_id = $1").bind(slot).fetch_one(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO decisions (id, site_id, slot_id, variant_id, subject, pool, features, scores, policy, policy_version, seed,
                                created_at, outcome, settled_at)
         SELECT gen_random_uuid(), $1, $2, $3, 's' || g, $4, '{\"bias\":1,\"returning\":0,\"mobile\":0,\"from_ad\":0}', '{}',
                'lints', $5, g, now() - interval '3 days', 0, now() - interval '3 days'
           FROM generate_series(1, 30) g",
    )
    .bind(&sb)
    .bind(slot)
    .bind(gift)
    .bind(vec![base, gift])
    .bind(version)
    .execute(&pool)
    .await
    .unwrap();
    let db: String = sqlx::query_scalar("SELECT current_database()").fetch_one(&pool).await.unwrap();
    let base_url = std::env::var("DATABASE_URL").unwrap();
    let url = format!("{}/{db}", base_url.rsplit_once('/').unwrap().0);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_pratibimba"))
        .arg("backfill-propensity")
        .env("PRB_DB", &url)
        .env("DATABASE_URL", &url)
        .env("RUST_LOG", "warn")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("filled for 30 decisions"), "{}", String::from_utf8_lossy(&out.stdout));
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions WHERE propensity_kind = 'replay'").await, 30);
}
