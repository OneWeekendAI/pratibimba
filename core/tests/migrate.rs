mod common;
use common::{FreshDb, table_exists};
use pratibimba::db;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::Barrier;

async fn seed_slot(pool: &PgPool) -> i64 {
    sqlx::query("INSERT INTO sites (id, origin) VALUES ('s', 'http://x')").execute(pool).await.unwrap();
    sqlx::query_scalar("INSERT INTO slots (site_id, key, selector) VALUES ('s','hero','#hero') RETURNING id")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
async fn fresh_database_gets_all_tables(pool: PgPool) {
    db::migrate(&pool).await.unwrap();
    for t in ["sites", "slots", "variants", "models", "decisions", "events", "pending_events"] {
        assert!(table_exists(&pool, t).await, "missing table {t}");
    }
}

#[sqlx::test(migrations = false)]
async fn migrate_twice_is_a_noop(pool: PgPool) {
    db::migrate(&pool).await.unwrap();
    db::migrate(&pool).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM schema_migrations").fetch_one(&pool).await.unwrap();
    assert_eq!(n, db::MIGRATIONS.len() as i64);
}

#[sqlx::test(migrations = false)]
async fn failed_migration_leaves_no_trace(pool: PgPool) {
    let migs = [("0001_ok", "CREATE TABLE ok_t (id int)"), ("0002_bad", "CREATE TABLE half_t (id int); SELECT nonsense_column FROM ok_t")];
    assert!(db::migrate_with(&pool, &migs).await.is_err());
    assert!(table_exists(&pool, "ok_t").await);
    assert!(!table_exists(&pool, "half_t").await, "a failed migration must roll back its own DDL");
    let bad: i64 = sqlx::query_scalar("SELECT count(*) FROM schema_migrations WHERE version='0002_bad'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(bad, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_migrations_both_succeed() {
    // Several rounds on fresh databases: one unlucky interleaving is enough to fail.
    for _ in 0..5 {
        let db = FreshDb::new().await;
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = vec![];
        for _ in 0..2 {
            let (pool, b) = (db.pool.clone(), barrier.clone());
            handles.push(tokio::spawn(async move {
                b.wait().await;
                db::migrate(&pool).await
            }));
        }
        for h in handles {
            h.await.unwrap().expect("a concurrent migrate failed");
        }
        db.drop().await;
    }
}

#[sqlx::test(migrations = false)]
async fn one_baseline_per_slot_enforced(pool: PgPool) {
    db::migrate(&pool).await.unwrap();
    let slot = seed_slot(&pool).await;
    sqlx::query("INSERT INTO variants (slot_id, key, is_baseline) VALUES ($1, 'a', true)").bind(slot).execute(&pool).await.unwrap();
    let second = sqlx::query("INSERT INTO variants (slot_id, key, is_baseline) VALUES ($1, 'b', true)").bind(slot).execute(&pool).await;
    assert!(second.is_err(), "two baselines in one slot were accepted");
}

#[sqlx::test(migrations = false)]
async fn approved_requires_approver(pool: PgPool) {
    db::migrate(&pool).await.unwrap();
    let slot = seed_slot(&pool).await;
    let ok = sqlx::query("INSERT INTO variants (slot_id, key, status, approved_by) VALUES ($1, 'a', 'approved', 'me')")
        .bind(slot)
        .execute(&pool)
        .await;
    assert!(ok.is_ok(), "the accepting case must pass: {ok:?}");
    let bad = sqlx::query("INSERT INTO variants (slot_id, key, status) VALUES ($1, 'b', 'approved')").bind(slot).execute(&pool).await;
    assert!(bad.is_err(), "approved without approver was accepted");
}

/// The file's SQL and its ledger row commit together. Postgres already runs one multi-
/// statement simple query atomically, so the only failure our transaction adds protection
/// against is one AFTER the file ran: here the file succeeds and the ledger insert fails.
#[sqlx::test(migrations = false)]
async fn migration_and_ledger_row_commit_together(pool: PgPool) {
    let migs = [("0001_sabotage", "CREATE TABLE half_t (id int); DROP TABLE schema_migrations")];
    assert!(db::migrate_with(&pool, &migs).await.is_err());
    assert!(!table_exists(&pool, "half_t").await, "the file committed without its ledger row");
    assert!(table_exists(&pool, "schema_migrations").await);
}
