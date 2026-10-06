//! Shared test helpers. Tests need `DATABASE_URL` pointing at a Postgres server where the
//! user may create databases; `#[sqlx::test]` makes one per test, and `FreshDb` does the same
//! for tests that need a multi-thread runtime (sqlx::test cannot select a flavour).
#![allow(dead_code)]

use sqlx::{Connection, PgConnection, PgPool, postgres::{PgConnectOptions, PgPoolOptions}};
use std::str::FromStr;

pub struct FreshDb {
    pub pool: PgPool,
    name: String,
}

impl FreshDb {
    pub async fn new() -> Self {
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for tests");
        let name = format!("prb_t_{}", uuid::Uuid::new_v4().simple());
        let mut admin = PgConnection::connect(&url).await.unwrap();
        sqlx::query(&format!("CREATE DATABASE {name}")).execute(&mut admin).await.unwrap();
        let opts = PgConnectOptions::from_str(&url).unwrap().database(&name);
        let pool = PgPoolOptions::new().max_connections(20).connect_with(opts).await.unwrap();
        FreshDb { pool, name }
    }

    pub async fn drop(self) {
        self.pool.close().await;
        let url = std::env::var("DATABASE_URL").unwrap();
        let mut admin = PgConnection::connect(&url).await.unwrap();
        sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", self.name)).execute(&mut admin).await.unwrap();
    }
}

pub async fn table_exists(pool: &PgPool, name: &str) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema='public' AND table_name=$1)",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap()
}
