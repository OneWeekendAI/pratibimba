//! Connection and migrations.
//!
//! Migrations are embedded with `include_str!` so `serve` and `worker` carry the same schema
//! and need no files beside the binary. A `schema_migrations` ledger records what ran; each
//! file runs in its own transaction together with its ledger row, so a failed file leaves no
//! trace. A session advisory lock serialises concurrent starts (a `serve` and a `worker`
//! booting together must not both try to create the same tables).

use anyhow::{Context, Result};
use sqlx::{PgPool, postgres::PgPoolOptions};

pub const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("../migrations/0001_init.sql")),
    ("0002_phase1", include_str!("../migrations/0002_phase1.sql")),
    ("0003_mcp", include_str!("../migrations/0003_mcp.sql")),
    ("0004_kill_switch", include_str!("../migrations/0004_kill_switch.sql")),
    ("0005_grounding", include_str!("../migrations/0005_grounding.sql")),
    ("0006_webhooks", include_str!("../migrations/0006_webhooks.sql")),
    ("0010_insights", include_str!("../migrations/0010_insights.sql")),
    ("0020_adapt", include_str!("../migrations/0020_adapt.sql")),
    ("0030_sim_conversions", include_str!("../migrations/0030_sim_conversions.sql")),
    ("0040_audiences", include_str!("../migrations/0040_audiences.sql")),
    ("0060_privacy", include_str!("../migrations/0060_privacy.sql")),
    ("0070_agent_tools", include_str!("../migrations/0070_agent_tools.sql")),
    ("0080_cold_start", include_str!("../migrations/0080_cold_start.sql")),
    ("0090_features", include_str!("../migrations/0090_features.sql")),
    ("0100_attribution", include_str!("../migrations/0100_attribution.sql")),
    ("0110_rollup_credit", include_str!("../migrations/0110_rollup_credit.sql")),
    ("0120_erase_schedule", include_str!("../migrations/0120_erase_schedule.sql")),
    ("0130_plugins", include_str!("../migrations/0130_plugins.sql")),
    ("0140_plugin_requests", include_str!("../migrations/0140_plugin_requests.sql")),
    ("0150_experiments", include_str!("../migrations/0150_experiments.sql")),
    ("0160_learn_at_close", include_str!("../migrations/0160_learn_at_close.sql")),
    ("0170_edge_snapshots", include_str!("../migrations/0170_edge_snapshots.sql")),
    ("0180_edge_decisions", include_str!("../migrations/0180_edge_decisions.sql")),
    ("0190_edge_tombstones", include_str!("../migrations/0190_edge_tombstones.sql")),
    ("0200_commerce", include_str!("../migrations/0200_commerce.sql")),
    ("0210_sorted", include_str!("../migrations/0210_sorted.sql")),
    ("0220_history", include_str!("../migrations/0220_history.sql")),
    ("0230_copy", include_str!("../migrations/0230_copy.sql")),
    ("0240_share_edits", include_str!("../migrations/0240_share_edits.sql")),
    ("0250_file_share", include_str!("../migrations/0250_file_share.sql")),
    ("0260_share_webhook", include_str!("../migrations/0260_share_webhook.sql")),
];

/// Arbitrary constant: "prb" in ASCII.
const MIGRATION_LOCK: i64 = 0x707262;

pub fn database_url() -> Result<String> {
    std::env::var("PRB_DB")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .context("set PRB_DB (or DATABASE_URL) to a Postgres URL")
}

pub async fn connect(url: &str) -> Result<PgPool> {
    let max = std::env::var("PRB_DB_MAX_CONNECTIONS").ok().and_then(|v| v.parse().ok()).unwrap_or(10);
    pool_options().max_connections(max).connect(url).await.context("failed to connect to Postgres")
}

/// A pooled connection idle longer than this is pinged before it is handed out.
pub const PING_AFTER_IDLE: std::time::Duration = std::time::Duration::from_secs(30);

/// The pool's options. sqlx pings every connection it hands out by default, which is one more
/// round trip per acquire: on a hosted database that is a few milliseconds each, several times
/// per decide. A connection used in the last [`PING_AFTER_IDLE`] is handed out as is; an older
/// one (a pooler or a NAT may have dropped it while the instance idled) is still pinged first.
pub fn pool_options() -> PgPoolOptions {
    PgPoolOptions::new().test_before_acquire(false).before_acquire(|conn, meta| {
        Box::pin(async move {
            if meta.idle_for > PING_AFTER_IDLE {
                sqlx::Connection::ping(conn).await?;
            }
            Ok(true)
        })
    })
}

pub async fn migrate(pool: &PgPool) -> Result<()> {
    migrate_with(pool, MIGRATIONS).await
}

pub async fn migrate_with(pool: &PgPool, migrations: &[(&str, &str)]) -> Result<()> {
    // The lock is session-level: one held connection owns it while the migrations themselves
    // run on other pooled connections. A second migrator blocks here until the first is done.
    let mut lock = pool.acquire().await?;
    sqlx::query("SELECT pg_advisory_lock($1)").bind(MIGRATION_LOCK).execute(&mut *lock).await?;
    let result = apply(pool, migrations).await;
    sqlx::query("SELECT pg_advisory_unlock($1)").bind(MIGRATION_LOCK).execute(&mut *lock).await?;
    result
}

async fn apply(pool: &PgPool, migrations: &[(&str, &str)]) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version     TEXT PRIMARY KEY,
             applied_at  TIMESTAMPTZ NOT NULL DEFAULT now()
         )",
    )
    .execute(pool)
    .await
    .context("failed to create schema_migrations")?;

    let applied: Vec<String> =
        sqlx::query_scalar("SELECT version FROM schema_migrations").fetch_all(pool).await?;

    for &(version, sql) in migrations {
        if applied.iter().any(|v| v == version) {
            continue;
        }
        let mut tx = pool.begin().await?;
        // A bare &str with no bind arguments goes over the simple-query protocol, so a file may
        // hold several statements. (`sqlx::raw_sql` does the same but its future is not
        // provably Send, which breaks spawning `migrate`.)
        sqlx::Executor::execute(&mut *tx, sql).await.with_context(|| format!("migration {version} failed"))?;
        sqlx::query("INSERT INTO schema_migrations (version) VALUES ($1)").bind(version).execute(&mut *tx).await?;
        tx.commit().await?;
        tracing::info!(version, "migrated");
    }
    Ok(())
}
