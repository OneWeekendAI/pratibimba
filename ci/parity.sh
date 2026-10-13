#!/usr/bin/env bash
# EC5 (edge-cache.md, "Self-hoster parity test"): every suite, run with the edge disabled, the way
# a self-hoster runs Pratibimba: Core alone, SDKs talking straight to it. No Worker, no KV, no
# beacon, no queue. Exits non-zero at the first stage that fails.
#
#   ci/parity.sh                 all stages
#   PARITY_STAGES="direct cargo" ci/parity.sh   only those (build always runs)
#
# Stages: direct (the binary end to end, ci/direct.mjs), cargo (the whole Core suite), units
# (the client, agent, next and edge SDK packages), e2e (next, edge and agent e2e, the dashboard
# and examples/next through Playwright).
#
# Needs: Postgres where the user may create databases (DATABASE_URL, default
# postgres://postgres@localhost:55440/postgres), cargo, node, psql, and `npm ci` done in each
# package (.github/workflows/parity.yml shows the setup).
set -euo pipefail
cd "$(dirname "$0")/.."

# The point of the run: no edge configuration may reach Core, whatever this shell exports.
for v in $(env | sed -n 's/^\(PRB_EDGE_[A-Z_]*\)=.*/\1/p'); do
  echo "parity: unsetting $v (the direct path runs with no edge)"
  unset "$v"
done
export DATABASE_URL=${DATABASE_URL:-postgres://postgres@localhost:55440/postgres}
export PRB_E2E_ADMIN_DB=${PRB_E2E_ADMIN_DB:-$DATABASE_URL}
STAGES=${PARITY_STAGES:-direct cargo units e2e}
want() { [[ " $STAGES " == *" $1 "* ]]; }
stage() { echo; echo "== parity: $1"; }

stage build
cargo build -p pratibimba

if want direct; then
  stage "direct (the binary, no edge)"
  node ci/direct.mjs
fi

if want cargo; then
  stage "cargo test (Core)"
  cargo test -p pratibimba
fi

if want units; then
  for p in packages/client packages/agent packages/next packages/edge; do
    stage "unit tests: $p"
    (cd "$p" && npm test)
  done
fi

if want e2e; then
  for p in packages/next packages/edge packages/agent; do
    stage "e2e: $p"
    (cd "$p" && npm run e2e)
  done
  stage "e2e: dashboard"
  (cd dashboard && npm run build && npx playwright test)
  stage "e2e: examples/next"
  (cd examples/next && npm run build && npx playwright test)
fi

echo
echo "parity: all stages passed ($STAGES) with no edge configured"
