# Settings for the optional Cloud Run example. Everything comes from the environment, so
# nothing here names a real project; set the required ones before running deploy.sh:
#   export GCP_PROJECT=my-project REGION=us-central1 SITE_ORIGIN=https://shop.example.com
# Secrets are never put here. They are read from Secret Manager by name (see deploy.sh):
#   $DB_SECRET           Postgres URL for Core (PRB_DB)
#   $ADMIN_TOKEN_SECRET  the admin token (PRB_ADMIN_TOKEN)
# Pick REGION close to your database: /decide makes four sequential round trips to Postgres,
# so the Core<->DB distance dominates its latency.
PROJECT=${GCP_PROJECT:?set GCP_PROJECT to your Google Cloud project id}
REGION=${REGION:?set REGION, e.g. us-central1}
REPO_REGION=${REPO_REGION:-$REGION}
REPO=${REPO:-$REPO_REGION-docker.pkg.dev/$PROJECT/pratibimba/core}
SERVICE=${SERVICE:-pratibimba-core}
JOB=${JOB:-pratibimba-worker}
SCHEDULER=${SCHEDULER:-pratibimba-worker-every-minute}
SITE_ORIGIN=${SITE_ORIGIN:?set SITE_ORIGIN to the site that embeds Pratibimba, e.g. https://shop.example.com}
DB_SECRET=${DB_SECRET:-prb-db}
ADMIN_TOKEN_SECRET=${ADMIN_TOKEN_SECRET:-prb-admin-token}
# Optional, the edge (README "Edge"): leave these unset to deploy Core with the edge off.
# EC1 snapshots: Core writes each site's snapshot to this Workers KV namespace, with a
# Cloudflare API token read from Secret Manager ($EDGE_KV_TOKEN_SECRET).
EDGE_KV_ACCOUNT=${EDGE_KV_ACCOUNT:-}
EDGE_KV_NAMESPACE=${EDGE_KV_NAMESPACE:-}
EDGE_KV_TOKEN_SECRET=${EDGE_KV_TOKEN_SECRET:-prb-edge-kv-token}
# EC2 edge decide: Core pushes every snapshot to the Worker's beacon at this URL (e.g.
# https://api.example.com/_prb/edge/snapshot) and accepts the Worker's decisions with the
# shared token in Secret Manager ($EDGE_TOKEN_SECRET, same value as the Worker's EDGE_TOKEN).
EDGE_BEACON_URL=${EDGE_BEACON_URL:-}
EDGE_TOKEN_SECRET=${EDGE_TOKEN_SECRET:-prb-edge-token}
