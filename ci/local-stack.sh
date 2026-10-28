#!/usr/bin/env bash
# The whole thing on this machine, nothing hosted: Postgres on 55440, Core on :8080 (dashboard at
# http://localhost:8080/app, sign in with $PRB_ADMIN_TOKEN) and the worker. Ctrl-C stops Core and
# the worker (Postgres keeps running). Point a site of your own at http://127.0.0.1:8080.
#
#   ci/local-stack.sh             build what is stale, then run
#   ci/local-stack.sh --fresh     drop and recreate the prb_local database first
#
# Env: PRB_PGDATA (default ../pratibimba-demo/pgdata, outside git), PRB_ADMIN_TOKEN (default
# local-admin-token).
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
PG_BIN=${PG_BIN:-/opt/homebrew/opt/postgresql@17/bin}
PGDATA=${PRB_PGDATA:-$ROOT/../pratibimba-demo/pgdata}
ADMIN_URL=postgres://postgres@localhost:55440/postgres
export PRB_DB=postgres://postgres@localhost:55440/prb_local
export PRB_ADMIN_TOKEN=${PRB_ADMIN_TOKEN:-local-admin-token}
export PRB_BIND=127.0.0.1:8080 PRB_DASHBOARD_DIR=$ROOT/dashboard/out PRB_WORKER_TICK_MS=1000 RUST_LOG=${RUST_LOG:-warn}
for v in $(env | sed -n 's/^\(PRB_EDGE_[A-Z_]*\)=.*/\1/p'); do unset "$v"; done # local: no edge

# 1. Postgres (trust auth, superuser `postgres`, the role the test suites expect).
if ! "$PG_BIN/pg_isready" -q -h localhost -p 55440; then
  if [ ! -f "$PGDATA/PG_VERSION" ]; then
    mkdir -p "$PGDATA"
    "$PG_BIN/initdb" -D "$PGDATA" -U postgres --auth=trust -E UTF8 >/dev/null
  fi
  "$PG_BIN/pg_ctl" -D "$PGDATA" -l "$PGDATA/../pg.log" -o "-p 55440 -k /tmp -c listen_addresses=localhost -c max_connections=300" start
fi
if [ "${1:-}" = --fresh ]; then psql "$ADMIN_URL" -qc "DROP DATABASE IF EXISTS prb_local WITH (FORCE)"; fi
psql "$ADMIN_URL" -qAtc "SELECT 1 FROM pg_database WHERE datname='prb_local'" | grep -q 1 || psql "$ADMIN_URL" -qc "CREATE DATABASE prb_local"

# 2. Builds: Core and the dashboard (static export served by Core).
cargo build -p pratibimba
BIN=$ROOT/target/debug/pratibimba
[ -f dashboard/out/index.html ] || (cd dashboard && npm run build)

# 3. Run.
pids=()
trap 'kill "${pids[@]}" 2>/dev/null' EXIT INT TERM
"$BIN" serve & pids+=($!)
"$BIN" worker & pids+=($!)
until curl -sf http://127.0.0.1:8080/health >/dev/null; do sleep 0.5; done
echo "local stack up: dashboard http://localhost:8080/app (token: $PRB_ADMIN_TOKEN)"
wait
