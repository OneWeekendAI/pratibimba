#!/usr/bin/env bash
# Latency bench for /decide: N sequential calls (fresh subject each), prints client-side
# p50/p95/max of curl time_total. Server-side time comes from Cloud Run's request logs
# (httpRequest.latency); see deploy/serverlat.sh. Usage: deploy/bench.sh <base-url> [n] [tag]
# Subjects are "latbench-<tag>-<i>", so the rows these calls write are easy to find.
set -euo pipefail
BASE=${1:?base url}; N=${2:-25}; TAG=${3:-$(date +%s)}
for i in $(seq 1 "$N"); do
  curl -sS -o /dev/null -w '%{http_code} %{time_total}\n' -X POST -H 'content-type: application/json' \
    "$BASE/decide" -d "{\"site\":\"shop\",\"slot\":\"hero\",\"subject\":\"latbench-$TAG-$i\",\"ctx\":{}}"
done | awk '$1 != 200 {bad++} {print $2} END {if (bad) print "NON-200: " bad > "/dev/stderr"}' | sort -n \
  | awk '{a[NR]=$1} END {p50=a[int((NR+1)/2)]; p95=a[int(NR*0.95+0.999)]; printf "client n=%d p50=%.0fms p95=%.0fms min=%.0fms max=%.0fms\n", NR, p50*1000, p95*1000, a[1]*1000, a[NR]*1000}'
