#!/usr/bin/env bash
# EC2's switch: whether the proxy Worker answers /decide itself. Usage: edge-decide.sh on | off
#   on   the Worker decides from the site's snapshot when it can (falls back to Core otherwise)
#   off  every /decide goes to Core, as in EC1 (the snapshot is still read, for its age header)
# Redeploys the Worker with EDGE_DECIDE set; takes effect as the new version rolls out (seconds).
set -euo pipefail
cd "$(dirname "$0")"
case "${1:-}" in
  on|off) wrangler deploy --var "EDGE_DECIDE:$1" ;;
  *) echo "usage: $0 on|off" >&2; exit 2 ;;
esac
