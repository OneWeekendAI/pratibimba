//! `pratibimba backup` and `pratibimba restore` (backend.md, "Deployment": "a `pratibimba
//! backup` subcommand that produces a timestamped SQL dump").
//!
//! A backup is a plain-SQL logical dump made by `pg_dump`, so it restores with nothing but
//! `psql` and survives a Postgres major upgrade. Two choices:
//!
//! - **The salts stay out.** The rows of `privacy_salts` are never dumped. A cookieless
//!   pseudonym is unlinkable only because yesterday's salt is gone; a backup that kept it would
//!   undo that for as long as the file lives. A restored install makes a new salt on first use,
//!   so cookieless visitors get a new pseudonym the day of a restore.
//! - **Restore only into an empty database.** Replaying a dump over live tables would fail
//!   halfway or merge two histories, so `restore` refuses a database that has any table in
//!   `public`, and runs the whole file in one transaction (`--single-transaction`,
//!   `ON_ERROR_STOP`): it lands completely or not at all.
//!
//! - **Local assets travel with it.** With the asset store on a local directory
//!   (`PRB_ASSETS_DIR`), `pratibimba backup` also copies that directory to `<dump>.assets/`, and
//!   `restore` copies it back. An S3 bucket is not copied: back it up with the bucket's own
//!   versioning or replication.
//!
//! The tools come from `PRB_PG_BIN` (a directory) when set, else from `PATH`. `pg_dump` must be
//! at least the server's major version; the Docker image carries Debian's client.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

/// Tables whose rows a backup leaves out (their schema is kept).
pub const EXCLUDED_DATA: [&str; 1] = ["privacy_salts"];

/// `pratibimba-backup-20261007T031500Z.sql`: sorts by time, and has no `:` (Windows, `scp`).
pub fn file_name(at: DateTime<Utc>) -> String {
    format!("pratibimba-backup-{}.sql", at.format("%Y%m%dT%H%M%SZ"))
}

/// A Postgres client tool: `<pg_bin>/<name>` when a directory is given, else `name` on `PATH`.
pub fn tool(pg_bin: Option<&Path>, name: &str) -> PathBuf {
    match pg_bin {
        Some(dir) => dir.join(name),
        None => PathBuf::from(name),
    }
}

