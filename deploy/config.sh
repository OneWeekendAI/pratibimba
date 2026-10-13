# Shared settings for the Cloud Run deploy. No secrets here: they live in Secret Manager
# (prb-db = Supabase session-pooler URL, prb-admin-token).
PROJECT=instilplayv1
# Supabase is in ap-northeast-2 (Seoul). REGION picks where Cloud Run runs; every script
# reads it, so `REGION=asia-east1 deploy/deploy.sh` targets Taiwan. The default is
# asia-northeast3 (Seoul), next to the database: /decide makes four sequential round trips
# (it made ~17 before 2026-10-08), so the Core<->DB round trip still dominates. Seoul has no Cloud Run domain mappings, so the public
# hostname reaches it through the Cloudflare Worker in deploy/proxy-worker/. Taiwan
# (asia-east1) is the old deploy and keeps the domain mapping (deploy/domains.sh) as the
# rollback path. The image repo stays in Tokyo; Cloud Run pulls across regions fine.
REGION=${REGION:-asia-northeast3}
REPO_REGION=asia-northeast1
REPO=$REPO_REGION-docker.pkg.dev/$PROJECT/pratibimba/core
SERVICE=pratibimba-core
JOB=pratibimba-worker
SCHEDULER=pratibimba-worker-every-minute
SITE_ORIGIN=https://pratibimba.meetdev.in
DOMAINS="pratibimbaapi.meetdev.in"
# EC1 edge snapshots (edge-cache.md): Core pushes each site's snapshot to this Workers KV
# namespace; the proxy Worker reads it (binding SNAPSHOTS). Not secrets. The API token is in
# Secret Manager as prb-edge-kv-token. Leave these empty to deploy Core with the edge off.
EDGE_KV_ACCOUNT=f35116f6863fad577cefb7900b50d073
EDGE_KV_NAMESPACE=b30829cd168e42768f85c882fca47ac0
# EC2 edge decide: Core pushes every snapshot to the Worker's beacon here too, and accepts the
# Worker's decisions (POST /edge/decisions) with the token in Secret Manager prb-edge-token.
# Leave empty to deploy Core without edge decide (the Worker then always falls back).
EDGE_BEACON_URL=https://pratibimbaapi.meetdev.in/_prb/edge/snapshot
