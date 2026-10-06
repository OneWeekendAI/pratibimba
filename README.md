# Pratibimba

Open-source, self-hostable website personalisation for developers. You mark the regions of
your page that may change (slots), declare a few versions of each (variants), and Pratibimba
picks the version each visitor is most likely to act on. It learns from what visitors actually
do, keeps the data on your own Postgres, and can tell you why it showed what it showed.

*Pratibimba* (प्रतिबिम्ब, Sanskrit) means reflection: the page takes the shape of whoever is
looking at it.

> **Status: Phase 1 (developer-first).** One Rust binary on Postgres, a per-slot contextual
> bandit with a holdout and auto-rollback, three installs (`@pratibimba/next`, the
> `@pratibimba/edge` worker and a script-tag fallback) and a dashboard with an approval gate.
> Phase 2 has started: an open MCP server at `/mcp` and an agent instruction pack are in.
> Expect breaking changes.

## How it works

- **Core** is one Rust binary with two jobs. `pratibimba serve` answers `/decide`, `/e` and
  `/manifest/:site`. `pratibimba worker` credits conversions and updates the models. Run both
  against the same Postgres.
- **Decisions** come from a LinTS bandit per slot: Thompson sampling over a Bayesian logistic
  regression, one per variant. It uses three visitor features (`returning`, `mobile`,
  `from_ad`) plus a bias. No LLM sits on the decision path. Each decision is stored with its
  features, every variant's sampled score, the policy version and the random seed, so it can be
  replayed exactly.
- **Learning** happens once per decision, and only when the visitor actually saw it. A goal
  event that matches the slot's metric (for example `click`) inside the attribution window
  (default 30 minutes) counts as a conversion. An exposure whose window closes without one
  counts as a miss. A decision that was never shown teaches nothing.
- **A holdout keeps it honest.** Each site sends a fixed share of visitors (default 10%,
  `holdout_fraction` in the site file, at most 50%) the baseline regardless. Lift is always the
  policy against that holdout, with a 95% confidence sequence that stays valid however often
  you look.
- **Auto-rollback.** When the top of a variant's confidence sequence sits below the holdout
  by more than the site's margin, the worker pauses it, restores the slot's model to the
  version current when the variant was approved, and writes a sentence saying why.
- **Only approved variants are ever served.** Variants in the site file you apply are approved
  by you. A draft written in the dashboard waits at the approval gate until someone approves it,
  one at a time, with a planned traffic share capped at 50%.
- **No demographic inference.** The decision context is a closed set of named fields. Sending
  `gender`, `age` or any other undeclared field gets a 4xx response.

## Quickstart

You need Docker and a Postgres you can reach.

```sh
docker build -t pratibimba .
export PRB_DB=postgres://user:pass@your-db:5432/pratibimba

# 1. Declare a site, its slots and its variants (see examples/plain/site.json).
docker run --rm -e PRB_DB -v $PWD/examples/plain/site.json:/site.json:ro pratibimba apply /site.json

# 2. Run the server and the learning worker.
docker run -d -e PRB_DB -e PRB_ADMIN_TOKEN=change-me -p 8080:8080 pratibimba   # serve (the default command)
docker run -d -e PRB_DB pratibimba worker
```

Open `http://localhost:8080/app/` and sign in with the admin token. Without
`PRB_ADMIN_TOKEN` the dashboard API answers 503 and only the public endpoints run.

Without Docker, use `cargo build --release` and then `./target/release/pratibimba serve` (or
`worker`, `migrate`, or `apply <file>`). Both `serve` and `worker` apply the database
migrations on start.

### The site file

```json
{
  "site": "demo",
  "origin": "https://shop.example",
  "slots": [{
    "key": "hero",
    "selector": "#hero",
    "metric": "click",
    "attribution_window_secs": 1800,
    "decision_ttl_secs": 60,
    "variants": [
      { "key": "baseline", "baseline": true },
      { "key": "gift", "html": "<h1>Gift sets</h1><a href=\"/gifts\">Shop gifts</a>" }
    ]
  }]
}
```

Each slot has exactly one `baseline`: the content your page already renders, with no `html`.
Re-applying the file updates the site in place. A variant you remove from the file is retired,
not deleted.

### Install on a page

The dashboard's install wizard (`/app/install/?site=<id>`) walks through these and checks each
step itself: it probes your site for the SDK header, the `/_prb` route and the first exposure,
rather than asking you to tick a box.

**Next.js** (`@pratibimba/next`) decides during the server render, so the first byte already
carries the variant:

```ts
// lib/prb.ts
import { createPratibimba } from '@pratibimba/next';
export const prb = createPratibimba({ core: process.env.PRB_CORE_URL!, site: 'demo' });
// proxy.ts (Next.js 16 renamed middleware to proxy)
export const proxy = prb.proxy;
// app/%5Fprb/[...path]/route.ts (a plain _prb folder is private in the App Router)
export const { GET, POST } = prb.route;
// any server component
<prb.Slot id="hero" variants={{ gift: <GiftHero /> }}><Hero /></prb.Slot>
```

