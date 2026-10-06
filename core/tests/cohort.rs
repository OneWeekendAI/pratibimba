//! Scripted cohorts: synthetic visitors with a planted ground truth go through the real
//! decide -> exposure -> click -> worker loop. Each slot archetype is its own test, so a failure
//! names the behaviour that broke.

mod common;
use chrono::{Duration, Utc};
use common::*;
use pratibimba::worker;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use sqlx::PgPool;

const VISITORS: usize = 2000;
const MEASURE: usize = 400; // the last N decisions are what we judge
const SETTLE_EVERY: usize = 100;

struct Tally {
    returning: (usize, usize), // (gift, total)
    new: (usize, usize),
}

impl Tally {
    fn gift_share(&self) -> f64 {
        (self.returning.0 + self.new.0) as f64 / (self.returning.1 + self.new.1) as f64
    }
}

/// `p(variant, returning)` is the planted conversion probability. `learn = false` never runs
/// the worker, which is the control: the same cohort with the learning loop cut.
async fn run_cohort(pool: &PgPool, seed: u64, learn: bool, p: impl Fn(&str, bool) -> f64) -> Tally {
    setup_demo(pool).await;
    let app = app(pool);
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut tally = Tally { returning: (0, 0), new: (0, 0) };
    for i in 0..VISITORS {
        let subject = format!("sim{i}");
        let returning = rng.random::<f64>() < 0.5;
        let d = decide(&app, &subject, returning).await.json();
        let variant = d["variant"].as_str().unwrap().to_string();
        let id = d["decision_id"].as_str().unwrap().to_string();
        event(&app, &subject, "exposure", Some(&id)).await;
        if rng.random::<f64>() < p(&variant, returning) {
            event(&app, &subject, "click", Some(&id)).await;
        }
        if i >= VISITORS - MEASURE {
            let t = if returning { &mut tally.returning } else { &mut tally.new };
            t.0 += (variant == "gift") as usize;
            t.1 += 1;
        }
        if learn && (i + 1) % SETTLE_EVERY == 0 {
            // Close every open window: clicks already posted are credited first, the rest
            // settle as misses.
            worker::run_once(pool, Utc::now() + Duration::hours(2)).await.unwrap();
        }
    }
    eprintln!(
        "cohort seed={seed} learn={learn}: returning gift {}/{}, new gift {}/{}",
        tally.returning.0, tally.returning.1, tally.new.0, tally.new.1
    );
    tally
}

#[sqlx::test(migrations = false)]
async fn cohort_global_winner_takes_traffic(pool: PgPool) {
    let t = run_cohort(&pool, 1, true, |v, _| if v == "gift" { 0.12 } else { 0.03 }).await;
    assert!(t.gift_share() >= 0.75, "gift share in the last {MEASURE}: {:.2}", t.gift_share());
}

#[sqlx::test(migrations = false)]
async fn cohort_context_split_is_learned(pool: PgPool) {
    let t = run_cohort(&pool, 2, true, |v, returning| match (v == "gift", returning) {
        (true, true) | (false, false) => 0.15,
        _ => 0.03,
    })
    .await;
    let ret = t.returning.0 as f64 / t.returning.1 as f64;
    let new_baseline = 1.0 - t.new.0 as f64 / t.new.1 as f64;
    assert!(ret >= 0.70, "returning visitors shown gift: {ret:.2}");
    assert!(new_baseline >= 0.70, "new visitors shown baseline: {new_baseline:.2}");
}

/// The accept-side control: without the learning loop the same cohort stays near an even
/// split, so the thresholds above cannot be met by the prior or by chance.
#[sqlx::test(migrations = false)]
async fn cohort_without_learning_stays_split(pool: PgPool) {
    let t = run_cohort(&pool, 1, false, |v, _| if v == "gift" { 0.12 } else { 0.03 }).await;
    let s = t.gift_share();
    assert!((0.35..=0.65).contains(&s), "gift share without learning: {s:.2}");
}
