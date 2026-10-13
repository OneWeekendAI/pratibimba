#!/usr/bin/env bash
# Build locally (linux/amd64), push to Artifact Registry, and deploy the service and the job
# by digest, so what runs is exactly what was pushed. Usage: deploy/deploy.sh
# To redeploy an image already pushed (no build): IMAGE=<repo>@sha256:... deploy/deploy.sh
# REGION picks the region (default asia-northeast3, see config.sh). While Taiwan (asia-east1)
# is the rollback target, deploy the same digest there too, or a rollback serves an old build:
#   REGION=asia-east1 IMAGE=<the digest printed below> deploy/deploy.sh
set -euo pipefail
cd "$(dirname "$0")/.."
. deploy/config.sh
if [ -z "${IMAGE:-}" ]; then
  TAG=$(git rev-parse --short HEAD)
  docker buildx build --platform linux/amd64 -t "$REPO:$TAG" --push .
  DIGEST=$(gcloud artifacts docker images describe "$REPO:$TAG" --format='value(image_summary.digest)')
  IMAGE="$REPO@$DIGEST"
fi
DIGEST=${IMAGE##*@}
echo "deploying $IMAGE"
# The edge snapshot publisher (core/src/edge.rs): on when config.sh names a KV namespace.
EDGE_ENV=""; EDGE_SECRET=""
if [ -n "${EDGE_KV_NAMESPACE:-}" ]; then
  EDGE_ENV=",PRB_EDGE_KV_ACCOUNT=$EDGE_KV_ACCOUNT,PRB_EDGE_KV_NAMESPACE=$EDGE_KV_NAMESPACE"
  EDGE_SECRET=",PRB_EDGE_KV_TOKEN=prb-edge-kv-token:latest"
fi
# EC2 edge decide: the Worker's shared token (Secret Manager prb-edge-token, the same value as the
# Worker's EDGE_TOKEN secret) and its beacon URL, which then gets every snapshot too.
if [ -n "${EDGE_BEACON_URL:-}" ]; then
  EDGE_ENV="$EDGE_ENV,PRB_EDGE_BEACON_URL=$EDGE_BEACON_URL"
  EDGE_SECRET="$EDGE_SECRET,PRB_EDGE_TOKEN=prb-edge-token:latest"
fi

# The API (also /mcp, /sdk.js, /manifest, /app). Scales to zero.
gcloud run deploy "$SERVICE" --project "$PROJECT" --region "$REGION" --image "$IMAGE" \
  --args serve --port 8080 --allow-unauthenticated \
  --min-instances 0 --max-instances 2 --cpu 1 --memory 512Mi --concurrency 80 \
  --set-env-vars "PRB_SITE_ORIGIN=$SITE_ORIGIN,PRB_DB_MAX_CONNECTIONS=4,RUST_LOG=info$EDGE_ENV" \
  --set-secrets "PRB_DB=prb-db:latest,PRB_ADMIN_TOKEN=prb-admin-token:latest$EDGE_SECRET"

# The learning worker: one pass per execution (`worker --once`), triggered by Cloud Scheduler.
gcloud run jobs deploy "$JOB" --project "$PROJECT" --region "$REGION" --image "$IMAGE" \
  --args worker,--once --tasks 1 --max-retries 0 --task-timeout 300s --cpu 1 --memory 512Mi \
  --set-env-vars "PRB_DB_MAX_CONNECTIONS=2,RUST_LOG=info$EDGE_ENV" \
  --set-secrets "PRB_DB=prb-db:latest$EDGE_SECRET"

# Check: both run the pushed digest.
for got in "$(gcloud run services describe "$SERVICE" --project "$PROJECT" --region "$REGION" --format='value(spec.template.spec.containers[0].image)')" \
           "$(gcloud run jobs describe "$JOB" --project "$PROJECT" --region "$REGION" --format='value(spec.template.spec.template.spec.containers[0].image)')"; do
  [ "$got" = "$IMAGE" ] || { echo "MISMATCH: deployed $got, pushed $IMAGE" >&2; exit 1; }
done
echo "service and job run $DIGEST"
