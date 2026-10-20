# deploy/ (optional)

Nothing here is needed to run Pratibimba. The supported self-hosted install is the Docker
image with Postgres: see the README's Quickstart and `examples/compose.yml`. This folder holds
optional pieces for running Core on managed infrastructure and in front of a CDN.

| Path | What it is |
|---|---|
| `proxy-worker/` | The edge Worker (README "Edge"): edge snapshot read, edge decide, `/e` queue, tombstones. Its tests run with `npm test` and need no Cloudflare account. Before `wrangler deploy`, replace the placeholders in `wrangler.jsonc` (`ORIGIN`, the KV namespace id) and set the `EDGE_TOKEN` secret. `edge-decide.sh` and `queue.sh` flip its two switches. |
| `cloud-run/` | An example of running Core as a Cloud Run service plus the worker as a Cloud Run job on a one-minute Cloud Scheduler trigger. All settings come from environment variables (`config.sh` lists them); secrets are read from Secret Manager by name. |
| `checks/` | Scripts that measure a running deployment you point them at: `/decide` latency as an SDK sees it (`bench-node.mjs`), how fast the kill switch reaches the edge (`edge-killswitch.mjs`), and how fast an erase stops the edge serving a subject (`edge-erase.mjs`). |

Core reads the edge settings from `PRB_EDGE_KV_ACCOUNT`, `PRB_EDGE_KV_NAMESPACE`,
`PRB_EDGE_KV_TOKEN`, `PRB_EDGE_BEACON_URL` and `PRB_EDGE_TOKEN`; leave them unset and the edge
stays off.
