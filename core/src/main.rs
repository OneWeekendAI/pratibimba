//! `pratibimba serve | worker | migrate | apply <site.json> | mcp-token <site> <name> | simulate <site> ...`
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
//! - `PRB_PG_BIN`: the directory holding `pg_dump` and `psql` for `backup` / `restore`;
//!   default: `PATH`.
//! - `PRB_ASSETS_DIR` (default `prb-assets`), or `PRB_ASSETS_S3_ENDPOINT`, `_BUCKET`,
//!   `_ACCESS_KEY`, `_SECRET_KEY`, `_REGION`: where uploaded images live (see `assets`).
//!   `PRB_ASSET_MAX_BYTES` (default 5 MiB) caps an upload; `PRB_ASSET_GC_GRACE_SECS` (default
//!   7 days) is how long an unreferenced one is kept.

use anyhow::{Context, Result, bail};
use pratibimba::{db, http, site_file, worker};
use std::time::Duration;

const USAGE: &str = "usage: pratibimba <serve | worker [--once] | migrate | apply <site.json> | mcp-token <site> <name> | simulate <site> [options]
                  | erase <site> <subject> --by <actor> [--reason R] | erasures <site>
                  | backup [dir-or-file] | restore <file>>

erase <site> <subject>: right-to-erase one visitor; the subject is tombstoned and never comes back.
backup [dir-or-file]: a timestamped SQL dump (pg_dump) of the database; default: the current directory.
restore <file>: load a backup into an EMPTY database (PRB_DB), in one transaction.

