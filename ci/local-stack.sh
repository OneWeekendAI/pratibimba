#!/usr/bin/env bash
# The whole thing on this machine, nothing hosted: Postgres on 55440, Core on :8080 (dashboard at
# http://localhost:8080/app, sign in with $PRB_ADMIN_TOKEN), the worker, and examples/shop on :3200
# talking to that Core. Ctrl-C stops Core, the worker and the shop (Postgres keeps running).
#
#   ci/local-stack.sh             build what is stale, then run
#   ci/local-stack.sh --fresh     drop and recreate the prb_local database first
#
# Env: PRB_PGDATA (default ../pratibimba-demo/pgdata, outside git), PRB_ADMIN_TOKEN (default
# local-admin-token). Then e.g. `node examples/shop/scripts/cohort.mjs --base http://localhost:3200`
# for traffic, or `node examples/shop/scripts/demo-record.mjs` for the demo video.
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

# 2. Builds: Core, the dashboard (static export served by Core), the Next SDK copied into the
# shop (its node_modules holds a copy, not a link), the shop.
cargo build -p pratibimba
BIN=$ROOT/target/debug/pratibimba
[ -f dashboard/out/index.html ] || (cd dashboard && npm run build)
(cd packages/next && npm run build >/dev/null)
rm -rf examples/shop/node_modules/@pratibimba/next/dist && cp -R packages/next/dist examples/shop/node_modules/@pratibimba/next/dist
(cd examples/shop && npx next build >/dev/null)

# 3. Site files: the shop's own, plus the hero swap slot its home page renders.
"$BIN" apply examples/shop/site.json
"$BIN" apply examples/shop/scripts/local-hero.json

# 4. Run.
pids=()
trap 'kill "${pids[@]}" 2>/dev/null' EXIT INT TERM
"$BIN" serve & pids+=($!)
"$BIN" worker & pids+=($!)
(cd examples/shop && PRB_CORE_URL=http://127.0.0.1:8080 PRB_TIMEOUT_MS=1000 PRB_PERSIST=1 exec npx next start -p 3200) & pids+=($!)
until curl -sf http://127.0.0.1:8080/health >/dev/null; do sleep 0.5; done
until curl -sf -o /dev/null http://localhost:3200/; do sleep 0.5; done
echo "local stack up: dashboard http://localhost:8080/app (token: $PRB_ADMIN_TOKEN), shop http://localhost:3200"
wait