**Any other origin** (`@pratibimba/edge`) sits in front of it as an edge worker and rewrites
`<!-- prb:slot="hero" -->...<!-- /prb:slot -->` regions as the HTML streams:

```js
const prb = createEdge({ core: 'https://core.internal', site: 'demo' });
export default { fetch: (req) => prb.handleRequest(req) };   // Cloudflare; Deno and Vercel Edge take the same handler
```

See `examples/next` and `examples/edge`.

#### The script fallback

Proxy `/_prb/*` on your own origin to Core, so the browser only ever talks to your domain.
Then add both tags to `<head>`, without `defer` or `async`:

```html
<script src="/_prb/manifest/demo.js"></script>
<script src="/_prb/sdk.js" data-site="demo"></script>
```

The script:
- hides only the slots, never the whole page;
- asks Core for a decision;
- swaps the slot and reveals it.

A hard 400 ms timeout (`data-timeout`) puts the original content back if the decision is slow.
A decision that arrives after that is not swapped in. It is reported as `missed` instead.

**Be honest with yourself about this path.** On a slow network a slot can stay blank for up to
the timeout. The two server-side installs above decide before any HTML is sent, so they have
no flicker at all.

Clicks inside a slot are reported automatically. For other goals, call
`window.prb.track('purchase')`. The subject id lives in `sessionStorage` by default. Add
`data-cookie="on"` to use a one-year first-party `_prb_subject` cookie, which turns on the
`returning` feature, and collect consent before you do.

To try it locally, run `serve` with `PRB_SITE_ORIGIN=http://localhost:5173`, then serve
`examples/plain/` on port 5173.

## The dashboard

Core serves it at `/app` (a static Next.js export in `dashboard/`, pointed to by
`PRB_DASHBOARD_DIR`; the Docker image includes it). Sign in with `PRB_ADMIN_TOKEN`.

- **Site home**: live tiles over server-sent events (decisions in the last minute,
  conversions in the last hour, lift today against the holdout), an install-status strip, and
  the slot list with each slot's champion, 7-day lift and traffic split. The decisions tile
  turns amber when a busy site goes quiet.
- **Slot workbench**: a card per variant with its share, its lift and, if auto-rollback paused
  it, why; the cumulative lift curve with its confidence band; and the last 500 decisions,
  filterable by variant and outcome.
- **Decision detail**: one plain-English sentence on why this visitor saw this variant, then
  the features the bandit saw, every arm's score, whether replaying the stored seed
  reproduces them, a counterfactual (what the baseline or the policy would have shown), the
  event timeline and what reviewers wrote at the approval gate.
- **Results**: the site-level lift headline, slots ranked by lift times traffic, and a
  learnings feed written when the evidence crosses a threshold.
- **Approvals**: each draft with its diff against the champion, the constraint checks, a
  planned-traffic slider capped at 50% and a comment box. A failing check blocks approval.
  There is no bulk approve. You can write a draft by hand here too.
- **Install**: the six-step wizard above, for Next.js, the edge worker or the script tag.

## The MCP server

Your own coding agent (Claude Code, Cursor, any MCP client) can read Pratibimba's state and
propose variants. Core serves MCP at `/mcp` over the streamable HTTP transport, in the same
binary. Tokens are per site:

```bash
pratibimba mcp-token demo "Claude Code"      # prints prb_mcp_…; only its hash is stored
# or: POST /api/sites/demo/mcp-tokens {"name": "Claude Code"} with the admin token
claude mcp add --transport http pratibimba https://core.example.com/mcp \
  --header "Authorization: Bearer prb_mcp_…"
```

| Tool | What it does |
|---|---|
| `list_sites`, `get_site` | The token's site: origin, holdout, forbidden claims, slots. A token sees no other site. |
| `list_slots` | Each slot's selector, metric, champion and variant counts by status. |
| `list_variants` | A slot's variants with their HTML, approval details and the reviewer comment thread. |
| `get_results` | Per-slot and per-variant results with lift against the holdout. |
| `list_decisions`, `explain_decision` | Recent decisions, and why one visitor saw what they saw. |
| `propose_variant` | Submits a variant as a **draft**, with an optional note to the reviewer. Returns the constraint checks. |

- **An agent cannot approve.** There is no approve tool, `propose_variant` writes `draft`
  whatever it is sent (a `status` or `traffic_share` argument fails the call), and an MCP
  token is not accepted by `/api`. A person approves in the dashboard.
- **Every call is audited.** Each tool takes `agent` and `justification` (and optionally
  `on_behalf_of`); a call without them is refused. Every call, refused or not, writes a row
  to `mcp_calls`, and each result returns that row as `audit`. A draft links to the call that
  wrote it, so the approvals API shows the agent and its justification.

### The agent instruction pack

The MCP server gives your agent the tools. The pack teaches it how to use them. Run this in
your site's repo:

