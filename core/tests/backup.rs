//! `pratibimba backup` / `restore`: a pg_dump of a live install restores into a fresh database
//! with every row matching, leaves the privacy salts out, refuses a non-empty target, and the
//! restored install keeps working (migrations are a no-op, sequences continue).

mod common;
use chrono::Utc;
use common::*;
use pratibimba::{backup, privacy, worker};
use serde_json::json;
use sqlx::{Connection, PgConnection, PgPool, postgres::PgConnectOptions};
use std::path::{Path, PathBuf};
use std::str::FromStr;

struct Db {
    url: String,
    pool: PgPool,
    name: String,
}

async fn fresh() -> Db {
    let base = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for tests");
    let name = format!("prb_b_{}", uuid::Uuid::new_v4().simple());
    let mut admin = PgConnection::connect(&base).await.unwrap();
    sqlx::query(&format!("CREATE DATABASE {name}")).execute(&mut admin).await.unwrap();
    let opts = PgConnectOptions::from_str(&base).unwrap().database(&name);
    let pool = PgPool::connect_with(opts).await.unwrap();
    // Same server and credentials, another database: swap the path of the URL.
    let (head, _) = base.rsplit_once('/').unwrap();
    Db { url: format!("{head}/{name}"), pool, name }
}

impl Db {
    async fn drop(self) {
        self.pool.close().await;
        let mut admin = PgConnection::connect(&std::env::var("DATABASE_URL").unwrap()).await.unwrap();
        sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", self.name)).execute(&mut admin).await.unwrap();
    }
}

/// Where the Postgres client tools are: `PRB_PG_BIN`, then `PATH`, then Homebrew's 17.
fn pg_bin() -> Option<PathBuf> {
    if let Some(d) = backup::pg_bin_from_env() {
        return Some(d);
    }
    let on_path = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("pg_dump").is_file()));
    if on_path {
        return None;
    }
    ["/opt/homebrew/opt/postgresql@17/bin", "/usr/local/opt/postgresql@17/bin", "/usr/lib/postgresql/17/bin"]
        .iter()
        .map(PathBuf::from)
        .find(|d| d.join("pg_dump").is_file())
}

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("prb-backup-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A live install: decisions, events, a credited conversion, a visitor, an erasure, a salt.
async fn populate(pool: &PgPool) {
    setup_learning(pool).await;
    let app = app(pool);
    for who in ["alice", "bob", "carol"] {
        let d = decide(&app, who, true).await.json();
        let id = d["decision_id"].as_str().unwrap().to_string();
        event(&app, who, "exposure", Some(&id)).await;
        event(&app, who, "click", Some(&id)).await;
    }
    worker::run_once(pool, Utc::now()).await.unwrap();
    privacy::erase(pool, "demo", "carol", "owner", Some("asked")).await.unwrap();
    privacy::salt(pool, "demo", Utc::now().date_naive()).await.unwrap();
}

/// Every public table's rows, as one sorted text per table.
async fn snapshot(pool: &PgPool) -> Vec<(String, i64, String)> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables WHERE table_schema = 'public' AND table_type = 'BASE TABLE' ORDER BY 1",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let mut out = vec![];
    for t in tables {
        let (n, digest): (i64, Option<String>) =
            sqlx::query_as(&format!("SELECT count(*), md5(string_agg(x::text, '|' ORDER BY x::text)) FROM \"{t}\" x")).fetch_one(pool).await.unwrap();
        out.push((t, n, digest.unwrap_or_default()));
    }
    out
}

async fn backed_up(src: &Db, dir: &Path) -> PathBuf {
    backup::backup(&src.url, dir, pg_bin().as_deref(), Utc::now()).await.unwrap()
}