/// The directory `PRB_PG_BIN` names, if any.
pub fn pg_bin_from_env() -> Option<PathBuf> {
    std::env::var_os("PRB_PG_BIN").filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Dump the database at `url`. `out` is a directory (the file gets [`file_name`]) or a file
/// path. Returns the file written.
pub async fn backup(url: &str, out: &Path, pg_bin: Option<&Path>, at: DateTime<Utc>) -> Result<PathBuf> {
    let file = if out.is_dir() { out.join(file_name(at)) } else { out.to_path_buf() };
    if file.exists() {
        bail!("{} already exists; a backup never overwrites one", file.display());
    }
    // pg_dump refuses a newer server with a terse "aborting because of server version
    // mismatch"; say which two versions disagree and what to do about it.
    let version = tokio::process::Command::new(tool(pg_bin, "pg_dump"))
        .arg("--version")
        .output()
        .await
        .context("cannot run pg_dump (set PRB_PG_BIN, or put it on PATH)")?;
    let pool = crate::db::connect(url).await?;
    let server: i64 = sqlx::query_scalar("SELECT current_setting('server_version_num')::int8").fetch_one(&pool).await?;
    pool.close().await;
    check_versions(&String::from_utf8_lossy(&version.stdout), server)?;
    let mut cmd = tokio::process::Command::new(tool(pg_bin, "pg_dump"));
    cmd.arg(format!("--dbname={url}")).args(["--format=plain", "--no-owner", "--no-privileges", "--quote-all-identifiers"]);
    for t in EXCLUDED_DATA {
        cmd.arg(format!("--exclude-table-data=public.{t}"));
    }
    cmd.arg(format!("--file={}", file.display()));
    let out = cmd.output().await.context("cannot run pg_dump (set PRB_PG_BIN, or put it on PATH)")?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&file);
        bail!("pg_dump failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(file)
}

/// `pg_dump` dumps a server of its own major version or older, never a newer one.
/// `client` is `pg_dump --version` output; `server_num` is `server_version_num` (170006).
pub fn check_versions(client: &str, server_num: i64) -> Result<()> {
    let client_major: i64 = client
        .split_whitespace()
        .find_map(|w| w.split('.').next()?.parse().ok())
        .with_context(|| format!("cannot read the pg_dump version from {:?}", client.trim()))?;
    let server_major = server_num / 10000;
    if server_major > client_major {
        bail!(
            "the database is PostgreSQL {server_major} but pg_dump is {client_major}; pg_dump cannot dump a newer server. \
             Use a pg_dump {server_major} or later (set PRB_PG_BIN to its bin directory), or keep the server at {client_major}"
        );
    }
    Ok(())
}

/// Load a backup into the empty database at `url`, in one transaction.
pub async fn restore(url: &str, file: &Path, pg_bin: Option<&Path>) -> Result<()> {
    if !file.is_file() {
        bail!("{} is not a backup file", file.display());
    }
    let pool = crate::db::connect(url).await?;
    let tables: i64 = sqlx::query_scalar("SELECT count(*) FROM information_schema.tables WHERE table_schema = 'public'").fetch_one(&pool).await?;
    pool.close().await;
    if tables > 0 {
        bail!("the target database already has {tables} tables; restore only into an empty database");
    }
    let out = tokio::process::Command::new(tool(pg_bin, "psql"))
        .arg(format!("--dbname={url}"))
        .args(["--no-psqlrc", "--quiet", "--single-transaction", "--set=ON_ERROR_STOP=1"])
        .arg(format!("--file={}", file.display()))
        .output()
        .await
        .context("cannot run psql (set PRB_PG_BIN, or put it on PATH)")?;
    if !out.status.success() {
        bail!("restore failed, nothing was written: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Where a backup's copy of the local asset directory goes: beside the dump, `<dump>.assets/`.
pub fn assets_dir_for(dump: &Path) -> PathBuf {
    let mut name = dump.file_name().unwrap_or_default().to_os_string();
    name.push(".assets");
    dump.with_file_name(name)
}

/// Copy a directory tree (the local asset store, S11) to `to`, which must not exist yet.
/// Returns the number of files copied. Temporary `.part` files are skipped.
pub fn copy_tree(from: &Path, to: &Path) -> Result<u64> {
    if to.exists() {
        bail!("{} already exists; a backup never overwrites one", to.display());
    }
    fn walk(from: &Path, to: &Path, n: &mut u64) -> Result<()> {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            let name = e.file_name();
            if name.to_string_lossy().ends_with(".part") {
                continue;
            }
            let ty = e.file_type()?;
            if ty.is_dir() {
                walk(&e.path(), &to.join(&name), n)?;
            } else if ty.is_file() {
                std::fs::copy(e.path(), to.join(&name))?;
                *n += 1;
            }
        }
        Ok(())
    }
    let mut n = 0;
    walk(from, to, &mut n)?;
    Ok(n)
}

/// Copy a backup's assets into the (local) store directory, keeping files already there: an
/// asset's name is its hash, so a file of the same name holds the same bytes.
pub fn merge_tree(from: &Path, to: &Path) -> Result<u64> {
    let mut n = 0;
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            n += merge_tree(&e.path(), &dest)?;
        } else if !dest.exists() {
            std::fs::copy(e.path(), &dest)?;
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn backup_file_name_is_timestamped() {
        let at = Utc.with_ymd_and_hms(2026, 10, 7, 3, 15, 9).unwrap();
        assert_eq!(file_name(at), "pratibimba-backup-20261007T031509Z.sql");
    }

    #[test]
    fn tool_uses_the_given_directory() {
        assert_eq!(tool(Some(Path::new("/opt/pg/bin")), "pg_dump"), PathBuf::from("/opt/pg/bin/pg_dump"));
    }

    #[test]
    fn pg_dump_may_be_newer_or_equal() {
        assert!(check_versions("pg_dump (PostgreSQL) 17.6 (Debian 17.6-1)\n", 170006).is_ok());
        assert!(check_versions("pg_dump (PostgreSQL) 18.1\n", 160004).is_ok());
    }

    #[test]
    fn pg_dump_older_than_the_server_is_named() {
        let e = check_versions("pg_dump (PostgreSQL) 17.6 (Debian 17.6-1)\n", 180001).unwrap_err().to_string();
        assert!(e.contains("PostgreSQL 18 but pg_dump is 17"), "{e}");
    }

    #[test]
    fn unreadable_pg_dump_version_is_an_error() {
        assert!(check_versions("garbage", 170006).is_err());
    }

    #[test]
    fn assets_go_beside_the_dump() {
        assert_eq!(assets_dir_for(Path::new("/b/pratibimba-backup-1.sql")), PathBuf::from("/b/pratibimba-backup-1.sql.assets"));
    }

    #[test]
    fn copy_tree_copies_files_and_refuses_an_existing_target() {
        let root = std::env::temp_dir().join(format!("prb-copytree-{}", uuid::Uuid::new_v4().simple()));
        let src = root.join("src");
        std::fs::create_dir_all(src.join("demo")).unwrap();
        std::fs::write(src.join("demo").join("a"), b"1").unwrap();
        std::fs::write(src.join("demo").join(".x.part"), b"half").unwrap();
        assert_eq!(copy_tree(&src, &root.join("out")).unwrap(), 1);
        assert_eq!(std::fs::read(root.join("out").join("demo").join("a")).unwrap(), b"1");
        assert!(!root.join("out").join("demo").join(".x.part").exists());
        assert!(copy_tree(&src, &root.join("out")).is_err());
        assert_eq!(merge_tree(&root.join("out"), &root.join("store")).unwrap(), 1);
        assert_eq!(merge_tree(&root.join("out"), &root.join("store")).unwrap(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tool_falls_back_to_path() {
        assert_eq!(tool(None, "psql"), PathBuf::from("psql"));
    }
}
