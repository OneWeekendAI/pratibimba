//! Declared features on the on-device scorer. The server bandit learns `feature:<key>`
//! dimensions (see `pii.rs`); the manifest ships them with each in-session variant's weights,
//! and a `rescore` event may carry them back. A PII one is dropped from a rescore that is not
//! identified, as it is from a decision; an undeclared one is refused.

mod common;
use common::*;
use pratibimba::{bandit::LinTs, stated::{DynArm, StatedModel}};
use serde_json::{Value, json};
use sqlx::PgPool;

fn site(mode: &str) -> Value {
    json!({
        "site": "demo", "origin": "http://localhost:5173", "privacy_mode": mode,
        "features": [{ "key": "corp_email", "pii": true }, { "key": "loyalty_member", "pii": false }],
        "slots": [{ "key": "shelf", "selector": "#shelf", "metric": "click", "in_session": true,
                    "variants": [ { "key": "baseline", "baseline": true }, { "key": "member", "html": "<p>M</p>" } ] }]
    })
}

async fn setup(pool: &PgPool, mode: &str) {
    pratibimba::db::migrate(pool).await.unwrap();
    pratibimba::site_file::apply(pool, &file_from(site(mode))).await.unwrap();
    set_holdout(pool, 0.0).await;
}

async fn variant_id(pool: &PgPool, key: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM variants WHERE key = $1").bind(key).fetch_one(pool).await.unwrap()
}

/// `member` wins only for a loyalty member: bias -3, feature:loyalty_member 5.
async fn plant(pool: &PgPool) {
    let (b, m) = (variant_id(pool, "baseline").await, variant_id(pool, "member").await);
    // Vector order: bias, returning, mobile, from_ad, feature:corp_email, feature:loyalty_member.
    let arm = |mean: [f64; 6]| {
        let mut cov = vec![vec![0.0; 6]; 6];
        for (i, row) in cov.iter_mut().enumerate() { row[i] = 1e-8; }
        DynArm { mean: mean.to_vec(), cov }
    };
    let model = LinTs { stated: Some(StatedModel {
        keys: vec!["feature:corp_email".into(), "feature:loyalty_member".into()],
        arms: [(b, arm([-1.0, 0.0, 0.0, 0.0, 0.0, 0.0])), (m, arm([-3.0, 0.0, 0.0, 0.0, 0.0, 5.0]))].into(),
    }), ..LinTs::default() };
    sqlx::query("UPDATE models SET state = $1, version = version + 1").bind(serde_json::to_value(&model).unwrap()).execute(pool).await.unwrap();
}

async fn weights(pool: &PgPool, variant: &str) -> Value {
    let m = get(&app(pool), "/manifest/demo").await.json();
    m["slots"][0]["variants"].as_array().unwrap().iter().find(|v| v["key"] == variant).unwrap()["weights"].clone()
}

fn rescore(id: &str, features: Value, score: f64) -> Value {
    json!({ "id": uuid::Uuid::new_v4(), "name": "rescore", "slot": "shelf", "decision_id": id,
            "variant": "member", "features": features, "score": score })
}

fn feats(extra: Value) -> Value {
    let mut f = json!({ "bias": 1, "returning": 0, "mobile": 0, "from_ad": 0 });
    f.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    f
}

async fn send(pool: &PgPool, consent: bool, ev: Value) -> Resp {
    post(&app(pool), "/e", json!({ "site": "demo", "subject": "v1", "consent": consent, "events": [ev] })).await
}

async fn stored(pool: &PgPool, id: &str) -> Value {
    sqlx::query_scalar("SELECT features FROM decisions WHERE id = $1::uuid").bind(id).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn manifest_weights_name_declared_features_before_any_learning(pool: PgPool) {
    setup(&pool, "full").await;
    let w = weights(&pool, "member").await;
    assert!(w.get("feature:loyalty_member").is_some() && w.get("feature:corp_email").is_some(), "{w}");
}

#[sqlx::test(migrations = false)]
async fn manifest_weights_carry_the_learned_feature_weight(pool: PgPool) {
    setup(&pool, "full").await;
    plant(&pool).await;
    assert_eq!(weights(&pool, "member").await["feature:loyalty_member"], 5.0);
}

#[sqlx::test(migrations = false)]
async fn rescore_with_a_declared_feature_is_recorded(pool: PgPool) {
    setup(&pool, "full").await;
    let id = uuid::Uuid::new_v4().to_string();
    let r = send(&pool, false, rescore(&id, feats(json!({ "feature:loyalty_member": 1 })), 0.5)).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    assert_eq!(stored(&pool, &id).await["feature:loyalty_member"], 1);
}

#[sqlx::test(migrations = false)]
async fn rescore_with_an_undeclared_feature_is_refused(pool: PgPool) {
    setup(&pool, "full").await;
    let id = uuid::Uuid::new_v4().to_string();
    let r = send(&pool, false, rescore(&id, feats(json!({ "feature:age_band": 1 })), 0.5)).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
}

#[sqlx::test(migrations = false)]
async fn rescore_pii_feature_is_dropped_without_consent(pool: PgPool) {
    setup(&pool, "consent_aware").await;
    let id = uuid::Uuid::new_v4().to_string();
    let r = send(&pool, false, rescore(&id, feats(json!({ "feature:corp_email": 1, "feature:loyalty_member": 0 })), 0.5)).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    let f = stored(&pool, &id).await;
    assert!(f.get("feature:corp_email").is_none() && f.get("feature:loyalty_member").is_some(), "{f}");
}

#[sqlx::test(migrations = false)]
async fn rescore_pii_feature_is_kept_with_consent(pool: PgPool) {
    setup(&pool, "consent_aware").await;
    let id = uuid::Uuid::new_v4().to_string();
    let r = send(&pool, true, rescore(&id, feats(json!({ "feature:corp_email": 1 })), 0.5)).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    assert_eq!(stored(&pool, &id).await["feature:corp_email"], 1);
}

#[sqlx::test(migrations = false)]
async fn rescore_divergence_reads_the_feature_weight(pool: PgPool) {
    setup(&pool, "full").await;
    plant(&pool).await;
    let id = uuid::Uuid::new_v4().to_string();
    // sigmoid(-3 + 5) = 0.881: the client and the server agree only if the server reads the feature.
    let r = send(&pool, false, rescore(&id, feats(json!({ "feature:loyalty_member": 1 })), 0.881)).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    let div: Option<f32> = sqlx::query_scalar("SELECT divergence FROM decisions WHERE id = $1::uuid").bind(&id).fetch_one(&pool).await.unwrap();
    assert!(div.unwrap() < 0.01, "{div:?}");
}