simulate <site>: run synthetic visitors against the site's sandbox (<site>~sim), never the site itself.
  --days N          simulated days to run, one cohort per day (default 30)
  --visitors N      visitors per day (default 200; 0 just moves the clock)
  --rate S.V=P      planted conversion rate for variant V of slot S (repeatable)
  --default-rate P  rate for variants not named (default 0.02)
  --returning P     fraction of returning visitors (default 0)
  --seed N          RNG seed (default 1)
  --keep            continue the existing sandbox instead of resetting it";

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
    let url = db::database_url()?;
    // backup and restore drive pg_dump / psql; restore needs a database with no schema yet.
    match cmd {
        Some("backup") => {
            let out = std::path::PathBuf::from(args.get(1).map(String::as_str).unwrap_or("."));
            let file = pratibimba::backup::backup(&url, &out, pratibimba::backup::pg_bin_from_env().as_deref(), chrono::Utc::now()).await?;
            println!("{}", file.display());
            if let Some(dir) = pratibimba::assets::Assets::from_env()?.store.local_dir().filter(|d| d.is_dir()) {
                let to = pratibimba::backup::assets_dir_for(&file);
                let n = pratibimba::backup::copy_tree(&dir, &to)?;
                println!("{} ({n} asset files)", to.display());
            }
            return Ok(());
        }
        Some("restore") => {
            let file = args.get(1).context(USAGE)?;
            pratibimba::backup::restore(&url, std::path::Path::new(file), pratibimba::backup::pg_bin_from_env().as_deref()).await?;
            println!("restored {file}");
            let saved = pratibimba::backup::assets_dir_for(std::path::Path::new(file));
            if saved.is_dir() {
                match pratibimba::assets::Assets::from_env()?.store.local_dir() {
                    Some(dir) => println!("restored {} asset files into {}", pratibimba::backup::merge_tree(&saved, &dir)?, dir.display()),
                    None => eprintln!("{} holds local assets but the store is S3: copy them to the bucket yourself", saved.display()),
                }
            }
            return Ok(());
        }
        _ => {}
    }
    let pool = db::connect(&url).await?;
    match cmd {
        Some("serve") => {
            db::migrate(&pool).await?;
            // An erase made through the API needs the purge delay; refuse a bad one at start-up.
            pratibimba::privacy::purge_delay().map_err(anyhow::Error::msg)?;
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
            let edge = pratibimba::edge::Edge::from_env()?;
            tracing::info!(on = edge.is_on(), "edge snapshot publisher");
            let assets = pratibimba::assets::Assets::from_env()?;
            tracing::info!(store = %assets.store.describe(), max_bytes = assets.max_bytes, "asset store");
            let app = http::router(http::AppState::configured_full(pool, None, admin, dashboard, edge, assets), &origins);
            let bind = std::env::var("PRB_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());
            let listener = tokio::net::TcpListener::bind(&bind).await.with_context(|| format!("cannot bind {bind}"))?;
            tracing::info!(%bind, "serving");
            axum::serve(listener, app).with_graceful_shutdown(shutdown()).await?;
        }
        Some("worker") => {
            db::migrate(&pool).await?;
            if args.get(1).map(String::as_str) == Some("--once") {
                worker::run_drain_assets(&pool, &pratibimba::edge::Edge::from_env()?, &pratibimba::assets::Assets::from_env()?).await?;
                tracing::info!("worker pass done (--once)");
                return Ok(());
            }
            let tick = std::env::var("PRB_WORKER_TICK_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(1000);
            tracing::info!(tick_ms = tick, "worker running");
            tokio::select! {
                r = worker::run_loop_assets(pool, Duration::from_millis(tick), pratibimba::edge::Edge::from_env()?, pratibimba::assets::Assets::from_env()?) => r?,
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
            site_file::apply_path_edge(&pool, std::path::Path::new(path), &pratibimba::edge::Edge::from_env()?).await?;
            println!("applied {path}");
        }
        Some("mcp-token") => {
            let (Some(site), Some(name)) = (args.get(1), args.get(2)) else { bail!(USAGE) };
            db::migrate(&pool).await?;
            let token = pratibimba::mcp::issue_token(&pool, site, name).await?;
            eprintln!("MCP token for site {site:?} ({name}). It is shown once; only its hash is stored.");
            println!("{token}");
        }
        Some("simulate") => {
            db::migrate(&pool).await?;
            simulate(&pool, &args[1..]).await?;
        }
        Some("erase") => {
            let (Some(site), Some(subject)) = (args.get(1), args.get(2)) else { bail!(USAGE) };
            let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
            let Some(actor) = flag("--by") else { bail!("erase needs --by <actor>: who asked, for the audit row") };
            pratibimba::privacy::purge_delay().map_err(anyhow::Error::msg)?;
            db::migrate(&pool).await?;
            let mut row = pratibimba::privacy::erase(&pool, site, subject, &actor, flag("--reason").as_deref()).await.map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let id = row["id"].as_i64().context("an erasure has an id")?;
            row["edge"] = pratibimba::edge::erased(&pool, &pratibimba::edge::Edge::from_env()?, id).await;
            println!("{}", serde_json::to_string_pretty(&row)?);
        }
        Some("erasures") => {
            let site = args.get(1).context(USAGE)?;
            db::migrate(&pool).await?;
            for row in pratibimba::privacy::erasures(&pool, site).await.map_err(|e| anyhow::anyhow!("{e:?}"))? {
                println!("{row}");
            }
        }
        Some(other) => bail!("unknown command {other:?}\n{USAGE}"),
        None => unreachable!(),
    }
    Ok(())
}

/// `pratibimba simulate`: reset (or keep) the sandbox, time-travel day by day, print the curve.
async fn simulate(pool: &sqlx::PgPool, args: &[String]) -> Result<()> {
    use pratibimba::sim::{self, Cohort};
    let site = args.first().filter(|a| !a.starts_with("--")).context(USAGE)?;
    let (mut days, mut keep) = (30i64, false);
    let mut c = Cohort { visitors: 200, seed: 1, default_rate: 0.02, ..Cohort::default() };
    let mut it = args[1..].iter();
    while let Some(flag) = it.next() {
        let mut val = || it.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--days" => days = val()?.parse()?,
            "--visitors" => c.visitors = val()?.parse()?,
            "--default-rate" => c.default_rate = val()?.parse()?,
            "--returning" => c.returning = val()?.parse()?,
            "--seed" => c.seed = val()?.parse()?,
            "--rate" => {
                let v = val()?;
                let parsed = v.split_once('=').and_then(|(sv, p)| Some((sv.split_once('.')?, p.parse::<f64>().ok()?)));
                let Some(((slot, variant), p)) = parsed else { bail!("--rate wants slot.variant=probability, got {v:?}") };
                c.rates.entry(slot.into()).or_default().insert(variant.into(), p);
            }
            "--keep" => keep = true,
            other => bail!("unknown option {other:?}\n{USAGE}"),
        }
    }
    let api = |e: http::ApiError| anyhow::anyhow!("{e:?}");
    if !keep {
        sim::reset(pool, site, chrono::Utc::now()).await.map_err(api)?;
    }
    println!("sandbox {}  (the real site {site:?} is not touched)", sim::sandbox_id(site));
    let cohort = (c.visitors > 0).then_some(&c);
    for d in sim::advance(pool, site, days * sim::DAY, cohort).await.map_err(api)? {
        let served: Vec<String> = d
            .served
            .iter()
            .map(|(slot, by)| {
                let total: u64 = by.values().sum();
                let parts: Vec<String> = by.iter().map(|(v, n)| format!("{v} {:.0}%", 100.0 * *n as f64 / total.max(1) as f64)).collect();
                format!("{slot}: {}", parts.join(", "))
            })
            .collect();
        println!("day {:>3}  {}  conv {:>4}  {}", d.day, d.at.format("%Y-%m-%d"), d.conversions.values().sum::<u64>(), served.join("  |  "));
    }
    let status = sim::status(pool, site).await.map_err(api)?;
    for slot in status["results"]["slots"].as_array().into_iter().flatten() {
        for v in slot["variants"].as_array().into_iter().flatten() {
            let (n, k) = (v["counts"]["n"].as_u64().unwrap_or(0), v["counts"]["conversions"].as_u64().unwrap_or(0));
            println!("{}.{:<16} n {:>6}  conversions {:>5}  rate {:.2}%", slot["key"].as_str().unwrap_or("?"), v["key"].as_str().unwrap_or("?"), n, k, 100.0 * k as f64 / n.max(1) as f64);
        }
    }
    println!("clock now {}", status["clock"].as_str().unwrap_or("?"));
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
