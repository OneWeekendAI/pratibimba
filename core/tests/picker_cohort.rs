//! A scripted cohort with a planted truth: each visitor has a hidden taste (kitchen or garden)
//! and converts on the `shelf` slot far more often when shown their own taste. Visitors who
//! state their taste through the picker should teach a policy that shows each of them their
//! own taste; the same cohort with every visitor skipping can do no better than a coin.

mod common;
use chrono::{Duration, Utc};
use common::*;
use pratibimba::worker;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde_json::json;
use sqlx::PgPool;

const VISITORS: usize = 1500;
const MEASURE: usize = 400;
const SETTLE_EVERY: usize = 100;

/// Planted conversion probability for the shelf.
fn truth(variant: &str, taste: &str) -> f64 {
    match variant {
        "baseline" => 0.08,
        v if v == taste => 0.30,
        _ => 0.03,
    }
}

struct Outcome {
    matched: f64,
    converted: f64,
}

async fn run(pool: &PgPool, seed: u64, state_taste: bool) -> Outcome {
    pratibimba::db::migrate(pool).await.unwrap();
    pratibimba::site_file::apply(pool, &file_from(json!({
        "site": "demo", "origin": "http://localhost:5173",
        "slots": [
            { "key": "taste", "selector": "#taste", "kind": "picker", "metric": "click",
              "picker": { "max_picks": 1, "options": [ { "key": "kitchen", "label": "Kitchen" }, { "key": "garden", "label": "Garden" } ] },
              "variants": [ { "key": "none", "baseline": true }, { "key": "picker" } ] },
            { "key": "shelf", "selector": "#shelf", "metric": "click", "in_session": true,
              "variants": [ { "key": "baseline", "baseline": true }, { "key": "kitchen", "html": "<p>K</p>" }, { "key": "garden", "html": "<p>G</p>" } ] }
        ]
    })))
    .await
    .unwrap();
    set_holdout(pool, 0.0).await;
    let app = app(pool);
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let (mut matched, mut converted) = (0usize, 0usize);
    for i in 0..VISITORS {
        let subject = format!("sim{i}");
        let taste = if rng.random::<f64>() < 0.5 { "kitchen" } else { "garden" };
        let ev = |name: &str| json!({ "id": uuid::Uuid::new_v4(), "name": name, "slot": "taste" });
        let first = if state_taste {
            let mut p = ev("pick");
            p["picks"] = json!([taste]);
            p
        } else {
            ev("skip")
        };
        assert_eq!(post(&app, "/e", json!({ "site": "demo", "subject": subject, "events": [first] })).await.status, StatusCode::ACCEPTED);
        let d = post(&app, "/decide", json!({ "site": "demo", "slot": "shelf", "subject": subject })).await.json();
        let variant = d["variant"].as_str().unwrap().to_string();
        let id = d["decision_id"].as_str().unwrap();
        let mut evs = vec![json!({ "id": uuid::Uuid::new_v4(), "name": "exposure", "slot": "shelf", "decision_id": id })];
        let click = rng.random::<f64>() < truth(&variant, taste);
        if click {
            evs.push(json!({ "id": uuid::Uuid::new_v4(), "name": "click", "slot": "shelf", "decision_id": id }));
        }
        post(&app, "/e", json!({ "site": "demo", "subject": subject, "events": evs })).await;
        if i >= VISITORS - MEASURE {
            matched += (variant == taste) as usize;
            converted += click as usize;
        }
        if (i + 1) % SETTLE_EVERY == 0 {
            worker::run_once(pool, Utc::now() + Duration::hours(2)).await.unwrap();
        }
    }
    let o = Outcome { matched: matched as f64 / MEASURE as f64, converted: converted as f64 / MEASURE as f64 };
    eprintln!("stated={state_taste} seed={seed}: shown own taste {:.2}, converted {:.3}", o.matched, o.converted);
    o
}

#[sqlx::test(migrations = false)]
async fn stated_taste_is_learned(pool: PgPool) {
    let o = run(&pool, 11, true).await;
    assert!(o.matched >= 0.80, "visitors shown their own taste: {:.2}", o.matched);
}

/// The control: the same cohort, everyone skips. Nothing tells the policy a visitor's taste,
/// so it cannot beat a coin, which is what makes the threshold above mean something.
#[sqlx::test(migrations = false)]
async fn skipping_cohort_cannot_match_taste(pool: PgPool) {
    let o = run(&pool, 11, false).await;
    assert!((0.30..=0.65).contains(&o.matched), "skippers shown their own taste: {:.2}", o.matched);
}

/// Stated preference pays off on the site's own metric, not only on the match rate.
#[sqlx::test(migrations = false)]
async fn stated_taste_raises_conversion(pool: PgPool) {
    let with = run(&pool, 12, true).await;
    let db2 = FreshDb::new().await;
    let without = run(&db2.pool, 12, false).await;
    db2.drop().await;
    assert!(with.converted > without.converted + 0.05, "with {:.3} vs without {:.3}", with.converted, without.converted);
}
