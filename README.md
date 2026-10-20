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
- **Three slot kinds.** `swap` picks one variant; `sorted` learns a per-item order with
  cascade credit (misses above the acted item, reward to it, nothing below); `copy` renders
  typed plain-text fields with length limits.
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
  file:line, selector, kind (swap/copy/sorted) and the exact `create_slot` arguments.

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
| `cargo test` (Core) | **957 / 0** | bandit math, HTTP, worker, migrations, cohort sims, replay, erase, edge snapshot, parity, webhooks |
| `dashboard` Playwright e2e | **58 / 58** | real Core + worker + seeded 8,000-visitor history, headless Chromium, every screen |
| `@pratibimba/next` | 111 / 0 | SSR decide, cookies, proxy, `<prb.Slot>`, keep-alive pool |
| `@pratibimba/edge` | 102 / 0 | marker rewriter, streaming, snapshot reader, Workers/Deno/Vercel handlers |
| `@pratibimba/client` | 109 / 0 | script-tag swap in jsdom, including slots a single-page app mounts later |
| `@pratibimba/agent` | 107 / 0 | installer, MCP, `scan` across JSX/TSX, HTML, Vue, Svelte, Astro |
| `deploy/proxy-worker` | 77 / 0 | edge decide, queue drain, tombstones, Bloom erased-set |

Counts above are the latest recorded in the project tracker; `cargo test` is the number
reported after the G4 pass (2026-10-16).

**Mutation testing.** Each feature's suite is held to killing every planted mutant before
the box ticks on `main`. Representative recent runs:

| Feature pass | Mutants | Killed | Notes |
|---|---|---|---|
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

## Quickstart

### Self-hosted stack (Docker Compose)

The compose file at [`examples/compose.yml`](examples/compose.yml) runs Postgres, Core
(serve), the worker and a nightly backup:

```sh
export PRB_ADMIN_TOKEN=change-me
docker compose -f examples/compose.yml up -d     # first run builds the image: several minutes

# Declare a site, its slots and its variants (re-run after every edit to the file).
docker compose -f examples/compose.yml run --rm --no-deps \
  -v "$PWD/examples/plain/site.json:/site.json:ro" serve apply /site.json

# Ask for a decision.
curl -s localhost:8080/decide -H 'content-type: application/json' \
  -d '{"site":"demo","slot":"hero","subject":"visitor-1"}'
```

