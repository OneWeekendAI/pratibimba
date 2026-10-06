//! `pratibimba serve | worker | migrate | apply <site.json> | mcp-token <site> <name>`
//!
//! One binary, several entry points, so the HTTP server and the learning worker always agree
//! on schema and policy. Configuration is environment only:
//!
//! - `PRB_DB` (or `DATABASE_URL`): Postgres URL. Required.
//! - `PRB_BIND`: listen address for `serve`, default `0.0.0.0:8080`.
//! - `PRB_SITE_ORIGIN`: comma-separated origins allowed by CORS. Needed only when the page is not
//!   served from the same origin as Core (dev setups); production proxies `/_prb/*` instead.
//! - `PRB_WORKER_TICK_MS`: how often `worker` closes attribution windows, default 1000.
//! - `PRB_ADMIN_TOKEN`: turns on the dashboard API (`/api`); without it `/api` answers 503.
//! - `PRB_DASHBOARD_DIR`: the dashboard's static export, served at `/app`.

use anyhow::{Context, Result, bail};
use pratibimba::{db, http, site_file, worker};
use std::time::Duration;

const USAGE: &str = "usage: pratibimba <serve | worker | migrate | apply <site.json> | mcp-token <site> <name>>";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        // Logs go to stderr so stdout carries only a command's output (`mcp-token` prints the token).
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str);
    if matches!(cmd, None | Some("help" | "-h" | "--help")) {
        println!("{USAGE}");
        return Ok(());
    }
    let pool = db::connect(&db::database_url()?).await?;
    match cmd {
        Some("serve") => {
            db::migrate(&pool).await?;
            let origins: Vec<String> = std::env::var("PRB_SITE_ORIGIN")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
            let admin = std::env::var("PRB_ADMIN_TOKEN").ok();
            if admin.as_deref().is_none_or(str::is_empty) {
                tracing::warn!("PRB_ADMIN_TOKEN is not set: the dashboard API (/api) is off");
            }
            let dashboard = std::env::var("PRB_DASHBOARD_DIR").ok().map(std::path::PathBuf::from).filter(|d| d.join("index.html").exists());
            match &dashboard {
                Some(d) => tracing::info!(dir = %d.display(), "serving the dashboard at /app"),
                None => tracing::info!("no dashboard: set PRB_DASHBOARD_DIR to a built dashboard (dashboard/out)"),
            }
            let app = http::router(http::AppState::configured(pool, None, admin, dashboard), &origins);
            let bind = std::env::var("PRB_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());
            let listener = tokio::net::TcpListener::bind(&bind).await.with_context(|| format!("cannot bind {bind}"))?;
            tracing::info!(%bind, "serving");
            axum::serve(listener, app).with_graceful_shutdown(shutdown()).await?;
        }
        Some("worker") => {
            db::migrate(&pool).await?;
            let tick = std::env::var("PRB_WORKER_TICK_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(1000);
            tracing::info!(tick_ms = tick, "worker running");
            tokio::select! {
                r = worker::run_loop(pool, Duration::from_millis(tick)) => r?,
                _ = shutdown() => {}
            }
        }
        Some("migrate") => {
            db::migrate(&pool).await?;
            println!("migrations applied");
        }
        Some("apply") => {
            let path = args.get(1).context(USAGE)?;
            db::migrate(&pool).await?;
            site_file::apply_path(&pool, std::path::Path::new(path)).await?;
            println!("applied {path}");
        }
        Some("mcp-token") => {
            let (Some(site), Some(name)) = (args.get(1), args.get(2)) else { bail!(USAGE) };
            db::migrate(&pool).await?;
            let token = pratibimba::mcp::issue_token(&pool, site, name).await?;
            eprintln!("MCP token for site {site:?} ({name}). It is shown once; only its hash is stored.");
            println!("{token}");
        }
        Some(other) => bail!("unknown command {other:?}\n{USAGE}"),
        None => unreachable!(),
    }
    Ok(())
}

/// Ctrl-C locally, SIGTERM from Docker.
async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}