#[tokio::test]
async fn a_backup_restores_into_a_fresh_database_with_matching_rows() {
    let (src, dst, dir) = (fresh().await, fresh().await, scratch());
    populate(&src.pool).await;
    let file = backed_up(&src, &dir).await;
    backup::restore(&dst.url, &file, pg_bin().as_deref()).await.unwrap();
    let keep = |s: Vec<(String, i64, String)>| s.into_iter().filter(|(t, ..)| !backup::EXCLUDED_DATA.contains(&t.as_str())).collect::<Vec<_>>();
    let (a, b) = (keep(snapshot(&src.pool).await), keep(snapshot(&dst.pool).await));
    assert!(a.iter().any(|(t, n, _)| t == "decisions" && *n == 3), "the fixture has decisions: {a:?}");
    assert_eq!(a, b);
    src.drop().await;
    dst.drop().await;
}

#[tokio::test]
async fn a_backup_leaves_the_privacy_salts_out() {
    let (src, dst, dir) = (fresh().await, fresh().await, scratch());
    populate(&src.pool).await;
    let file = backed_up(&src, &dir).await;
    backup::restore(&dst.url, &file, pg_bin().as_deref()).await.unwrap();
    assert_eq!(count(&src.pool, "SELECT count(*) FROM privacy_salts").await, 1);
    assert_eq!(count(&dst.pool, "SELECT count(*) FROM privacy_salts").await, 0);
    src.drop().await;
    dst.drop().await;
}

#[tokio::test]
async fn a_backup_file_is_named_by_its_time() {
    let (src, dir) = (fresh().await, scratch());
    populate(&src.pool).await;
    let file = backed_up(&src, &dir).await;
    let name = file.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("pratibimba-backup-") && name.ends_with("Z.sql"), "{name}");
    src.drop().await;
}

#[tokio::test]
async fn a_backup_never_overwrites_a_file() {
    let (src, dir) = (fresh().await, scratch());
    populate(&src.pool).await;
    let target = dir.join("b.sql");
    std::fs::write(&target, "keep me").unwrap();
    assert!(backup::backup(&src.url, &target, pg_bin().as_deref(), Utc::now()).await.is_err());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep me");
    src.drop().await;
}

#[tokio::test]
async fn restore_refuses_a_database_that_has_tables() {
    let (src, dst, dir) = (fresh().await, fresh().await, scratch());
    populate(&src.pool).await;
    let file = backed_up(&src, &dir).await;
    pratibimba::db::migrate(&dst.pool).await.unwrap();
    let e = backup::restore(&dst.url, &file, pg_bin().as_deref()).await.expect_err("refused").to_string();
    assert!(e.contains("empty database"), "{e}");
    assert_eq!(count(&dst.pool, "SELECT count(*) FROM sites").await, 0);
    src.drop().await;
    dst.drop().await;
}

#[tokio::test]
async fn a_failed_restore_writes_nothing() {
    let (dst, dir) = (fresh().await, scratch());
    let file = dir.join("broken.sql");
    std::fs::write(&file, "CREATE TABLE a (x int);\nTHIS IS NOT SQL;\n").unwrap();
    assert!(backup::restore(&dst.url, &file, pg_bin().as_deref()).await.is_err());
    assert_eq!(count(&dst.pool, "SELECT count(*) FROM information_schema.tables WHERE table_schema = 'public'").await, 0);
    dst.drop().await;
}

#[tokio::test]
async fn a_restored_install_migrates_as_a_noop_and_keeps_working() {
    let (src, dst, dir) = (fresh().await, fresh().await, scratch());
    populate(&src.pool).await;
    let file = backed_up(&src, &dir).await;
    backup::restore(&dst.url, &file, pg_bin().as_deref()).await.unwrap();
    let before = count(&dst.pool, "SELECT count(*) FROM schema_migrations").await;
    pratibimba::db::migrate(&dst.pool).await.unwrap();
    assert_eq!(count(&dst.pool, "SELECT count(*) FROM schema_migrations").await, before);
    // Sequences came along: a new row gets a fresh id, and the tombstone still refuses.
    insert_variant(&dst.pool, "late", "draft").await;
    let r = decide(&app(&dst.pool), "dave", false).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let gone = post(&app(&dst.pool), "/decide", json!({ "site": "demo", "slot": "hero", "subject": "carol" })).await;
    assert_eq!(gone.status, StatusCode::GONE, "{}", gone.text);
    src.drop().await;
    dst.drop().await;
}
