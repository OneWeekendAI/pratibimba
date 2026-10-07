#!/usr/bin/env bash
# One-time: Cloud Scheduler runs the worker job every minute. Idempotent.
set -euo pipefail
. "$(dirname "$0")/config.sh"
PN=$(gcloud projects describe "$PROJECT" --format='value(projectNumber)')
SA=$PN-compute@developer.gserviceaccount.com
gcloud run jobs add-iam-policy-binding "$JOB" --project "$PROJECT" --region "$REGION" \
  --member "serviceAccount:$SA" --role roles/run.invoker >/dev/null
URI="https://run.googleapis.com/v2/projects/$PROJECT/locations/$REGION/jobs/$JOB:run"
verb=create; gcloud scheduler jobs describe "$SCHEDULER" --project "$PROJECT" --location "$REGION" >/dev/null 2>&1 && verb=update
gcloud scheduler jobs $verb http "$SCHEDULER" --project "$PROJECT" --location "$REGION" \
  --schedule "* * * * *" --http-method POST --uri "$URI" --oauth-service-account-email "$SA"
