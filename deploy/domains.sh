#!/usr/bin/env bash
# One-time: map the API hostnames to the service and add DNS-only CNAMEs in Cloudflare.
# Needs CLOUDFLARE_API_TOKEN with Zone:DNS:Edit on meetdev.in. Never touches other records.
# Cloud Run domain mappings exist only in some regions (asia-east1 yes, asia-northeast3 no), so
# this runs with REGION=asia-east1. For Seoul the hostname goes through deploy/proxy-worker/.
set -euo pipefail
REGION=${REGION:-asia-east1}
. "$(dirname "$0")/config.sh"
ZONE=$(curl -fsS -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN" "https://api.cloudflare.com/client/v4/zones?name=meetdev.in" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"][0]["id"])')
for d in $DOMAINS; do
  gcloud beta run domain-mappings describe --domain "$d" --project "$PROJECT" --region "$REGION" >/dev/null 2>&1 \
    || gcloud beta run domain-mappings create --service "$SERVICE" --domain "$d" --project "$PROJECT" --region "$REGION"
  existing=$(curl -fsS -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN" "https://api.cloudflare.com/client/v4/zones/$ZONE/dns_records?name=$d" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["result"]))')
  if [ "$existing" = 0 ]; then
    curl -fsS -X POST -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN" -H "Content-Type: application/json" \
      "https://api.cloudflare.com/client/v4/zones/$ZONE/dns_records" \
      --data "{\"type\":\"CNAME\",\"name\":\"$d\",\"content\":\"ghs.googlehosted.com\",\"proxied\":false,\"ttl\":300}" >/dev/null
    echo "added CNAME $d -> ghs.googlehosted.com (DNS only)"
  else
    echo "$d already has a DNS record; left alone"
  fi
done
echo "Certificate status: gcloud beta run domain-mappings describe --domain <d> --project $PROJECT --region $REGION --format='value(status.conditions)'"