Open `http://localhost:8080/app/` and sign in with `PRB_ADMIN_TOKEN`. Events go to
`POST /e` with the `decision_id` (see [HTTP API](#http-api-summary)); the worker credits them
at once and updates the model when the slot's `attribution_window_secs` closes.

The backup service writes a plain-SQL dump to `examples/backups/` (next to the compose file)
on start and then once a day; the salts are left out of the dump by design, so a backup cannot
link one day's cookieless visitors to the next. To take one now, or restore one into a new,
empty database:

```sh
docker compose -f examples/compose.yml run --rm --no-deps --entrypoint pratibimba backup backup /backups
docker compose -f examples/compose.yml exec db createdb -U postgres restored
docker compose -f examples/compose.yml run --rm --no-deps --entrypoint pratibimba \
  -e PRB_DB=postgres://postgres:pratibimba@db:5432/restored backup restore /backups/<file>.sql
```

`docker compose -f examples/compose.yml down` stops the stack (keep `PRB_ADMIN_TOKEN` set);
`down -v` also deletes the database volume.

### Self-hosted (plain Docker)

```sh
docker build -t pratibimba .
export PRB_DB=postgres://user:pass@your-db:5432/pratibimba

# 1. Declare a site, its slots and its variants.
docker run --rm -e PRB_DB -v $PWD/examples/plain/site.json:/site.json:ro pratibimba apply /site.json

# 2. Run the server and the learning worker.
docker run -d -e PRB_DB -e PRB_ADMIN_TOKEN=change-me -p 8080:8080 pratibimba   # serve
docker run -d -e PRB_DB pratibimba worker
```

### Self-hosted (cargo)

```sh
cargo build --release
export PRB_DB=postgres://…/pratibimba
./target/release/pratibimba apply examples/plain/site.json
PRB_ADMIN_TOKEN=change-me ./target/release/pratibimba serve &
./target/release/pratibimba worker &
```

Both `serve` and `worker` run the migrations on start. Without `PRB_ADMIN_TOKEN`, `/api`
answers 503 and only the public endpoints run.

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

// a copy slot: typed plain-text fields with length limits
<prb.Copy id="hero_copy" fallback={{ headline: 'Roasted on Monday', cta_label: 'Shop' }}>
  {(t) => <><h1>{t.headline}</h1><a href="/shop">{t.cta_label}</a></>}
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

### Point your coding agent at it

```sh
# In the project the SDK is installed in:
npx @pratibimba/agent scan           # proposes candidate slots from your repo
npx @pratibimba/agent install        # drops the instruction pack into .claude / .cursor / AGENTS.md
```

Then mint a per-site MCP token in the dashboard and point your agent at `POST /mcp` on your
Core. The agent reads results and drafts variants in real components; a human still has to
approve every draft before it is served.

### Package status

The TypeScript packages (`@pratibimba/next`, `@pratibimba/edge`, `@pratibimba/client`,
`@pratibimba/agent`) are **not yet published to npm**. The examples in this repo depend on
them with `file:` paths and need `install-links=true` in `.npmrc` (Turbopack does not resolve
a symlinked package outside the app). To consume them in your own project today, build from
this repo and copy `packages/<name>/dist` into your tree, or point your `package.json` at a
Git ref.

## Repo layout

| Path | What's in it |
|---|---|
| `core/` | the Rust binary: HTTP server, worker, MCP, bandit math, migrations, parity, cohort sims |
| `core/migrations/` | SQL migrations (`0001_init.sql` … `0260_share_webhook.sql`) |
| `core/tests/` | integration tests on throwaway databases (bandit, decide, worker, edge, cohort, …) |
| `dashboard/` | the Next.js dashboard, built as a static export, served by Core at `/app` |
| `packages/next/` | `@pratibimba/next` SDK |
| `packages/edge/` | `@pratibimba/edge` SDK |
| `packages/client/` | the script-tag fallback (embedded in Core via `include_str!`) |
| `packages/agent/` | `@pratibimba/agent` MCP instruction pack and `scan` CLI |
| `examples/next/` | plain Next.js example |
| `examples/edge/` | plain Cloudflare Worker example |
| `examples/plain/` | script-tag example with a declared `site.json` |
| `examples/shop/` | a Next.js store with commerce events, an agent-drafted hero and sorted grid |
| `examples/compose.yml` | self-hosted Postgres + Core + worker + nightly backup |
| `deploy/` | optional, not the install path: the edge Worker (`proxy-worker/`), a Cloud Run example (`cloud-run/`), latency and edge checks (`checks/`); see `deploy/README.md` |
| `ci/` | `parity.sh` (edge vs Core on identical inputs), `direct.mjs`, local stack bringup |
| `Dockerfile` | one image, several entry points (`serve`, `worker`, `migrate`, `apply`, `backup`, `restore`, `erase`) |

## Configuration

Core is configured entirely from the environment.

| Variable | Default | Meaning |
|---|---|---|
| `PRB_DB` (or `DATABASE_URL`) | required | Postgres URL |
| `PRB_BIND` | `0.0.0.0:8080` | listen address for `serve` |
| `PRB_SITE_ORIGIN` | — | comma-separated CORS origins; set only when the page is not same-origin with Core |
| `PRB_WORKER_TICK_MS` | `1000` | how often `worker` closes attribution windows (events wake it immediately) |
| `PRB_DB_MAX_CONNECTIONS` | `10` | pool size |
| `PRB_ADMIN_TOKEN` | — | turns on the dashboard API at `/api`; without it `/api` answers 503 |
| `PRB_DASHBOARD_DIR` | set in the Docker image | the built dashboard (`dashboard/out`), served at `/app` |
| `PRB_PG_BIN` | `PATH` | directory holding `pg_dump` and `psql`, for `backup` and `restore` |
| `PRB_ERASE_PURGE_AFTER_SECS` | 1 day (max 30) | how long erased events sit tombstoned before deletion |
| `PRB_EDGE_KV_ACCOUNT`, `PRB_EDGE_KV_NAMESPACE`, `PRB_EDGE_KV_TOKEN` | — | Cloudflare Workers KV for the edge snapshot; set all three or none |
| `PRB_EDGE_BEACON_URL` | — | optional per-visitor beacon endpoint on the edge Worker |
| `PRB_EDGE_TOKEN` | — | shared secret the edge Worker uses to talk to Core |
| `RUST_LOG` | `info` | log filter |

## HTTP API (summary)

| Method and path | What it does |
|---|---|
| `POST /decide` | returns `{decision_id, variant, payload, explanation}` for one slot and subject |
| `POST /decide/batch` | several slots for one subject in one call |
| `POST /e` | up to 100 events per call (`exposure`, `click`, `pick`, `rescore`, `addToCart`, `purchase`); `id` is your idempotency key |
| `GET /manifest/:site` | slots and approved variant keys (`.js` for the script form); edge-cacheable |
| `GET /sdk.js` | the client script |
| `GET /health` | 200 when the database answers |
| `POST /mcp` | the MCP server (JSON-RPC), per-site `prb_mcp_…` token |
| `POST /webhook/stripe/:site` | Stripe Checkout conversions, verified with the site's signing secret |
| `POST /webhook/conversion/:site` | generic signed conversions |
| `/api/*` | the dashboard API, behind `PRB_ADMIN_TOKEN` or the session cookie — see the dashboard screens |

A late platform webhook can **correct a settled miss** inside the attribution window: the
decision flips, the model is updated exactly (`+w·x` on log-likelihood, covariance unchanged),
and a `conversion` webhook refires with `late: true`.

## Backups

`pratibimba backup [dir-or-file]` writes a timestamped plain-SQL dump with `pg_dump`,
leaving the privacy salts out by design. `pratibimba restore <file>` loads a dump into an
**empty** `PRB_DB` in one transaction. The Docker image includes `pg_dump` and `psql`
(Debian's 17); against a newer server, `backup` stops with an error naming both versions.

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
