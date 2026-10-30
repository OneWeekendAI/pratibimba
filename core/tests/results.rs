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

/// Decisions whose window is still open are not counted, but they are reported as pending, so
/// the first half hour after real traffic does not read as no traffic at all.
#[sqlx::test(migrations = false)]
async fn results_report_decisions_still_in_their_window(pool: PgPool) {
    setup_demo(&pool).await;
    seed_decisions(&pool, "gift", false, 100, 10, 60).await; // closed
    seed_decisions(&pool, "gift", false, 50, 50, 5).await; // open, all converted already
    seed_decisions(&pool, "baseline", true, 20, 2, 5).await; // open, holdout
    seed_decisions(&pool, "baseline", false, 10, 0, 5).await; // open, never shown yet
    sqlx::query("UPDATE decisions SET exposed_at = NULL WHERE subject LIKE 'seed-baseline-false-%'").execute(&pool).await.unwrap();
    let s = serde_json::to_value(hero(&pool).await).unwrap();
    assert_eq!(s["treated"]["n"], 100, "pending decisions leaked into the counted ones: {s}");
    assert_eq!(s["pending"], serde_json::json!({ "decisions": 80, "exposures": 70, "conversions": 52 }), "{s}");
    let v = |k: &str| s["variants"].as_array().unwrap().iter().find(|v| v["key"] == k).unwrap()["pending"].clone();
    assert_eq!(v("gift"), serde_json::json!({ "decisions": 50, "exposures": 50, "conversions": 50 }), "{s}");
    // Per variant is treated only, like the counts beside it.
    assert_eq!(v("baseline"), serde_json::json!({ "decisions": 10, "exposures": 0, "conversions": 0 }), "{s}");
    let site = serde_json::to_value(results::site(&pool, "demo", results::ALL_TIME, Utc::now()).await.unwrap().unwrap()).unwrap();
    assert_eq!(site["pending"]["decisions"], 80, "{site}");
}

/// A closed window is never pending, and pending honours the period like the counts do.
#[sqlx::test(migrations = false)]
async fn pending_excludes_closed_windows_and_older_decisions(pool: PgPool) {
    setup_demo(&pool).await;
    seed_decisions(&pool, "gift", false, 100, 10, 60).await;
    let s = serde_json::to_value(hero(&pool).await).unwrap();
    assert_eq!(s["pending"]["decisions"], 0, "{s}");
    seed_decisions(&pool, "baseline", false, 7, 1, 20).await;
    let now = Utc::now();
    let r = results::site(&pool, "demo", now - chrono::Duration::minutes(10), now).await.unwrap().unwrap();
    let s = serde_json::to_value(r.slots.into_iter().find(|s| s.key == "hero").unwrap()).unwrap();
    assert_eq!(s["pending"]["decisions"], 0, "a decision before the period was counted as pending: {s}");
    let s = serde_json::to_value(hero(&pool).await).unwrap();
    assert_eq!(s["pending"]["decisions"], 7, "{s}");
}

// ---- B8: the advisor's line on each slot ----

async fn hero_with_advisor(pool: &PgPool) -> results::SlotResult {
    let now = Utc::now();
    let mut r = results::site(pool, "demo", results::ALL_TIME, now).await.unwrap().unwrap();
    results::advisor(pool, &mut r, now).await.unwrap();
    r.slots.into_iter().find(|s| s.key == "hero").unwrap()
}

/// An approved pause card on hero, decided `days` ago; gift is paused as approving does.
async fn approved_card(pool: &PgPool, days: i32) -> chrono::DateTime<Utc> {
    sqlx::query("UPDATE variants SET status = 'paused', paused_at = now() WHERE key = 'gift'").execute(pool).await.unwrap();
    sqlx::query_scalar(
        "INSERT INTO advisor_cards (site_id, slot_id, kind, state, evidence, status, decided_by, decided_at, created_at)
         SELECT 'demo', id, 'pause_slot', 'confident', '{}', 'approved', 'Asha', now() - make_interval(days => $1), now() - make_interval(days => $1 + 1)
           FROM slots WHERE key = 'hero' RETURNING decided_at",
    )
    .bind(days)
    .fetch_one(pool)
    .await
    .unwrap()
}

// Mutation: the line is None for a slot under min_rows (drop the not_enough_data arm) -> red.
#[sqlx::test(migrations = false)]
async fn results_shows_not_enough_data(pool: PgPool) {
    setup_demo(&pool).await;
    seed_decisions(&pool, "gift", false, 40, 4, 60).await;
    let a = hero_with_advisor(&pool).await.advisor.expect("a line for an assessed slot");
    assert_eq!(a.state, "not_enough_data");
    assert!(a.since.is_none() && a.orders_delta.is_none(), "{a:?}");
}

// Mutation: ignore approved cards (look up pending cards only) -> red.
#[sqlx::test(migrations = false)]
async fn results_shows_approved_card_delta(pool: PgPool) {
    setup_demo(&pool).await;
    let decided = approved_card(&pool, 3).await;
    // Since the pause: the page's own for everyone, 10% treated and 10% held out.
    seed_decisions(&pool, "baseline", false, 3000, 300, 60).await;
    seed_decisions(&pool, "baseline", true, 3000, 300, 60).await;
    let a = hero_with_advisor(&pool).await.advisor.expect("a line for the approved card");
    assert_eq!(a.state, "approved");
    assert_eq!(a.since.map(|t| t.timestamp()), Some(decided.timestamp()));
    let d = a.orders_delta.expect("a delta once data has come in since the pause");
    assert!(d.estimate.abs() < 1e-9 && d.lo < 0.0 && d.hi > 0.0, "{d:?}");
}

// Mutation: compute the delta from treated rows only (after minus before) -> red.
#[sqlx::test(migrations = false)]
async fn delta_uses_holdout_not_treated_only(pool: PgPool) {
    setup_demo(&pool).await;
    approved_card(&pool, 3).await;
    // Before the pause (4 days ago): treated 5%, holdout 5%. Since: treated 10%, holdout 10%,
    // so the slot against its holdout moved by nothing, though treated alone rose 5 points.
    seed_decisions(&pool, "gift", false, 3000, 150, 4 * 24 * 60).await;
    seed_decisions(&pool, "baseline", true, 3000, 150, 4 * 24 * 60).await;
    seed_decisions(&pool, "baseline", false, 3000, 300, 60).await;
    seed_decisions(&pool, "baseline", true, 3000, 300, 60).await;
    let a = hero_with_advisor(&pool).await.advisor.unwrap();
    let d = a.orders_delta.unwrap();
    assert!(d.estimate.abs() < 1e-9, "since the pause the slot matches its holdout: {d:?}");
    let b = a.before.expect("the window before the pause");
    assert!(b.estimate.abs() < 1e-9, "{b:?}");
}

// Mutation: a pending card reads as not_enough_data (drop the card_pending arm) -> red.
#[sqlx::test(migrations = false)]
async fn results_shows_pending_card(pool: PgPool) {
    setup_demo(&pool).await;
    sqlx::query("INSERT INTO advisor_cards (site_id, slot_id, kind, state, evidence) SELECT 'demo', id, 'pause_slot', 'confident', '{}' FROM slots WHERE key = 'hero'")
        .execute(&pool)
        .await
        .unwrap();
    let a = hero_with_advisor(&pool).await.advisor.unwrap();
    assert_eq!(a.state, "card_pending");
    assert!(a.since.is_some());
}
