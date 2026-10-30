#!/usr/bin/env bash
# L3: the local demo matrix. One command brings up the local stack (Core, the worker, the proxy
# Worker under `wrangler dev`, a fake KV for it) and examples/demo-site, exercises slot kinds x
# audiences x edge decide x write queue x consent x erase in real Chromium, prints a PASS/FAIL/SKIP
# table, and tears down everything it started. Nothing is hosted; nothing outside this repo is touched.
#
#   ci/demo-matrix.sh                 all modes
#   ci/demo-matrix.sh --record        also record Playwright video to target/demo-matrix/videos
#   ci/demo-matrix.sh --prove         then break one thing per category and show its row go red
#   ci/demo-matrix.sh --mutate=erase  only that mutation (exits non-zero by design)
#   ci/demo-matrix.sh --modes=direct,w-on-on
#
# Env: PRB_MATRIX_ADMIN_DB (a Postgres URL where this user may create databases; default
# postgres://$USER@localhost:5432/postgres if it answers, else the Postgres ci/local-stack.sh uses
# on :55440, started here), PRB_MATRIX_PORT (base port, default 18180; uses base..base+9).
# Needs: cargo, node, psql, wrangler (the Worker modes are SKIP without it), Playwright's Chromium.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
for v in $(env | sed -n 's/^\(PRB_[A-Z_]*\)=.*/\1/p'); do unset "$v"; done # the run configures its own Core
if [ -z "${PRB_MATRIX_ADMIN_DB:-}" ]; then
  if pg_isready -q -h localhost -p 5432; then
    PRB_MATRIX_ADMIN_DB=postgres://$(id -un)@localhost:5432/postgres
  else
    # Same Postgres as ci/local-stack.sh (trust auth, superuser postgres, :55440).
    PG_BIN=${PG_BIN:-/opt/homebrew/opt/postgresql@17/bin}
    PGDATA=${PRB_PGDATA:-$ROOT/../pratibimba-demo/pgdata}
    if ! "$PG_BIN/pg_isready" -q -h localhost -p 55440; then
      [ -f "$PGDATA/PG_VERSION" ] || { mkdir -p "$PGDATA"; "$PG_BIN/initdb" -D "$PGDATA" -U postgres --auth=trust -E UTF8 >/dev/null; }
      "$PG_BIN/pg_ctl" -D "$PGDATA" -l "$PGDATA/../pg.log" -o "-p 55440 -k /tmp -c listen_addresses=localhost -c max_connections=300" start
    fi
    PRB_MATRIX_ADMIN_DB=postgres://postgres@localhost:55440/postgres
  fi
fi
export PRB_MATRIX_ADMIN_DB

cargo build -p pratibimba 2>&1 | tail -n 2
[ -f dashboard/out/index.html ] || (cd dashboard && npm run build)
exec node ci/demo-matrix.mjs "$@"
