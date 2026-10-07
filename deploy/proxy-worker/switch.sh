#!/usr/bin/env bash
# Moves pratibimbaapi.meetdev.in between Core in Seoul (via this Worker) and Core in Taiwan
# (the Cloud Run domain mapping). Usage: switch.sh on | off | status
#   on     add the Worker route pratibimbaapi.meetdev.in/* -> pratibimba-api-proxy (Seoul)
#   off    delete that route: traffic falls straight back to the proxied CNAME
#          ghs.googlehosted.com -> Taiwan's domain mapping (pratibimba-core, asia-east1)
# Neither touches DNS or the domain mapping. Needs CLOUDFLARE_API_TOKEN with Workers Routes:Edit.
set -euo pipefail
HOST=pratibimbaapi.meetdev.in; PATTERN="$HOST/*"; SCRIPT=pratibimba-api-proxy
API=https://api.cloudflare.com/client/v4
auth=(-H "Authorization: Bearer $CLOUDFLARE_API_TOKEN")
ZONE=$(curl -fsS "${auth[@]}" "$API/zones?name=meetdev.in" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"][0]["id"])')
route_id() { curl -fsS "${auth[@]}" "$API/zones/$ZONE/workers/routes" | python3 -c "import json,sys; print(next((r['id'] for r in json.load(sys.stdin)['result'] if r['pattern']=='$PATTERN'), ''))"; }
case "${1:-status}" in
  on)  [ -n "$(route_id)" ] && { echo "route already on"; exit 0; }
       curl -fsS -X POST "${auth[@]}" -H 'Content-Type: application/json' "$API/zones/$ZONE/workers/routes" \
         --data "{\"pattern\":\"$PATTERN\",\"script\":\"$SCRIPT\"}" >/dev/null; echo "on: $PATTERN -> $SCRIPT (Seoul)";;
  off) id=$(route_id); [ -z "$id" ] && { echo "route already off"; exit 0; }
       curl -fsS -X DELETE "${auth[@]}" "$API/zones/$ZONE/workers/routes/$id" >/dev/null; echo "off: $HOST -> domain mapping (Taiwan)";;
  status) [ -n "$(route_id)" ] && echo "on (Seoul via Worker)" || echo "off (Taiwan via domain mapping)";;
  *) echo "usage: $0 on|off|status" >&2; exit 2;;
esac
