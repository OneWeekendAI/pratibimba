#!/usr/bin/env bash
# Optional example: build locally (linux/amd64), push to Artifact Registry, and deploy Core as
# a Cloud Run service plus the learning worker as a Cloud Run job, both by digest, so what runs
# is exactly what was pushed. Settings come from the environment (see config.sh).
#   GCP_PROJECT=my-project REGION=us-central1 SITE_ORIGIN=https://shop.example.com deploy/cloud-run/deploy.sh
# To redeploy an image already pushed (no build): IMAGE=<repo>@sha256:... deploy/cloud-run/deploy.sh
# Needs: gcloud (authenticated), docker buildx, an Artifact Registry repo at $REPO, and the
# secrets named in config.sh created in Secret Manager.
set -euo pipefail
cd "$(dirname "$0")/../.."
. deploy/cloud-run/config.sh
if [ -z "${IMAGE:-}" ]; then
  TAG=$(git rev-parse --short HEAD)
  docker buildx build --platform linux/amd64 -t "$REPO:$TAG" --push .
  DIGEST=$(gcloud artifacts docker images describe "$REPO:$TAG" --format='value(image_summary.digest)')
  IMAGE="$REPO@$DIGEST"
fi
DIGEST=${IMAGE##*@}
echo "deploying $IMAGE"
# The edge snapshot publisher (core/src/edge.rs): on when a KV namespace is set.
EDGE_ENV=""; EDGE_SECRET=""
if [ -n "$EDGE_KV_NAMESPACE" ]; then
  EDGE_ENV=",PRB_EDGE_KV_ACCOUNT=${EDGE_KV_ACCOUNT:?EDGE_KV_ACCOUNT is required with EDGE_KV_NAMESPACE},PRB_EDGE_KV_NAMESPACE=$EDGE_KV_NAMESPACE"
  EDGE_SECRET=",PRB_EDGE_KV_TOKEN=$EDGE_KV_TOKEN_SECRET:latest"
fi
# EC2 edge decide: the Worker's shared token and its beacon URL, which then gets every snapshot too.
if [ -n "$EDGE_BEACON_URL" ]; then
  EDGE_ENV="$EDGE_ENV,PRB_EDGE_BEACON_URL=$EDGE_BEACON_URL"
  EDGE_SECRET="$EDGE_SECRET,PRB_EDGE_TOKEN=$EDGE_TOKEN_SECRET:latest"
fi

# The API (also /mcp, /sdk.js, /manifest, /app). Scales to zero.
gcloud run deploy "$SERVICE" --project "$PROJECT" --region "$REGION" --image "$IMAGE" \
  --args serve --port 8080 --allow-unauthenticated \
  --min-instances 0 --max-instances 2 --cpu 1 --memory 512Mi --concurrency 80 \
  --set-env-vars "PRB_SITE_ORIGIN=$SITE_ORIGIN,PRB_DB_MAX_CONNECTIONS=4,RUST_LOG=info$EDGE_ENV" \
  --set-secrets "PRB_DB=$DB_SECRET:latest,PRB_ADMIN_TOKEN=$ADMIN_TOKEN_SECRET:latest$EDGE_SECRET"

# The learning worker: one pass per execution (`worker --once`), triggered by schedule.sh.
gcloud run jobs deploy "$JOB" --project "$PROJECT" --region "$REGION" --image "$IMAGE" \
  --args worker,--once --tasks 1 --max-retries 0 --task-timeout 300s --cpu 1 --memory 512Mi \
  --set-env-vars "PRB_DB_MAX_CONNECTIONS=2,RUST_LOG=info$EDGE_ENV" \
  --set-secrets "PRB_DB=$DB_SECRET:latest$EDGE_SECRET"

# Check: both run the pushed digest.
for got in "$(gcloud run services describe "$SERVICE" --project "$PROJECT" --region "$REGION" --format='value(spec.template.spec.containers[0].image)')" \
           "$(gcloud run jobs describe "$JOB" --project "$PROJECT" --region "$REGION" --format='value(spec.template.spec.template.spec.containers[0].image)')"; do
  [ "$got" = "$IMAGE" ] || { echo "MISMATCH: deployed $got, pushed $IMAGE" >&2; exit 1; }
done
echo "service and job run $DIGEST"
