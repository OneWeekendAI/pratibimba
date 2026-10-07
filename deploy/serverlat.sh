#!/usr/bin/env bash
# Server-side /decide latency from Cloud Run request logs (httpRequest.latency), since a
# given UTC timestamp. Usage: deploy/serverlat.sh <region> <since-RFC3339> [path]
set -euo pipefail
. "$(dirname "$0")/config.sh"
R=${1:?region}; SINCE=${2:?since}; P=${3:-/decide}
gcloud logging read "resource.type=cloud_run_revision AND resource.labels.service_name=$SERVICE AND resource.labels.location=$R AND httpRequest.requestUrl:\"$P\" AND timestamp>=\"$SINCE\"" \
  --project "$PROJECT" --limit 500 --format='value(httpRequest.latency)' \
  | sed 's/s$//' | sort -n | awk '{a[NR]=$1} END {if (NR==0) {print "no log lines"; exit} p50=a[int((NR+1)/2)]; p95=a[int(NR*0.95+0.999)]; printf "server n=%d p50=%.0fms p95=%.0fms min=%.0fms max=%.0fms\n", NR, p50*1000, p95*1000, a[1]*1000, a[NR]*1000}'