```sh
npx @pratibimba/agent install --url https://core.example.com    # /mcp is added for you
export PRATIBIMBA_MCP_TOKEN=prb_mcp_…                           # from pratibimba mcp-token
```

| File | For |
|---|---|
| `.claude/skills/pratibimba/SKILL.md`, `.mcp.json` | Claude Code |
| `.cursor/rules/pratibimba.mdc`, `.cursor/mcp.json` | Cursor |
| `.pratibimba/agent-instructions.md`, `.pratibimba/mcp.json` | any other MCP client |

The instructions are the same in all three files. They tell the agent to:
- read the slot's results, its variant history and a few decision explanations before it
  proposes anything;
- write each variant as a real component next to the baseline, using the repo's own components
  and copy and no invented facts, and send the same markup as the draft's `html`;
- give a specific justification on every call;
- never claim a draft is live, and never add variants through the site file, which would
  approve them.

The pack is yours to edit and commit. Running the install again never overwrites a pack file
you changed (`--force` does). In an MCP config it only adds the `pratibimba` entry and leaves
other servers alone. The token is never written to a file: the configs read it from
`PRATIBIMBA_MCP_TOKEN` (`--token-env` picks another name), as `${VAR}` for Claude Code and
`${env:VAR}` for Cursor.

It ships as an npm package rather than a `pratibimba agent` subcommand because it runs in your
site's repo, where Node is already installed. The Core binary runs on a server next to
Postgres.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `PRB_DB` (or `DATABASE_URL`) | (required) | Postgres URL |
| `PRB_BIND` | `0.0.0.0:8080` | listen address for `serve` |
| `PRB_SITE_ORIGIN` | (none) | comma-separated CORS origins; only needed when the page is not same-origin with Core |
| `PRB_WORKER_TICK_MS` | `1000` | how often `worker` closes attribution windows (events wake it immediately) |
| `PRB_DB_MAX_CONNECTIONS` | `10` | pool size |
| `PRB_ADMIN_TOKEN` | (none) | turns on the dashboard API at `/api`; without it `/api` answers 503 |
| `PRB_DASHBOARD_DIR` | (none; set in the Docker image) | the built dashboard (`dashboard/out`), served at `/app` |
| `RUST_LOG` | `info` | log filter |

## HTTP API

| Method and path | What it does |
|---|---|
| `POST /decide` | `{site, slot, subject, ctx: {returning, mobile, from_ad}}` returns `{decision_id, variant, payload, explanation}`. The same subject and slot inside the slot's TTL get the same decision. |
| `POST /e` | `{site, subject, events: [{id, name, slot?, decision_id?}]}`. Accepts up to 100 events per call. `id` is your idempotency key. |
| `GET /manifest/:site` | Slots and approved variant keys. Append `.js` for the script form. Edge-cacheable. |
| `GET /sdk.js` | The client script. |
| `POST /decide/batch` | Several slots for one subject in one call: `{site, subject, slots, ctx}`. |
| `GET /health` | 200 when the database answers. |
| `POST /mcp` | The MCP server (JSON-RPC), behind a per-site `prb_mcp_…` token. See above. |

The dashboard API lives under `/api` and needs `Authorization: Bearer $PRB_ADMIN_TOKEN` (or the
session cookie that `POST /api/session` sets). It covers each site's overview and live stream,
results, slots, decisions (with replay and the counterfactual), drafts, the approve, reject
and comment actions, install probes, and model versions with manual rollback.

## Development

```sh
# Any Postgres where you can create databases; each test gets a fresh one.
export DATABASE_URL=postgres://postgres@localhost:5432/postgres
cargo test                                  # bandit math, migrations, HTTP, worker, scripted cohorts
cd packages/client && npm install && npm test   # the client script, in jsdom
cd packages/agent && npm install && npm test    # the instruction pack installer
npm run mutate                                  # 23 mutants, each must turn that suite red
# Install the pack into a copy of examples/next, then propose a draft through the MCP SDK client:
PRB_BIN=../../target/debug/pratibimba DATABASE_URL=postgres://…/scratch npm run e2e

# The dashboard, in headless Chromium against real Core and a seeded history.
cargo build && cd dashboard && npm install && npx playwright install chromium
npm run build && PRB_E2E_ADMIN_DB=$DATABASE_URL npm run e2e
node e2e/stack.mjs & node e2e/shots.mjs /tmp/prb-shots   # screenshots of every screen, light and dark
```

The dashboard's end-to-end stack (`dashboard/e2e/stack.mjs`) seeds 8,000 visitors with a
planted ground truth in two halves, so the bandit learns from the first and the history shows
it, then runs the real worker. Each test checks the screen against the API it reads from.

The scripted-cohort tests (`core/tests/cohort.rs`) push 2,000 synthetic visitors with a
planted ground truth through the real decide, event and worker loop. They check that the
better variant takes the traffic, and that a returning-versus-new split is learned per
context. A control test runs the same cohort with learning switched off and checks that it
stays near an even split.

## Licence

MIT. See [LICENSE](LICENSE).
