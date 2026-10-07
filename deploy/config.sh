# Shared settings for the Cloud Run deploy. No secrets here: they live in Secret Manager
# (prb-db = Supabase session-pooler URL, prb-admin-token).
PROJECT=instilplayv1
# Supabase is in ap-northeast-2 (Seoul). REGION picks where Cloud Run runs; every script
# reads it, so `REGION=asia-east1 deploy/deploy.sh` targets Taiwan. The default is
# asia-northeast3 (Seoul), next to the database: /decide makes ~17 sequential queries, so the
# Core<->DB round trip dominates. Seoul has no Cloud Run domain mappings, so the public
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
