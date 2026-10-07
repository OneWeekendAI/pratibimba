//! Time features on the on-device scorer. Once a slot's model has learned the local-clock
//! dimensions (`time:sin_hour` .. `time:is_business_hours_local`), the manifest ships them in
//! each in-session variant's weights under their decision names (`sin_hour`, ...). The SDK fills
//! them from the browser clock and the site's night window (shipped as `night`), and a `rescore`
//! may carry them back: the hour's sine and cosine in -1..1, the three flags in 0-1.

mod common;
use common::*;
use pratibimba::{bandit::LinTs, stated::{DynArm, StatedModel}};
use serde_json::{Value, json};
use sqlx::PgPool;

fn site() -> Value {
    json!({
        "site": "demo", "origin": "http://localhost:5173", "privacy_mode": "full",
        "slots": [{ "key": "shelf", "selector": "#shelf", "metric": "click", "in_session": true,
                    "variants": [ { "key": "baseline", "baseline": true }, { "key": "night", "html": "<p>N</p>" } ] }]
    })
}

async fn setup(pool: &PgPool) {
    pratibimba::db::migrate(pool).await.unwrap();
    pratibimba::site_file::apply(pool, &file_from(site())).await.unwrap();
    set_holdout(pool, 0.0).await;
}

async fn variant_id(pool: &PgPool, key: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM variants WHERE key = $1").bind(key).fetch_one(pool).await.unwrap()
}

/// A model that has learned the time features: `night` wins only at night (bias -3, night 5).
async fn plant(pool: &PgPool) {
    let (b, n) = (variant_id(pool, "baseline").await, variant_id(pool, "night").await);
    // Vector order: the base four, then the time keys in the sorted order a model appends them.
    let arm = |mean: [f64; 9]| {
        let mut cov = vec![vec![0.0; 9]; 9];
        for (i, row) in cov.iter_mut().enumerate() { row[i] = 1e-8; }
        DynArm { mean: mean.to_vec(), cov }
    };
    let keys = ["time:cos_hour", "time:is_business_hours_local", "time:is_night_local", "time:is_weekend_local", "time:sin_hour"];
    let model = LinTs { stated: Some(StatedModel {
        keys: keys.iter().map(|k| k.to_string()).collect(),
        arms: [(b, arm([-1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])), (n, arm([-3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 5.0, 0.0, 0.0]))].into(),
    }), ..LinTs::default() };
    sqlx::query("UPDATE models SET state = $1, version = version + 1").bind(serde_json::to_value(&model).unwrap()).execute(pool).await.unwrap();
}

async fn manifest(pool: &PgPool) -> Value {
    get(&app(pool), "/manifest/demo").await.json()
}

fn rescore(id: &str, features: Value, score: f64) -> Value {
    json!({ "id": uuid::Uuid::new_v4(), "name": "rescore", "slot": "shelf", "decision_id": id,
            "variant": "night", "features": features, "score": score })
}

/// The base four and all five time features, at 23:00 on a Wednesday (night, not business hours).
fn feats(over: Value) -> Value {
    let mut f = json!({ "bias": 1, "returning": 0, "mobile": 0, "from_ad": 0,
                        "sin_hour": -0.259, "cos_hour": 0.966, "is_night_local": 1, "is_weekend_local": 0, "is_business_hours_local": 0 });
    f.as_object_mut().unwrap().extend(over.as_object().unwrap().clone());
    f
}

async fn send(pool: &PgPool, ev: Value) -> Resp {
    post(&app(pool), "/e", json!({ "site": "demo", "subject": "v1", "consent": true, "events": [ev] })).await
}

#[sqlx::test(migrations = false)]
async fn manifest_weights_name_learned_time_features(pool: PgPool) {
    setup(&pool).await;
    plant(&pool).await;
    let w = &manifest(&pool).await["slots"][0]["variants"][1]["weights"];
    assert_eq!(w["is_night_local"], 5.0, "{w}");
}

#[sqlx::test(migrations = false)]
async fn manifest_ships_the_site_night_window(pool: PgPool) {
    setup(&pool).await;
    sqlx::query("UPDATE sites SET night_start_hour = 21, night_end_hour = 6").execute(&pool).await.unwrap();
    assert_eq!(manifest(&pool).await["night"], json!({ "start": 21, "end": 6 }));
}

#[sqlx::test(migrations = false)]
async fn rescore_with_learned_time_features_is_recorded(pool: PgPool) {
    setup(&pool).await;
    plant(&pool).await;
    let id = uuid::Uuid::new_v4().to_string();
    let r = send(&pool, rescore(&id, feats(json!({})), 0.881)).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    let f: Value = sqlx::query_scalar("SELECT features FROM decisions WHERE id = $1::uuid").bind(&id).fetch_one(&pool).await.unwrap();
    assert_eq!(f["sin_hour"], -0.259, "{f}");
}

#[sqlx::test(migrations = false)]
async fn rescore_divergence_reads_the_time_weight(pool: PgPool) {
    setup(&pool).await;
    plant(&pool).await;
    let id = uuid::Uuid::new_v4().to_string();
    // sigmoid(-3 + 5) = 0.881: the client and the server agree only if the server reads the night flag.
    let r = send(&pool, rescore(&id, feats(json!({})), 0.881)).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    let div: Option<f32> = sqlx::query_scalar("SELECT divergence FROM decisions WHERE id = $1::uuid").bind(&id).fetch_one(&pool).await.unwrap();
    assert!(div.unwrap() < 0.01, "{div:?}");
}

#[sqlx::test(migrations = false)]
async fn rescore_hour_sine_outside_minus_one_to_one_is_refused(pool: PgPool) {
    setup(&pool).await;
    let id = uuid::Uuid::new_v4().to_string();
    let r = send(&pool, rescore(&id, feats(json!({ "sin_hour": -1.5 })), 0.5)).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn rescore_negative_time_flag_is_refused(pool: PgPool) {
    setup(&pool).await;
    let id = uuid::Uuid::new_v4().to_string();
    let r = send(&pool, rescore(&id, feats(json!({ "is_night_local": -0.5 })), 0.5)).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn rescore_negative_base_feature_is_still_refused(pool: PgPool) {
    setup(&pool).await;
    let id = uuid::Uuid::new_v4().to_string();
    let r = send(&pool, rescore(&id, feats(json!({ "mobile": -0.5 })), 0.5)).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn rescore_negative_hour_cosine_is_accepted(pool: PgPool) {
    setup(&pool).await;
    plant(&pool).await;
    let id = uuid::Uuid::new_v4().to_string();
    // 14:00: the hour's cosine is -0.866.
    let r = send(&pool, rescore(&id, feats(json!({ "sin_hour": -0.5, "cos_hour": -0.866, "is_night_local": 0 })), 0.047)).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
}
