# Pratibimba

[![licence: MIT](https://img.shields.io/badge/licence-MIT-black.svg)](LICENSE)
[![core: Rust](https://img.shields.io/badge/core-Rust-dea584.svg)](core)
[![SDKs: TypeScript](https://img.shields.io/badge/SDKs-TypeScript-3178c6.svg)](packages)

Open-source, self-hostable website personalisation for developers. Mark the regions of your
page that may change (slots), declare a few versions of each (variants), and Pratibimba picks
the version each visitor is most likely to act on. It decides **before first paint**, learns
**per slot** from what visitors actually do, keeps every decision **explainable and
replayable**, holds every variant behind a **human approval gate**, and lets your own coding
agent drive the whole thing over **MCP**. One Rust binary on Postgres. MIT. An open
alternative to commercial personalisation tools.

*Pratibimba* (प्रतिबिम्ब, Sanskrit) means reflection: the page takes the shape of whoever is
looking at it.

> **Status.** Phase 1 (developer-first) and Phase 2 (agent-first) are in. The core, three
> installs, dashboard, approval gate, MCP server, agent instruction pack, signed webhooks,
> edge snapshot, cookieless + consent-aware modes, right-to-erase, pluggable attribution,
> fixed-allocation experiments, sorted and copy slots, visitor history, and commerce events
> are all live and tested. Expect breaking changes until 1.0.

## How it works

```mermaid
flowchart LR
  subgraph Visitor["Browser"]
    page["your page"]
  end
  subgraph FirstPaint["Decide before first paint"]
    next["@pratibimba/next<br/>(SSR / proxy)"]
    edge["@pratibimba/edge<br/>(Cloudflare / Deno / Vercel)"]
    tag["script tag"]
  end
  subgraph Platform["Self-hosted"]
    core["pratibimba serve<br/>/decide /e /manifest /mcp /app"]
    worker["pratibimba worker<br/>closes windows, learns, rolls back"]
    db[("Postgres")]
    kv[["Workers KV<br/>(edge snapshot, optional)"]]
    queue["Cloudflare Queue<br/>(edge /e buffer, optional)"]
  end
  subgraph Humans["Humans and agents"]
    dash["dashboard /app<br/>(approval gate)"]
    mcp["your coding agent<br/>over MCP"]
  end

  page --> next & edge & tag
  next --> core
  edge -- "snapshot read" --> kv
  edge -- "snapshot miss" --> core
  edge -- "events" --> queue --> core
  tag --> core
  core <--> db
  worker <--> db
  core -- "snapshot push" --> kv
  dash --> core
  mcp --> core
```

- **Core** is one Rust binary with two jobs. `pratibimba serve` answers `/decide`, `/e`,
  `/manifest/:site`, `/mcp`, the `/api` dashboard API and serves `/app`. `pratibimba worker`
  closes attribution windows, credits conversions, updates the models, publishes the edge
  snapshot and runs auto-rollback. Both apply migrations on start, so they can never disagree
  about schema or policy.
- **Decisions** come from a **LinTS bandit per slot**: Thompson sampling over a Bayesian
  logistic regression, one per variant. Features are the visitor's `returning`, `mobile`,
  `from_ad` plus a bias; when the visitor's time zone is known, the local clock
  (`sin_hour`, `cos_hour`, `is_night_local`, `is_weekend_local`, `is_business_hours_local`);
  any declared `feature:*` fields; and the visitor's history (orders, LTV bucket, recency).
  No LLM sits on the decision path.
- **Learning** happens once per decision, and only when the visitor actually saw it. A goal
  event (for example `click`) inside the attribution window counts as a conversion; an
  exposed decision whose window closes without one counts as a miss. A decision that was
  never shown teaches nothing.
- **A holdout keeps it honest.** Each site sends a fixed share of visitors (default 10%,
  at most 50%) the baseline regardless. Lift is always the policy against that holdout, with
  a 95% confidence sequence that stays valid however often you look.
- **Auto-rollback.** When the top of a variant's confidence sequence sits below the
  holdout's by more than the site's margin, the worker pauses it and restores the slot's
  model to the version current when the variant was approved.
- **Only approved variants are ever served.** The approval gate is unconditional, including
  for agent-written drafts; variants in the site file you apply are approved by you.
- **Every decision is replayable.** Each decision is stored with its features, every
  variant's sampled score, the policy version and the random seed, so the dashboard can show
  it back and the simulator can rerun it.

## Features

### Decisioning

- **Per-slot LinTS bandit** with a holdout and auto-rollback.
- **Three slot kinds.** `swap` picks one variant (any HTML: a new layout, an image beside the
  headline, a different banner); `sorted` learns a per-item order with cascade credit (misses
  above the acted item, reward to it, nothing below); `copy` renders typed fields: plain text
  with length limits, and images (an uploaded asset or URL, with required alt text).
- **Images are first-class.** Core stores uploads per site, content-addressed, served with a
  one-year immutable cache and a policy that cannot run script; SVGs with script are refused.
  Local directory or any S3-compatible bucket. Images no live variant uses are cleaned up.
- **Explain and replay.** Each decision's record names the arm, every variant's sampled
  score, the policy version, the audiences evaluated and whether the visitor matched.
- **Fixed-allocation experiments.** Lay a weighted A/B/n over one slot's approved variants,
  with anytime-valid confidence sequences. Bandit learns from the arm the hash drops the
  visitor into, so experiment traffic is not wasted.
- **Pluggable attribution.** `last_touch` (default) or `linear`; a slot overrides the site.
- **Audiences gate variants.** A promoted segment can restrict a variant (never the
  baseline). Audiences read booleans, local clock windows and visitor history. Gate fails
  closed: archived or clock-unknowable audiences match nobody.
- **Cold start.** Equal exposure until a slot has `cold_start_exposures` learned-from
  exposures.

### Learning

- **Learn-at-close.** The worker learns from each decision at window close, not from clicks
  as they arrive, so late conversions and late platform webhooks are counted exactly and
  **feedback is unbiased** (clicked-only training would overweight short attribution paths).
- **Late corrections.** A conversion webhook arriving after a decision has settled flips the
  decision to converted, updates the model exactly (`+w·x` on log-likelihood, covariance
  unchanged) and refires the `conversion` webhook with `late: true`.
- **Model versioning and manual rollback.** Every learning pass bumps a version; the
  dashboard shows versions and can roll a slot's model back by hand.
- **Simulator with time-travel.** Replay a slot's history against a copy of its models and
  running experiments; stop or restart on the sandbox clock.

### Install surfaces

- `@pratibimba/next` — Next.js proxy and server component (`<prb.Slot>`, `<prb.Sorted>`,
  `<prb.Copy>`). Decides during the server render; the first byte already carries the variant.
- `@pratibimba/edge` — one handler for Cloudflare Workers, Deno Deploy and Vercel Edge.
  Rewrites `<!-- prb:slot="hero" -->…<!-- /prb:slot -->` regions as the HTML streams.
- Script tag — a self-contained fallback that hides only the slots, decides, swaps and
  reveals within a hard timeout. For any site, including static HTML.

### Privacy

- **Three modes per site:** `full`, `cookieless` (daily-rotated salt, no `returning`,
  salts never backed up), `consent_aware` (default; `full` when consent is present,
  `cookieless` otherwise).
- **Consent sources.** `_prb_consent`, IAB TCF v2 `euconsent-v2` (purpose 1), OneTrust
  `OptanonConsent` (C0002), or a user-supplied function.
- **PII features opt-in.** A declared `feature:*` tagged `pii: true` is only sent and used
  for an identified visitor; dropped at the SDK and dropped again in Core.
- **Minimum collection.** `fields_allowed` whitelists which optional ctx fields are stored;
  anything else is dropped at `/decide` and `/e` before writing.
- **Right to erase.** `pratibimba erase <site> <subject>` or the admin API hides the
  subject's events on the spot, schedules deletion, writes a tombstone (later requests get
  `410 Gone`), and tombstones the subject in the edge snapshot's Bloom filter so a CDN read
  cannot resurrect them. Audit log included.
- **No demographic inference.** The decision context is a closed set of named fields;
  sending `gender`, `age` or any other undeclared field returns 4xx.

### Edge

- **Edge snapshot.** Core writes one JSON per site to Cloudflare Workers KV with everything
  `/decide` reads that is the same for every visitor (approved arms, model state and
  version, cold-start counts, audiences, the running experiment, holdout, kill switch,
  manifest, installed plugins, erased-subject Bloom filter).
- **Edge decide.** `deploy/proxy-worker` reads the snapshot and decides at the PoP for
  consented visitors; a cookieless or copy or sorted request falls through to Core.
- **Edge /e queue.** Events batch through a Cloudflare Queue to Core's `/e/batch`; `/e` is
  non-blocking at the edge.
- **Edge tombstones + parity CI.** Erases cross-publish to KV so the edge honours them within
  one worker pass; `ci/parity.sh` compares every edge decision against Core on the same
  inputs.

### Agent and MCP

- **Open MCP server at `/mcp`** behind a per-site `prb_mcp_…` token. The developer's own
  coding agent (Claude Code, Cursor, …) reads results, drafts, slot definitions, audiences,
  commerce facts and visitor history; proposes slots, variants, copy, sorted items,
  audiences and plugin installs; and submits drafts for human approval.
- **Instruction pack.** `npm i -g @pratibimba/agent` ships a guide the agent reads first;
  `npx @pratibimba/agent install` adds the pack to a project, `npx @pratibimba/agent scan`
  reads JSX/TSX, plain HTML, Vue, Svelte and Astro and proposes candidate slots with
  file:line, selector, kind (swap/copy/sorted) and the exact `create_slot` arguments; a hero or
  banner with a picture comes with an image field. `upload_asset` lets the agent upload an
  image it was given; the variant that uses it still needs a person's approval.

### Dashboard

- Live stream per site, slot results with lift and anytime-valid CIs, approval queue,
  decisions with explain + replay + counterfactual, model versions with manual rollback,
  draft comparison and comments, install wizard that probes your real site, visitor insights
  (heatmaps, retention cohorts, journeys, discovered segments), audiences and experiments.
- All screens are driven by rollups only; a test renames the raw tables and calls every
  insight route.

### Explainable and auditable

- Every decision stores the inputs it saw and the dice it rolled.
- Every variant change writes an approvals row with actor, timestamp, planned traffic share
  and comment.
- **Signed webhooks out** for `decision`, `exposure`, `conversion`, `variant.approved`,
  `variant.retired`, `variant.share_changed` and `auto_rollback`. Outbox-persisted, retried
  with backoff up to 8 attempts, HMAC-SHA256 signed, idempotency keys carried through retries.

## Engineering rigour

The project is built the way the problem demands: numbers are measured, not asserted, and the
work is held behind suites and mutation tests.

**Test suites, current counts (`cargo test`, `npm test`):**

| Suite | Pass count | Covers |
|---|---|---|
| `cargo test` (Core) | **1000 / 0** | bandit math, HTTP, worker, migrations, cohort sims, replay, erase, edge snapshot, parity, webhooks, assets |
| `dashboard` Playwright e2e | **62 / 62** | real Core + worker + seeded 8,000-visitor history, headless Chromium, every screen |
| `@pratibimba/next` | 123 / 0 | SSR decide, cookies, proxy, `<prb.Slot>`, keep-alive pool |
| `@pratibimba/edge` | 109 / 0 | marker rewriter, streaming, snapshot reader, Workers/Deno/Vercel handlers |
| `@pratibimba/client` | 115 / 0 | script-tag swap in jsdom, including slots a single-page app mounts later |
| `@pratibimba/agent` | 119 / 0 | installer, MCP, `scan` across JSX/TSX, HTML, Vue, Svelte, Astro |
| `deploy/proxy-worker` | 77 / 0 | edge decide, queue drain, tombstones, Bloom erased-set |

Counts above are the latest recorded in the project tracker, as of the S11 pass (2026-10-20).

**Mutation testing.** Each feature's suite is held to killing every planted mutant before
the box ticks on `main`. Representative recent runs:

| Feature pass | Mutants | Killed | Notes |
|---|---|---|---|
| Images and assets (S11) | 65 | 65 | SVG script, type spoofing, size cap, path traversal, cross-site access, GC, renderers, scan |
| Edge round-trip cut (`latency-mutants/`) | 20 | 20 | two tests fail on the pre-cut code; the rest pin semantics |
| Erased-set Bloom filter (G5) | 20 | 20 | 9 Core, 11 Worker |
| Scan tokenizer (G4) | 84 | 84 | 26 new; three survived first run and were killed with sharper fixtures |
| MCP `create_slot` with kind (F5b) | 11 | 11 | plus 58 agent-side mutants |
| Agent scan (F5) | 50 | 50 | two survived first run and were killed by added tests |
| Sorted slot SDKs (F2b) | 13 | 13 | plus the trap: an equivalent mutation (unmoved nodes stay put) flagged a MISS and had to be ruled out |
| Sorted slot Core (F2a) | 12 | 12 | |
| Commerce events + retry (F1) | 4 | 4 | |
| Traffic share (`b0d34fa`) | 7 | 7 | first run had one survivor on the baseline branch |
| Cold-start, PII features, consent, erase, attribution, insights, experiments, plugins | each 100% | — | per-feature folders under the tracker; survivors are either killed with a new test or recorded as equivalent |

**Measured latency work.** We cut `/decide` from ~17 round trips to **4**, measured through
a counting TCP proxy in `core/tests/decide_fast.rs` (14 round trips in full mode and 18 in
cookieless/pick mode before; 4 and 4 after; 1 for a sticky reuse). All numbers below are
measured on our test deployment after the cut (Cloud Run + managed Postgres in the same region, Seoul, 2026-10-08):

| Measurement | Before the cut | After the cut |
|---|---|---|
| Server p50 / p95 warm, `/decide` (same service) | 189 / 283 ms (n=50) | **29 / 38 ms** (n=105) |
| Node keep-alive p50 / p95, India → Seoul (n=30) | 506 / 741 ms | 233 / 263 ms |
| curl per-call p50 / p95, India → Seoul (n=25) | 647 / 883 ms | 484 / 515 ms |
| `/health` server p50 (ping + `SELECT 1`) | — | 15 ms |

**Edge decide, consented visitor** (`deploy/proxy-worker`, keep-alive Node, 30 calls per PoP):
near PoP SIN p50 **164 ms** p95 253 ms; HKG p50 168 p95 298; MXP p50 265 p95 505; BOS p50
323 p95 526. Edge `/e` with the queue on at SIN: p50 **126 ms** p95 164 (vs 274 / 344 with
the queue off). Queue end-to-end (edge → Postgres, 70 batches, 2,110 messages): p50 3.4 s,
p95 8.6 s, 0 refused, 0 retried.

**Delayed-feedback learning.** The worker learns at window close rather than at click
arrival. In a scripted cohort of 2,000 synthetic visitors (`core/tests/cohort.rs`) that is
checked both ways, learn-at-close recovers the planted winner, while learn-at-click biases
the model toward short-path conversions. A control run with learning off stays near an even
split.

**Parity CI.** `ci/parity.sh` runs the same inputs through Core and the edge Worker and
diffs every decision; it is part of what each edge change has to pass.

**Reproducible decisions.** Every decision stores its features, every arm's sampled score,
the policy version and the random seed. The simulator and the dashboard's replay use that
record directly; the counterfactual lets you ask "what would the next-best arm have done" on
the stored sample.

## Self-hosting

Pratibimba is one Docker image and a Postgres database. You run it next to your product; your
pages ask it what to show. Nothing calls home and nothing here needs a cloud account.

| Piece | What it does | How many |
|---|---|---|
| `pratibimba serve` | HTTP: `/decide`, `/e`, `/manifest`, `/sdk.js`, `/assets`, the dashboard at `/app`, its API at `/api`, MCP at `/mcp` | one or more |
| `pratibimba worker` | the learning loop: credits events, settles attribution windows, updates models, sends webhooks, deletes unused images | one (two is safe) |
| Postgres 16 or 17 | all state | one |
| `pratibimba backup` | a daily SQL dump (optional, in the compose file) | one |

[`examples/compose.yml`](examples/compose.yml) is the reference install: all four, with a
volume for the database and one for uploaded images. The image is built from this repo; no
prebuilt image is published yet.

### 1. Start it

```sh
git clone https://github.com/OneWeekendAI/pratibimba && cd pratibimba
export PRB_ADMIN_TOKEN=$(openssl rand -hex 32)      # keep it: it is the dashboard password
export COMPOSE_FILE=examples/compose.yml            # every `docker compose` below uses it
docker compose up -d                                # first run builds the image: several minutes
curl -s localhost:8080/health                       # -> ok
```

`serve` and `worker` run the database migrations on start. Copy the compose file next to your
own infrastructure and change what you need (the Postgres password, the port, the volumes).

### 2. Declare your site

A site, its slots and its approved variants come from a JSON file you keep in your repo.
[`examples/plain/site.json`](examples/plain/site.json) is a small one; `origin` is your site's
URL (the install wizard probes it) and `site` is the id your pages name.

```sh
docker compose run --rm --no-deps -v "$PWD/examples/plain/site.json:/site.json:ro" serve apply /site.json

curl -s localhost:8080/decide -H 'content-type: application/json' \
  -d '{"site":"demo","slot":"hero","subject":"visitor-1"}'      # -> {"decision_id":…,"variant":…}
```

Re-run `apply` after every edit. Variants in the file count as approved by you; new ones an agent
or a teammate drafts wait in the dashboard's approval queue. A copy slot answers with its fields
in `copy` (`payload` is for swap HTML).

### 3. Sign in to the dashboard

Open `http://localhost:8080/app/` and sign in with `PRB_ADMIN_TOKEN`. Without the variable
`/api` answers 503 and the dashboard cannot sign in; the public endpoints still run.

| Token | Who uses it | How you get one |
|---|---|---|
| admin (`PRB_ADMIN_TOKEN`) | you, in the dashboard (cookie) or as `Authorization: Bearer` on `/api` | you choose it; rotate by changing the variable and restarting `serve` |
| MCP (`prb_mcp_…`, per site) | your coding agent on `POST /mcp` | the site's page in the dashboard, "Connect your coding agent", or `docker compose run --rm --no-deps serve mcp-token <site> <name>`; shown once, only its hash is stored |
| edge (`PRB_EDGE_TOKEN`) | the optional edge Worker | you choose it, set on both sides |

Pages need no token: a site id is public, like an analytics key.

**See it on a page.** [`examples/plain`](examples/plain) calls Core at `localhost:8080` from
`localhost:5173`, so Core must allow that origin:

```sh
PRB_SITE_ORIGIN=http://localhost:5173 docker compose up -d serve
python3 -m http.server 5173 -d examples/plain       # open http://localhost:5173
```

Each fresh visitor (a private window) may get another hero or banner; the dashboard's site page
shows the decisions arrive.

### 4. Put it behind HTTPS

Run Core on a private port and put a TLS-terminating reverse proxy in front. Two shapes, often
both:

- **Core on its own host** (`core.example.com`) for the dashboard, MCP and server-side SDKs.
  With [Caddy](https://caddyserver.com): `core.example.com { reverse_proxy 127.0.0.1:8080 }`.
  The proxy must send `X-Forwarded-Proto: https` (Caddy does; in nginx add
  `proxy_set_header X-Forwarded-Proto $scheme;`); the dashboard cookie is then marked `Secure`.
- **`/_prb/*` on your site's own origin**, forwarded to Core with the prefix stripped (nginx:
  `location /_prb/ { proxy_pass http://127.0.0.1:8080/; }`). The browser then only talks to
  your domain: no CORS, first-party cookie. The Next.js SDK does this itself (its `/_prb`
  route); the script tag and the edge SDK expect you to.

`/api`, `/app` and `/mcp` are token-gated, but you can keep them off the public internet
entirely and expose only `/decide`, `/e`, `/manifest`, `/sdk.js` and `/assets`. Set
`PRB_SITE_ORIGIN` only when a page calls Core from another origin (local testing).

### 5. Connect your site

Every SDK takes your Core URL; see [Install on your site](#install-on-your-site) for each.

| Your site | Install | Core URL goes in |
|---|---|---|
| Next.js 16 | `@pratibimba/next` | `createPratibimba({ core: process.env.PRB_CORE_URL, site })`; can be the private address |
| Any origin behind a Worker (Cloudflare, Deno Deploy, Vercel Edge) | `@pratibimba/edge` | `createEdge({ core, site })` |
| Anything else, static HTML included | script tag | the `/_prb/*` proxy above; or `src="https://core.example.com/sdk.js"` plus `PRB_SITE_ORIGIN` |

Install from npm: `npm i @pratibimba/next`, `npm i @pratibimba/edge`; see
[Packages](#packages). The script tag needs no package: Core serves it.

### Configuration

Core is configured entirely from the environment. Set the same values on `serve` and `worker`.

| Variable | Default | Meaning |
|---|---|---|
| `PRB_DB` (or `DATABASE_URL`) | required | Postgres URL |
| `PRB_ADMIN_TOKEN` | — | turns on the dashboard and `/api`; without it `/api` answers 503 |
| `PRB_BIND` | `0.0.0.0:8080` | listen address for `serve` |
| `PRB_SITE_ORIGIN` | — | comma-separated CORS origins; only when the page is not same-origin with Core |
| `PRB_DB_MAX_CONNECTIONS` | `10` | pool size, per process |
| `PRB_WORKER_TICK_MS` | `1000` | how often `worker` closes attribution windows (events wake it at once) |
| `PRB_DASHBOARD_DIR` | set in the image | the built dashboard (`dashboard/out`), served at `/app` |
| `PRB_PG_BIN` | `PATH` | directory holding `pg_dump` and `psql`, for `backup` and `restore` |
| `PRB_ERASE_PURGE_AFTER_SECS` | 1 day (max 30 days) | how long erased events sit tombstoned before deletion |
| `PRB_ASSETS_DIR` | `prb-assets` (`/var/lib/pratibimba/assets` in the image) | where uploaded images live, as `<site>/<sha256>` |
| `PRB_ASSETS_S3_BUCKET`, `_ENDPOINT`, `_ACCESS_KEY`, `_SECRET_KEY` | — | store images in an S3-compatible bucket instead; set the bucket and the rest are required |
| `PRB_ASSETS_S3_REGION` | `us-east-1` | the bucket's signing region (`auto` for R2 also works) |
| `PRB_ASSET_MAX_BYTES` | 5 MiB (max 50 MiB) | the largest image an upload may be |
| `PRB_ASSET_GC_GRACE_SECS` | 7 days (min 1 hour) | how long an image no live variant names is kept before the worker deletes it |
| `PRB_EDGE_KV_ACCOUNT`, `_NAMESPACE`, `_TOKEN` | — | Cloudflare Workers KV for the edge snapshot; all three or none (see [Edge](#optional-the-edge-worker)) |
| `PRB_EDGE_KV_API` | Cloudflare's API | override the KV API base (tests) |
| `PRB_EDGE_BEACON_URL` | — | optional per-visitor beacon endpoint on the edge Worker |
| `PRB_EDGE_TOKEN` | — | shared secret the edge Worker uses to talk to Core |
| `RUST_LOG` | `info` | log filter |

### Image storage

Images a variant shows can be a URL you host (`{"url": "/img/a.webp"}`) or an upload to Core
(`{"asset": "<sha256>"}`; see [Images and layout](#images-and-layout)). Uploads go to:

| Store | Set | Use when |
|---|---|---|
| local directory (default) | `PRB_ASSETS_DIR`; mount a volume on it, shared by `serve` and `worker` | one host |
| S3-compatible bucket (AWS S3, Cloudflare R2, MinIO; path-style, SigV4) | `PRB_ASSETS_S3_*` on `serve` and `worker`; drop the volume | several `serve` replicas, or you want the bucket's own durability |

Core serves both at `/assets/<site>/<sha256>`, cached a year, so a CDN in front is safe.

### Backups and restore

`pratibimba backup [dir-or-file]` writes a timestamped plain-SQL dump with `pg_dump`. The
privacy salts are left out by design, so a backup cannot link one day's cookieless visitors to
the next. With images in a local directory it also copies them to `<dump>.assets/`; a bucket is
not copied, so back it up with its own versioning or replication. The compose `backup` service
writes one to `examples/backups/` on start and then daily.

`pratibimba restore <file>` loads a dump into an **empty** `PRB_DB` in one transaction and copies
`<dump>.assets/` back into `PRB_ASSETS_DIR`. The image's `pg_dump` is Debian's 17; against a
newer server `backup` stops with an error naming both versions.

```sh
docker compose run --rm --no-deps --entrypoint pratibimba backup backup /backups   # one now

# Restore onto a new host, or one you wipe first (`down -v` DELETES the database and images).
docker compose down -v && docker compose up -d --wait db
docker compose run --rm --no-deps -v "$PWD/examples/backups:/backups:ro" serve restore /backups/<file>.sql
docker compose up -d
```

`restore` refuses a database that is not empty, so a running `serve` (which migrates on start)
must not come up first. `down` stops the stack and keeps the volumes.

### Upgrades

```sh
docker compose run --rm --no-deps --entrypoint pratibimba backup backup /backups   # back up first
git pull && docker compose build && docker compose up -d
```

Migrations run when `serve` or `worker` starts, under a lock, so replicas starting together
are safe. They only go forward: to roll back, run the old image against a restore of the
backup you took. Expect breaking changes until 1.0.

### Scaling

- **`serve` is stateless**: run as many as you like behind the proxy. Use the S3 store for
  images when replicas do not share a disk. Each holds `PRB_DB_MAX_CONNECTIONS` connections.
- **`worker`**: one is enough. A second is safe, not faster: event and webhook rows are claimed
  with `FOR UPDATE SKIP LOCKED` and each model row is locked around its update, so no event is
  credited twice and no update is lost.
- **Postgres** holds everything; size it for your event volume. The dashboard reads rollups,
  not raw events.

### Privacy modes

Set per site in the site file (`"privacy_mode"`):

| Mode | Subject | `returning` feature |
|---|---|---|
| `consent_aware` (default) | `full` for a request with consent (`_prb_consent`, IAB TCF v2 purpose 1, OneTrust C0002), `cookieless` otherwise | with consent |
| `cookieless` | stored only as a hash with a salt that rotates daily, so days cannot be joined | never |
| `full` | stored as the SDK sends it (the first-party `_prb_subject` cookie, or your user id) | yes |

`fields_allowed` limits which optional context fields are stored, and `pratibimba erase <site>
<subject> --by <you>` (or the dashboard) is the right-to-erase. See [Privacy](#privacy).

### Optional: the edge Worker

Not needed to self-host. For decisions at the CDN, [`deploy/proxy-worker`](deploy/proxy-worker)
is a Cloudflare Worker that reads a per-site snapshot Core writes to Workers KV and decides at
the edge for consented visitors, falling back to Core otherwise. Set the `PRB_EDGE_*` variables
on Core; [`deploy/README.md`](deploy/README.md) has the steps.

### Without Compose

```sh
docker build -t pratibimba .
export PRB_DB=postgres://user:pass@your-db:5432/pratibimba
docker run --rm -e PRB_DB -v $PWD/site.json:/site.json:ro pratibimba apply /site.json
docker run -d -e PRB_DB -e PRB_ADMIN_TOKEN -v prb-assets:/var/lib/pratibimba/assets -p 8080:8080 pratibimba   # serve
docker run -d -e PRB_DB -v prb-assets:/var/lib/pratibimba/assets pratibimba worker
```

Or from source: `cargo build --release`, then `./target/release/pratibimba apply|serve|worker`
with the same variables (`PRB_DASHBOARD_DIR=dashboard/out` after `cd dashboard && npm ci && npm
run build`).

## Install on your site

### Install on a Next.js page

The dashboard's install wizard (`/app/install/?site=<id>`) probes your site for each step
and ticks itself off. Code form:

```ts
// lib/prb.ts
import { createPratibimba } from '@pratibimba/next';
export const prb = createPratibimba({ core: process.env.PRB_CORE_URL!, site: 'demo' });

// proxy.ts  (Next.js 16; was middleware.ts)
export const proxy = prb.proxy;

// app/%5Fprb/[...path]/route.ts  (literal "_prb" is private in the App Router)
export const { GET, POST } = prb.route;

// any server component
<prb.Slot id="hero" variants={{ gift: <GiftHero /> }}><Hero /></prb.Slot>

// a sorted slot: keyed children, rendered in the order Core decides
<prb.Sorted id="grid">{products.map((p) => <Card key={p.slug} p={p} />)}</prb.Sorted>

// a copy slot: typed fields; text as plain strings, images as {src, alt}
<prb.Copy id="hero_copy" fallback={{ headline: 'Roasted on Monday', cta_label: 'Shop',
                                     banner: { src: '/beans.jpg', alt: 'Coffee beans' } }}>
  {(t) => <><prb.Image image={t.banner} /><h1>{t.headline}</h1><a href="/shop">{t.cta_label}</a></>}
</prb.Copy>
```

### Install on any other origin (edge worker)

```js
import { createEdge } from '@pratibimba/edge';
const prb = createEdge({ core: 'https://core.internal', site: 'demo' });
export default { fetch: (req) => prb.handleRequest(req) };   // Cloudflare, Deno Deploy and Vercel Edge all take this handler
```

Mark slots in the origin's HTML with the comment pair:

```html
<!-- prb:slot="hero" -->
  <h1>Default headline</h1>
-->
<!-- /prb:slot -->
```

### Install on a plain page (script tag)

Proxy `/_prb/*` on your own origin to Core (so the browser only ever talks to your domain)
and add both tags to `<head>`:

```html
<script src="/_prb/manifest/demo.js"></script>
<script src="/_prb/sdk.js" data-site="demo"></script>
```

The script hides each marked slot's element, calls `/decide`, swaps the DOM and reveals,
with a hard timeout that reveals the baseline rather than blocking the page.

It works with single-page apps (React, Vue, Svelte) too: a slot element that mounts after the
page loaded gets its decision as it mounts, before the next paint. One that mounts before its
decision stays hidden for at most `data-timeout` from its own mount, then shows the page's
version. A slot the visitor has already seen, or has the pointer on, is never swapped. Render a
slot's contents once and leave them static: the framework must not re-render children the
script replaced (in React, a constant `dangerouslySetInnerHTML`, or plain text in
`data-prb-field` elements of a copy slot).

### Images and layout

A variant can change what a visitor sees, not only what they read. Two ways, both behind the
same approval gate:

**An image field** changes a picture without anyone writing HTML. Declare it on a copy slot
and mark the `<img>` (from [`examples/plain`](examples/plain)):

```json
{ "key": "banner", "selector": "#banner", "kind": "copy",
  "copy": { "fields": [{ "name": "image", "type": "image", "required": true }] },
  "variants": [
    { "key": "own", "baseline": true },
    { "key": "autumn", "copy": { "image": { "url": "/img/autumn.svg", "alt": "Amber autumn candles", "width": 320, "height": 120 } } }
  ] }
```

```html
<section id="banner">
  <img data-prb-field="image" src="/img/beans.svg" alt="Coffee beans on a counter" width="320" height="120">
</section>
```

The script tag and the edge rewriter set `src`, `alt` and size as attributes and drop the
`srcset`/`<source>`s that would override them; in Next.js render it with `<prb.Image>`. A value
is `{ "url": "https://…" | "/path" }` or `{ "asset": "<id>" }` for an image uploaded to Core:

```sh
curl -X POST --data-binary @autumn.webp -H "authorization: Bearer $PRB_ADMIN_TOKEN" \
  'localhost:8080/api/sites/demo/assets?name=autumn.webp'     # -> {"id": "<sha256>", ...}
```

or the dashboard's Images shelf, or the agent's `upload_asset` tool. The type is judged by the
bytes, not the name; uploads are size-capped; an asset can only be named by its own site.
Core serves it at `/assets/<site>/<id>` (your page uses `/_prb/assets/<site>/<id>`).

**A swap variant** changes the layout. Its HTML can rearrange the region, for example the
image beside the headline instead of below it:

```json
{ "key": "side_by_side", "html": "<div style=\"display:flex;gap:16px;align-items:center\"><img src=\"/img/autumn.svg\" alt=\"Autumn candles in amber jars\" width=\"160\" height=\"60\"><div><h1>Autumn candles are here</h1><a href=\"#shop\">Shop autumn</a></div></div>" }
```

The dashboard previews every variant as it looks: swap HTML in a sandboxed frame, image
fields as the images, side by side with the champion.

### Point your coding agent at it

```sh
# In the project the SDK is installed in:
npx @pratibimba/agent scan           # proposes candidate slots from your repo
npx @pratibimba/agent install        # drops the instruction pack into .claude / .cursor / AGENTS.md
```

Then mint a per-site MCP token in the dashboard and point your agent at `POST /mcp` on your
Core. The agent reads results and drafts variants in real components; a human still has to
approve every draft before it is served.

### Packages

```sh
npm i @pratibimba/next      # Next.js 16
npm i @pratibimba/edge      # Cloudflare Workers, Deno Deploy, Vercel Edge, Node
npm i @pratibimba/client    # the script-tag fallback, to self-host a pinned copy
npx @pratibimba/agent install
```

The examples in this repo depend on the packages with `file:` paths, so they run against the
working tree, and need `install-links=true` in `.npmrc` (Turbopack does not resolve a
symlinked package outside the app).

## Repo layout

| Path | What's in it |
|---|---|
| `core/` | the Rust binary: HTTP server, worker, MCP, bandit math, migrations, parity, cohort sims |
| `core/migrations/` | SQL migrations (`0001_init.sql` … `0270_assets.sql`) |
| `core/tests/` | integration tests on throwaway databases (bandit, decide, worker, edge, cohort, …) |
| `dashboard/` | the Next.js dashboard, built as a static export, served by Core at `/app` |
| `packages/next/` | `@pratibimba/next` SDK |
| `packages/edge/` | `@pratibimba/edge` SDK |
| `packages/client/` | the script-tag fallback (embedded in Core via `include_str!`) |
| `packages/agent/` | `@pratibimba/agent` MCP instruction pack and `scan` CLI |
| `examples/next/` | plain Next.js example |
| `examples/edge/` | plain Cloudflare Worker example |
| `examples/plain/` | script-tag example with a declared `site.json`: a layout swap and an image field |
| `examples/shop/` | a Next.js store with commerce events, an agent-drafted hero and sorted grid |
| `examples/compose.yml` | self-hosted Postgres + Core + worker + nightly backup |
| `deploy/` | optional, not the install path: the edge Worker (`proxy-worker/`), a Cloud Run example (`cloud-run/`), latency and edge checks (`checks/`); see `deploy/README.md` |
| `ci/` | `parity.sh` (edge vs Core on identical inputs), `direct.mjs`, local stack bringup |
| `Dockerfile` | one image, several entry points (`serve`, `worker`, `migrate`, `apply`, `backup`, `restore`, `erase`) |

## HTTP API (summary)

| Method and path | What it does |
|---|---|
| `POST /decide` | returns `{decision_id, variant, payload, explanation}` for one slot and subject |
| `POST /decide/batch` | several slots for one subject in one call |
| `POST /e` | up to 100 events per call (`exposure`, `click`, `pick`, `rescore`, `addToCart`, `purchase`); `id` is your idempotency key |
| `GET /manifest/:site` | slots and approved variant keys (`.js` for the script form); edge-cacheable |
| `GET /sdk.js` | the client script |
| `GET /health` | 200 when the database answers |
| `GET /assets/:site/:sha256` | an uploaded image; cached a year (`immutable`), served with `nosniff` and a CSP that cannot run script |
| `POST /mcp` | the MCP server (JSON-RPC), per-site `prb_mcp_…` token |
| `POST /webhook/stripe/:site` | Stripe Checkout conversions, verified with the site's signing secret |
| `POST /webhook/conversion/:site` | generic signed conversions |
| `/api/*` | the dashboard API, behind `PRB_ADMIN_TOKEN` or the session cookie — see the dashboard screens |

A late platform webhook can **correct a settled miss** inside the attribution window: the
decision flips, the model is updated exactly (`+w·x` on log-likelihood, covariance unchanged),
and a `conversion` webhook refires with `late: true`.

## Running the tests

```sh
# Core — any Postgres where you can create databases; each test gets a fresh one.
export DATABASE_URL=postgres://postgres@localhost:5432/postgres
cargo test                                        # ~957 tests: bandit, HTTP, worker, migrations, cohorts, edge, parity

# TypeScript packages
cd packages/client && npm install && npm test     # the script, in jsdom
cd packages/next   && npm install && npm test     # unit; `npm run e2e` runs against real Core + worker
cd packages/edge   && npm install && npm test     # unit; `npm run e2e` same
cd packages/agent  && npm install && npm test     # installer, MCP, scan
cd packages/agent  && npm run mutate              # mutation suite

# Dashboard — headless Chromium against real Core and a seeded history.
cargo build && cd dashboard && npm install && npx playwright install chromium
npm run build && PRB_E2E_ADMIN_DB=$DATABASE_URL npm run e2e

# Edge-vs-Core parity CI
ci/parity.sh
```

The dashboard's e2e stack (`dashboard/e2e/stack.mjs`) seeds 8,000 visitors with a planted
ground truth in two halves so the bandit learns from the first and the history shows it, then
runs the real worker. The cohort tests (`core/tests/cohort.rs`) push 2,000 synthetic visitors
with planted truth through decide, event and worker, and compare against a learning-off
control.

## Licence

MIT. See [LICENSE](LICENSE).
