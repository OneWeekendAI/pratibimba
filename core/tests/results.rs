//! Lift against the holdout over closed windows.

mod common;
use chrono::Utc;
use common::*;
use pratibimba::results;
use sqlx::PgPool;

async fn hero(pool: &PgPool) -> results::SlotResult {
    let r = results::site(pool, "demo", results::ALL_TIME, Utc::now()).await.unwrap().unwrap();
    r.slots.into_iter().find(|s| s.key == "hero").unwrap()
}

#[sqlx::test(migrations = false)]
async fn results_count_only_closed_windows(pool: PgPool) {
    setup_demo(&pool).await;
    seed_decisions(&pool, "gift", false, 100, 10, 60).await; // closed
    seed_decisions(&pool, "gift", false, 50, 50, 5).await; // open: all converted already
    let s = hero(&pool).await;
    assert_eq!((s.treated.n, s.treated.conversions), (100, 10), "open windows were counted");
}

#[sqlx::test(migrations = false)]
async fn results_lift_is_treated_minus_holdout(pool: PgPool) {
    setup_demo(&pool).await;
    seed_decisions(&pool, "gift", false, 4000, 400, 60).await; // 10%
    seed_decisions(&pool, "baseline", true, 4000, 200, 60).await; // 5%
    let lift = hero(&pool).await.lift.unwrap();
    assert!((lift.estimate - 0.05).abs() < 1e-9, "{lift:?}");
    assert!(lift.lo > 0.0, "a 5 pp lift over 4,000 each was not detected: {lift:?}");
}

#[sqlx::test(migrations = false)]
async fn results_exclude_unexposed_decisions(pool: PgPool) {
    setup_demo(&pool).await;
    seed_decisions(&pool, "gift", false, 100, 10, 60).await;
    sqlx::query("UPDATE decisions SET exposed_at = NULL WHERE subject LIKE 'seed-gift-false-%' AND split_part(subject, '-', 4)::int > 80")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(hero(&pool).await.treated.n, 80, "unexposed decisions were counted");
}

#[sqlx::test(migrations = false)]
async fn results_split_per_variant(pool: PgPool) {
    setup_demo(&pool).await;
    seed_decisions(&pool, "gift", false, 300, 30, 60).await;
    seed_decisions(&pool, "baseline", false, 100, 3, 60).await;
    let s = hero(&pool).await;
    let gift = s.variants.iter().find(|v| v.key == "gift").unwrap();
    assert_eq!((gift.counts.n, gift.counts.conversions), (300, 30));
    assert!((gift.share - 0.75).abs() < 1e-9, "{}", gift.share);
}

#[sqlx::test(migrations = false)]
async fn curve_is_cumulative(pool: PgPool) {
    setup_demo(&pool).await;
    seed_decisions(&pool, "gift", false, 100, 10, 300).await;
    seed_decisions(&pool, "baseline", true, 100, 5, 300).await;
    seed_decisions(&pool, "gift", false, 100, 30, 120).await;
    seed_decisions(&pool, "baseline", true, 100, 5, 120).await;
    let slot: i64 = sqlx::query_scalar("SELECT id FROM slots WHERE key = 'hero'").fetch_one(&pool).await.unwrap();
    let c = results::curve(&pool, slot, Utc::now()).await.unwrap();
    assert!(c.len() >= 2, "{c:?}");
    let last = c.last().unwrap().policy.unwrap();
    // Cumulative: (10+30)/200 - (5+5)/200 = 0.15, not the last bucket's 0.25.
    assert!((last.estimate - 0.15).abs() < 1e-9, "{last:?}");
}

/// Results carry whether the slot's model has learned anything, so an agent reading them does
/// not take the prior for a finding.
#[sqlx::test(migrations = false)]
async fn results_say_how_many_lessons_the_model_has(pool: PgPool) {
    setup_demo(&pool).await;
    let s = serde_json::to_value(hero(&pool).await).unwrap();
    assert_eq!(s["lessons"], 0, "{s}");
    assert_eq!(s["learned"], false, "{s}");
    let mut m = pratibimba::bandit::LinTs::default();
    m.update(variant_id(&pool, "gift").await, &[1.0, 0.0, 0.0, 0.0], true);
    sqlx::query("UPDATE models SET state = $1").bind(serde_json::to_value(&m).unwrap()).execute(&pool).await.unwrap();
    let s = serde_json::to_value(hero(&pool).await).unwrap();
    assert_eq!(s["lessons"], 1, "{s}");
    assert_eq!(s["learned"], true, "{s}");
}
