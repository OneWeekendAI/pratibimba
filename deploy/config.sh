# Shared settings for the Cloud Run deploy. No secrets here: they live in Secret Manager
# (prb-db = Supabase session-pooler URL, prb-admin-token).
PROJECT=instilplayv1
# Supabase is in ap-northeast-2 (Seoul). Cloud Run runs in asia-east1 (Taiwan), which has
# domain mappings. The image repo stays in Tokyo, where it was first made; Cloud Run pulls
# across regions fine.
REGION=asia-east1
REPO_REGION=asia-northeast1
REPO=$REPO_REGION-docker.pkg.dev/$PROJECT/pratibimba/core
SERVICE=pratibimba-core
JOB=pratibimba-worker
SCHEDULER=pratibimba-worker-every-minute
SITE_ORIGIN=https://pratibimba.meetdev.in
DOMAINS="pratibimbaapi.meetdev.in"
