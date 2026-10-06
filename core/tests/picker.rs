//! The taste picker: the `picker` slot kind, `pick` and `skip` events, `stated` features in
//! the decision context, and the on-device scorer's server side (manifest weights, client
//! decisions, reconciliation).

mod common;
use chrono::{Duration, Utc};
use common::*;
use pratibimba::{bandit::LinTs, stated::{self, DynArm, StatedModel}, worker};
use serde_json::{Value, json};
use sqlx::PgPool;

fn picker_file() -> Value {
    json!({
        "site": "demo", "origin": "http://localhost:5173",
        "slots": [
            { "key": "taste", "selector": "#taste", "kind": "picker", "metric": "click",
              "picker": { "max_picks": 3, "options": [
                  { "key": "kitchen", "label": "Kitchen" }, { "key": "garden", "label": "Garden" }, { "key": "books", "label": "Books" } ] },
              "variants": [ { "key": "none", "baseline": true }, { "key": "picker", "html": "<p>What are you shopping for?</p>" } ] },
            { "key": "shelf", "selector": "#shelf", "metric": "click", "in_session": true,
              "variants": [ { "key": "baseline", "baseline": true },
                            { "key": "kitchen", "html": "<p>K</p>" }, { "key": "garden", "html": "<p>G</p>" } ] }
        ]
    })
}

async fn setup(pool: &PgPool) {
    pratibimba::db::migrate(pool).await.unwrap();
    pratibimba::site_file::apply(pool, &file_from(picker_file())).await.unwrap();
    set_holdout(pool, 0.0).await;
}

async fn apply_err(pool: &PgPool, f: Value) -> String {
    pratibimba::db::migrate(pool).await.unwrap();
    pratibimba::site_file::apply(pool, &file_from(f)).await.unwrap_err().to_string()
}

async fn decide_slot(app: &axum::Router, subject: &str, slot: &str, ctx: Value) -> Resp {
    post(app, "/decide", json!({ "site": "demo", "slot": slot, "subject": subject, "ctx": ctx })).await
}

async fn events(app: &axum::Router, subject: &str, evs: Value) -> Resp {
    post(app, "/e", json!({ "site": "demo", "subject": subject, "events": evs })).await
}

fn pick(keys: &[&str]) -> Value {
    json!([{ "id": uuid::Uuid::new_v4(), "name": "pick", "slot": "taste", "picks": keys }])
}

async fn variant_id(pool: &PgPool, slot: &str, key: &str) -> i64 {
    sqlx::query_scalar("SELECT v.id FROM variants v JOIN slots s ON s.id = v.slot_id WHERE s.key = $1 AND v.key = $2")
        .bind(slot).bind(key).fetch_one(pool).await.unwrap()
}

