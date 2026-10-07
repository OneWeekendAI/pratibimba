# Shared settings for the Cloud Run deploy. No secrets here: they live in Secret Manager
# (prb-db = Supabase session-pooler URL, prb-admin-token).
PROJECT=instilplayv1
# Supabase is in ap-northeast-2 (Seoul). asia-northeast3 has no Cloud Run domain mappings,
# so Tokyo is the closest region that does.
REGION=asia-northeast1
REPO=$REGION-docker.pkg.dev/$PROJECT/pratibimba/core
SERVICE=pratibimba-core
JOB=pratibimba-worker
SCHEDULER=pratibimba-worker-every-minute
SITE_ORIGIN=https://pratibimba.meetdev.in
DOMAINS="pratibimbaapi.meetdev.in pratibimbamcp.meetdev.in"
