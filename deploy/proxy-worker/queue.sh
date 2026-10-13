#!/usr/bin/env bash
# EC3's switch: whether /e and the edge's decision records go through the events queue.
# Usage: queue.sh on | off
#   on   /e is answered 202 at the edge and queued; the consumer batches it to Core's /e/batch
#   off  /e and decision records go to Core directly, as before EC3 (the consumer keeps
#        draining whatever is already queued)
# Redeploys the Worker with QUEUE_WRITES set; takes effect as the new version rolls out.
set -euo pipefail
cd "$(dirname "$0")"
case "${1:-}" in
  on|off) wrangler deploy --var "QUEUE_WRITES:$1" ;;
  *) echo "usage: $0 on|off" >&2; exit 2 ;;
esac