/// Plant a near-certain shelf model: `kitchen` wins only for a stated kitchen visitor,
/// `garden` wins otherwise. Covariances are tiny so Thompson draws sit on the means.
async fn plant_shelf_model(pool: &PgPool) {
    let (b, k, g) = (variant_id(pool, "shelf", "baseline").await, variant_id(pool, "shelf", "kitchen").await, variant_id(pool, "shelf", "garden").await);
    // Vector order: bias, returning, mobile, from_ad, stated:books, stated:garden, stated:kitchen.
    let arm = |mean: [f64; 7]| {
        let mut cov = vec![vec![0.0; 7]; 7];
        for (i, row) in cov.iter_mut().enumerate() { row[i] = 1e-8; }
        DynArm { mean: mean.to_vec(), cov }
    };
    let model = LinTs { stated: Some(StatedModel {
        keys: vec!["books".into(), "garden".into(), "kitchen".into()],
        arms: [
            (b, arm([-3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])),
            (k, arm([-4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 6.0])),
            (g, arm([-1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])),
        ].into(),
    }), ..LinTs::default() };
    sqlx::query("UPDATE models SET state = $1, version = version + 1 WHERE slot_id = (SELECT id FROM slots WHERE key = 'shelf')")
        .bind(serde_json::to_value(&model).unwrap()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO model_versions (slot_id, version, state, reason) SELECT slot_id, version, state, 'test' FROM models WHERE slot_id = (SELECT id FROM slots WHERE key = 'shelf')")
        .execute(pool).await.unwrap();
}

// --- the picker slot kind ---------------------------------------------------------------------

#[sqlx::test(migrations = false)]
async fn picker_vs_none_is_a_normal_two_variant_slot(pool: PgPool) {
    setup(&pool).await;
    let app = app(&pool);
    let mut seen = std::collections::BTreeSet::new();
    for i in 0..40 {
        let d = decide_slot(&app, &format!("v{i}"), "taste", json!({})).await;
        assert_eq!(d.status, StatusCode::OK, "{}", d.text);
        seen.insert(d.json()["variant"].as_str().unwrap().to_string());
    }
    assert_eq!(seen.into_iter().collect::<Vec<_>>(), ["none", "picker"]);
}

#[sqlx::test(migrations = false)]
async fn manifest_carries_picker_options(pool: PgPool) {
    setup(&pool).await;
    let m = get(&app(&pool), "/manifest/demo").await.json();
    let taste = m["slots"].as_array().unwrap().iter().find(|s| s["key"] == "taste").unwrap();
    assert_eq!(taste["kind"], "picker");
    assert_eq!(taste["picker"]["max_picks"], 3);
    assert_eq!(taste["picker"]["options"][0], json!({ "key": "kitchen", "label": "Kitchen" }));
}

#[sqlx::test(migrations = false)]
async fn picker_needs_exactly_none_and_picker(pool: PgPool) {
    let mut f = picker_file();
    f["slots"][0]["variants"].as_array_mut().unwrap().push(json!({ "key": "other", "html": "<p>x</p>" }));
    assert!(apply_err(&pool, f).await.contains("none"), "a third variant must be refused");
}

#[sqlx::test(migrations = false)]
async fn picker_max_picks_is_one_to_three(pool: PgPool) {
    let mut f = picker_file();
    f["slots"][0]["picker"]["max_picks"] = json!(4);
    assert!(apply_err(&pool, f).await.contains("max_picks"));
}

#[sqlx::test(migrations = false)]
async fn picker_options_cannot_be_demographic(pool: PgPool) {
    let mut f = picker_file();
    f["slots"][0]["picker"]["options"][2] = json!({ "key": "women", "label": "For women" });
    assert!(apply_err(&pool, f).await.contains("demographic"));
}

#[sqlx::test(migrations = false)]
async fn picker_slot_cannot_be_in_session(pool: PgPool) {
    let mut f = picker_file();
    f["slots"][0]["in_session"] = json!(true);
    assert!(apply_err(&pool, f).await.contains("in_session"));
}

// --- pick and skip ----------------------------------------------------------------------------

#[sqlx::test(migrations = false)]
async fn pick_events_are_validated(pool: PgPool) {
    setup(&pool).await;
    let app = app(&pool);
    for (why, evs) in [
        ("unknown option", pick(&["gender"])),
        ("four picks", pick(&["kitchen", "garden", "books", "kitchen"])),
        ("repeated pick", pick(&["kitchen", "kitchen"])),
        ("no picks", pick(&[])),
        ("not a picker slot", json!([{ "id": uuid::Uuid::new_v4(), "name": "pick", "slot": "shelf", "picks": ["kitchen"] }])),
        ("picks on another event", json!([{ "id": uuid::Uuid::new_v4(), "name": "click", "slot": "taste", "picks": ["kitchen"] }])),
        ("pick without picks", json!([{ "id": uuid::Uuid::new_v4(), "name": "pick", "slot": "taste" }])),
    ] {
        assert_eq!(events(&app, "v", evs).await.status, StatusCode::BAD_REQUEST, "{why}");
    }
    assert_eq!(events(&app, "v", pick(&["kitchen", "books"])).await.status, StatusCode::ACCEPTED);
}

#[sqlx::test(migrations = false)]
async fn a_pick_changes_later_decisions(pool: PgPool) {
    setup(&pool).await;
    plant_shelf_model(&pool).await;
    let app = app(&pool);
    let before = decide_slot(&app, "v", "shelf", json!({})).await.json();
    assert_eq!(before["variant"], "garden", "{before}");
    assert_eq!(before["explanation"]["features"]["stated:kitchen"], 0.0);

    assert_eq!(events(&app, "v", pick(&["kitchen"])).await.status, StatusCode::ACCEPTED);
    let after = decide_slot(&app, "v", "shelf", json!({})).await.json();
    assert_eq!(after["variant"], "kitchen", "{after}");
    assert_ne!(after["decision_id"], before["decision_id"], "a new statement must not get the sticky decision");
    assert_eq!(after["explanation"]["features"]["stated:kitchen"], 1.0);
    assert_eq!(after["explanation"]["features"]["stated:garden"], 0.0);
    assert_eq!(after["explanation"]["policy"], stated::POLICY);
}

#[sqlx::test(migrations = false)]
async fn a_skip_sets_nothing(pool: PgPool) {
    setup(&pool).await;
    plant_shelf_model(&pool).await;
    let app = app(&pool);
    let skip = json!([{ "id": uuid::Uuid::new_v4(), "name": "skip", "slot": "taste" }]);
    assert_eq!(events(&app, "v", skip).await.status, StatusCode::ACCEPTED);
    let d = decide_slot(&app, "v", "shelf", json!({})).await.json();
    assert_eq!(d["variant"], "garden");
    for k in ["kitchen", "garden", "books"] {
        assert_eq!(d["explanation"]["features"][format!("stated:{k}")], 0.0, "{d}");
    }
    assert_eq!(count(&pool, "SELECT count(*) FROM events WHERE picks IS NOT NULL").await, 0);
}

#[sqlx::test(migrations = false)]
async fn stated_belongs_to_the_subject_that_picked(pool: PgPool) {
    setup(&pool).await;
    plant_shelf_model(&pool).await;
    let app = app(&pool);
    events(&app, "a", pick(&["kitchen"])).await;
    let d = decide_slot(&app, "b", "shelf", json!({})).await.json();
    assert_eq!(d["explanation"]["features"]["stated:kitchen"], 0.0);
    assert_eq!(d["variant"], "garden");
}

#[sqlx::test(migrations = false)]
async fn the_latest_pick_replaces_the_earlier_one(pool: PgPool) {
    setup(&pool).await;
    let app = app(&pool);
    events(&app, "v", pick(&["kitchen"])).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    events(&app, "v", pick(&["garden"])).await;
    let d = decide_slot(&app, "v", "shelf", json!({})).await.json();
    assert_eq!(d["explanation"]["features"]["stated:kitchen"], 0.0, "{d}");
    assert_eq!(d["explanation"]["features"]["stated:garden"], 1.0, "{d}");
}

#[sqlx::test(migrations = false)]
async fn ctx_stated_is_used_and_must_be_an_option_key(pool: PgPool) {
    setup(&pool).await;
    plant_shelf_model(&pool).await;
    let app = app(&pool);
    let d = decide_slot(&app, "v", "shelf", json!({ "stated": ["kitchen"] })).await.json();
    assert_eq!(d["variant"], "kitchen");
    assert_eq!(decide_slot(&app, "w", "shelf", json!({ "stated": ["age_18_24"] })).await.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = false)]
async fn a_site_without_pickers_decides_as_before(pool: PgPool) {
    setup_learning(&pool).await;
    let app = app(&pool);
    let d = decide(&app, "v", true).await.json();
    assert_eq!(d["explanation"]["policy"], "lints");
    assert!(d["explanation"]["features"].as_object().unwrap().keys().all(|k| !k.starts_with("stated:")), "{d}");
    assert_eq!(decide_slot(&app, "v", "hero", json!({ "stated": ["kitchen"] })).await.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = false)]
async fn stated_decisions_replay(pool: PgPool) {
    setup(&pool).await;
    let app = app(&pool);
    events(&app, "v", pick(&["garden"])).await;
    let d = decide_slot(&app, "v", "shelf", json!({})).await.json();
    let id = d["decision_id"].as_str().unwrap().parse().unwrap();
    let detail = pratibimba::explain::detail(&pool, id, Utc::now()).await.unwrap().unwrap();
    assert_eq!(detail.replay_matches, Some(true), "{:?}", detail.scores);
}

#[sqlx::test(migrations = false)]
async fn the_worker_learns_stated_features(pool: PgPool) {
    setup(&pool).await;
    let app = app(&pool);
    events(&app, "v", pick(&["kitchen"])).await;
    let d = decide_slot(&app, "v", "shelf", json!({})).await.json();
    let id = d["decision_id"].as_str().unwrap();
    events(&app, "v", json!([{ "id": uuid::Uuid::new_v4(), "name": "exposure", "slot": "shelf", "decision_id": id },
                             { "id": uuid::Uuid::new_v4(), "name": "click", "slot": "shelf", "decision_id": id }])).await;
    worker::run_once(&pool, Utc::now() + Duration::hours(2)).await.unwrap();
    let state: Value = sqlx::query_scalar("SELECT state FROM models WHERE slot_id = (SELECT id FROM slots WHERE key = 'shelf')").fetch_one(&pool).await.unwrap();
    let model: LinTs = serde_json::from_value(state).unwrap();
    let st = model.stated.expect("the stated model learned");
    let vid = variant_id(&pool, "shelf", d["variant"].as_str().unwrap()).await;
    let k = st.keys.iter().position(|k| k == "kitchen").unwrap() + 4;
    assert!(st.arms[&vid].mean[k] > 0.0, "a stated conversion moves the kitchen weight up");
}

// --- the on-device scorer's server side -------------------------------------------------------

#[sqlx::test(migrations = false)]
async fn manifest_ships_weights_and_payloads_for_in_session_slots_only(pool: PgPool) {
    setup(&pool).await;
    plant_shelf_model(&pool).await;
    let m = get(&app(&pool), "/manifest/demo").await.json();
    let slot = |k: &str| m["slots"].as_array().unwrap().iter().find(|s| s["key"] == k).unwrap().clone();
    let shelf = slot("shelf");
    assert_eq!(shelf["in_session"], true);
    let kitchen = shelf["variants"].as_array().unwrap().iter().find(|v| v["key"] == "kitchen").unwrap();
    assert_eq!(kitchen["payload"], "<p>K</p>");
    assert_eq!(kitchen["weights"]["stated:kitchen"], 6.0);
    assert_eq!(kitchen["weights"]["bias"], -4.0);
    let taste = slot("taste");
    assert!(taste["variants"][1].get("weights").is_none() && taste["variants"][1].get("payload").is_none(), "{taste}");
}

fn rescore(decision: &str, variant: &str, features: Value, score: f64) -> Value {
    json!({ "id": uuid::Uuid::new_v4(), "name": "rescore", "slot": "shelf", "decision_id": decision,
            "variant": variant, "features": features, "score": score })
}

fn feats(kitchen: f64) -> Value {
    json!({ "bias": 1, "returning": 0, "mobile": 0, "from_ad": 0, "stated:kitchen": kitchen, "stated:garden": 0, "stated:books": 0 })
}

#[sqlx::test(migrations = false)]
async fn a_client_decision_is_recorded_and_learned_from(pool: PgPool) {
    setup(&pool).await;
    plant_shelf_model(&pool).await;
    let app = app(&pool);
    let id = uuid::Uuid::new_v4().to_string();
    let r = events(&app, "v", json!([rescore(&id, "kitchen", feats(1.0), 0.88),
        { "id": uuid::Uuid::new_v4(), "name": "exposure", "slot": "shelf", "decision_id": id },
        { "id": uuid::Uuid::new_v4(), "name": "click", "slot": "shelf", "decision_id": id }])).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    let (source, policy, div): (String, String, Option<f32>) =
        sqlx::query_as("SELECT source, policy, divergence FROM decisions WHERE id = $1::uuid").bind(&id).fetch_one(&pool).await.unwrap();
    assert_eq!((source.as_str(), policy.as_str()), ("client", "on_device"));
    assert!(div.unwrap() < 0.05, "the client agreed with the server: {div:?}");
    let n0: i64 = count(&pool, "SELECT (state->>'n')::bigint FROM models WHERE slot_id = (SELECT id FROM slots WHERE key = 'shelf')").await;
    worker::run_once(&pool, Utc::now() + Duration::hours(2)).await.unwrap();
    let n1: i64 = count(&pool, "SELECT (state->>'n')::bigint FROM models WHERE slot_id = (SELECT id FROM slots WHERE key = 'shelf')").await;
    assert_eq!(n1, n0 + 1, "the bandit learns from the on-device decision");
    let outcome: Option<i16> = sqlx::query_scalar("SELECT outcome FROM decisions WHERE id = $1::uuid").bind(&id).fetch_one(&pool).await.unwrap();
    assert_eq!(outcome, Some(1));
}

#[sqlx::test(migrations = false)]
async fn client_decisions_are_validated(pool: PgPool) {
    setup(&pool).await;
    let app = app(&pool);
    let id = uuid::Uuid::new_v4().to_string();
    let mut gender = feats(1.0);
    gender["gender"] = json!(1);
    for (why, ev) in [
        ("demographic feature", rescore(&id, "kitchen", gender, 0.5)),
        ("missing base feature", rescore(&id, "kitchen", json!({ "bias": 1 }), 0.5)),
        ("unknown variant", rescore(&id, "nope", feats(1.0), 0.5)),
        ("score out of range", rescore(&id, "kitchen", feats(1.0), 1.5)),
        ("not an in-session slot", json!({ "id": uuid::Uuid::new_v4(), "name": "rescore", "slot": "taste", "decision_id": id,
                                           "variant": "picker", "features": feats(1.0), "score": 0.5 })),
    ] {
        assert_eq!(events(&app, "v", json!([ev])).await.status, StatusCode::BAD_REQUEST, "{why}");
    }
}

#[sqlx::test(migrations = false)]
async fn a_holdout_visitor_gets_no_client_decision(pool: PgPool) {
    setup(&pool).await;
    set_holdout(&pool, 0.5).await;
    let held = (0..200).map(|i| format!("h{i}")).find(|s| pratibimba::hash::in_holdout("demo", s, 0.5)).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    assert_eq!(events(&app(&pool), &held, json!([rescore(&id, "kitchen", feats(1.0), 0.5)])).await.status, StatusCode::ACCEPTED);
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions").await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_divergent_client_decision_loses_to_the_server(pool: PgPool) {
    setup(&pool).await;
    plant_shelf_model(&pool).await;
    let app = app(&pool);
    // The server's model says kitchen converts ~0.12 for a non-kitchen visitor; the client claims 0.9.
    let id = uuid::Uuid::new_v4().to_string();
    events(&app, "v", json!([rescore(&id, "kitchen", feats(0.0), 0.9)])).await;
    let div: Option<f32> = sqlx::query_scalar("SELECT divergence FROM decisions WHERE id = $1::uuid").bind(&id).fetch_one(&pool).await.unwrap();
    assert!(div.unwrap() > 0.5, "{div:?}");
    let d = decide_slot(&app, "v", "shelf", json!({})).await.json();
    assert_ne!(d["decision_id"], id.as_str(), "the stale on-device decision must not be served as sticky");
    assert_eq!(d["variant"], "garden");

    // An agreeing client decision IS sticky: the next page keeps what the visitor saw.
    let ok_id = uuid::Uuid::new_v4().to_string();
    events(&app, "w", json!([rescore(&ok_id, "kitchen", feats(1.0), 0.88)])).await;
    let d = decide_slot(&app, "w", "shelf", json!({ "stated": ["kitchen"] })).await.json();
    assert_eq!(d["decision_id"], ok_id.as_str());
}

#[sqlx::test(migrations = false)]
async fn a_paused_site_ships_no_weights_and_records_no_client_decision(pool: PgPool) {
    setup(&pool).await;
    sqlx::query("UPDATE sites SET paused_at = now(), paused_by = 'test'").execute(&pool).await.unwrap();
    let m = get(&app(&pool), "/manifest/demo").await.json();
    let shelf = m["slots"].as_array().unwrap().iter().find(|s| s["key"] == "shelf").unwrap().clone();
    assert!(shelf.get("in_session").is_none() && shelf["variants"][1].get("weights").is_none(), "{shelf}");
    let id = uuid::Uuid::new_v4().to_string();
    assert_eq!(events(&app(&pool), "v", json!([rescore(&id, "kitchen", feats(1.0), 0.5)])).await.status, StatusCode::ACCEPTED);
    assert_eq!(count(&pool, "SELECT count(*) FROM decisions").await, 0);
}
