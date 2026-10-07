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
> Phase 2 is in: an open MCP server at `/mcp`, an agent instruction pack, and signed webhooks out.
> Phase 3 backend: the simulator with time-travel, and Stripe and generic conversion webhooks.
> Expect breaking changes.

## How it works

- **Core** is one Rust binary with two jobs. `pratibimba serve` answers `/decide`, `/e` and
  `/manifest/:site`. `pratibimba worker` credits conversions and updates the models. Run both
  against the same Postgres.
- **Decisions** come from a LinTS bandit per slot: Thompson sampling over a Bayesian logistic
  regression, one per variant. It uses three visitor features (`returning`, `mobile`,
  `from_ad`) plus a bias, and, when the visitor's time zone is known, the local clock
  (`sin_hour`, `cos_hour`, `is_night_local`, `is_weekend_local`, `is_business_hours_local`;
  see below). No LLM sits on the decision path. Each decision is stored with its
  features, every variant's sampled score, the policy version and the random seed, so it can be
  replayed exactly.
- **Learning** happens once per decision, and only when the visitor actually saw it. A goal
  event that matches the slot's metric (for example `click`) inside the attribution window
  (default 30 minutes) counts as a conversion. An exposure whose window closes without one
  counts as a miss. A decision that was never shown teaches nothing.
- **Attribution is pluggable.** `"attribution"` in the site file (or on one slot, which
  overrides the site) picks who gets the credit: `last_touch` (the default) gives the whole
  conversion to the decision the event names, or else the visitor's latest exposed one;
  `linear` splits it evenly across every exposed, not-yet-credited decision of that visitor in
  the slot's window. The bandit learns each share as a fractional reward (the same logistic
  update with `y` in [0, 1], so 0 and 1 are exactly a miss and a conversion), and results add
  up shares, so one conversion counts once whichever attributor ran.
  An installed attributor plugin (see [Plugins](#plugins)) overrides the site file's choice;
  a slot that names its own still wins.
- **A holdout keeps it honest.** Each site sends a fixed share of visitors (default 10%,
  `holdout_fraction` in the site file, at most 50%) the baseline regardless. Lift is always the
  policy against that holdout, with a 95% confidence sequence that stays valid however often
  you look.
- **Auto-rollback.** When the top of a variant's confidence sequence sits below the holdout
  by more than the site's margin, the worker pauses it, restores the slot's model to the
  version current when the variant was approved, and writes a sentence saying why. In a
  simulator sandbox it runs on the sandbox's clock after every simulated day.
- **Audiences gate variants.** A segment an owner promotes to an audience can restrict a
  variant (never the baseline): visitors outside it never get that variant, and every
  explanation lists the audiences it evaluated, whether the visitor matched, and what each kept
  out. The gate fails closed: an archived audience, or one that reads a local clock the visitor
  did not report, matches nobody.
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

An optional `"facts"` string is the site's facts.md: what an agent may claim. The grounding
check flags any figure (a number, a percentage, a price) in an agent-written draft that the
facts do not contain; with no facts, agent drafts warn that their claims are unchecked.
Human drafts skip grounding.

Each slot has exactly one `baseline`: the content your page already renders, with no `html`.
Re-applying the file updates the site in place. A variant you remove from the file is retired,
not deleted.

**Cold start.** Until a slot has had `cold_start_exposures` learned-from exposures (per slot,
default 100, `0` turns it off), each visitor gets the eligible variant cold start has served
least, so a new slot's variants get equal turns before the bandit takes over. The explanation
says when cold start chose.

**Declared features.** `"features": [{"key": "loyalty_member", "pii": false}, {"key":
"corp_email", "pii": true}]` lists booleans your page may send in `ctx.features` (up to 16).
`pii` is required: say whether the feature can identify or profile a person. They join the
decision as `feature:<key>` dimensions and are learned. A PII feature is used only for an
identified visitor (see [Privacy](#privacy)); an undeclared or demographic name is refused.

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

See `examples/next` and `examples/edge`. `examples/shop` is a small Next.js store wired to a hosted
Core (`PRB_CORE_URL`, default `https://pratibimbaapi.meetdev.in`) with an add-to-cart and a purchase
goal reported from server actions (`prb.track`), and the agent pack installed. The packages are not on
npm yet: the examples depend on them with `file:` and need `install-links=true` in `.npmrc`, because
Turbopack does not resolve a symlinked package outside the app.

Both send the visitor's consent as `ctx.consent`, read from the `_prb_consent` cookie, IAB TCF
v2's `euconsent-v2` (purpose 1) or OneTrust's `OptanonConsent` (C0002). Pass `consent: true`,
`false` or a function of the request's cookies to decide it yourself.

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
`window.prb.track('purchase')`. Where the subject id lives follows the site's privacy mode (see
[Privacy](#privacy)): `sessionStorage`, or a one-year first-party `_prb_subject` cookie (which
turns on the `returning` feature). On a `consent_aware` site the script uses the cookie only
with consent: `data-cookie="on"` on the tag, IAB TCF v2 purpose 1 from `__tcfapi`, a granted
Google Consent Mode `analytics_storage`, or OneTrust's performance group, in that order.

To send [declared features](#the-site-file), set them before the script tag:
`window.__prb_features = { loyalty_member: true }`. The script sends only names the manifest
declares, only booleans, and a PII one only when the visitor is identified, so it never leaves
the browser otherwise. On a server install, pass `features` to `createPratibimba` (a function of
the request headers) or `createEdge` (a function of the `Request`); the same rule applies, against
the manifest's declared list, and in-session weights carry the features too.

To try it locally, run `serve` with `PRB_SITE_ORIGIN=http://localhost:5173`, then serve
`examples/plain/` on port 5173.

### The taste picker

A `picker` slot asks visitors what they are after and uses the answer. In the site file:

```json
{ "key": "taste", "selector": "#taste", "kind": "picker", "metric": "purchase",
  "picker": { "max_picks": 3, "options": [ { "key": "kitchen", "label": "Kitchen" },
                                            { "key": "garden", "label": "Garden" } ] },
  "variants": [ { "key": "none", "baseline": true },
                { "key": "picker", "html": "<p>What are you shopping for?</p>" } ] }
```

- **It tests itself.** `none` and `picker` are an ordinary two-variant slot. The bandit and
  the holdout judge it on the slot's metric, so a picker that costs sales gets rolled back
  like any losing variant.
- **Picking.** On `picker` the script renders the options plus Done and Skip. A visitor picks
  1 to `max_picks` (1-3). Done sends a `pick` event with the keys. Skip sends `skip` and
  stores nothing.
- **What a pick does.** Every later decision for that visitor on the site carries a `stated`
  feature group, `stated:<key>` = 1 or 0, and the model learns a weight for each key. Without
  cookies a pick lasts for the tab's session. With `data-cookie="on"` it persists.
- **No demographics.** Option keys are your own slugs. Core refuses any key that names an
  attribute of a person (`age`, `gender`, `women` and so on), and refuses `ctx.stated` keys
  that are not one of your options.

### In-session adaptation

Mark a slot `"in_session": true` and the manifest ships each variant's payload and linear
weights. After a signal, the script re-scores those slots on the device: a dot product per
variant, batched into one animation frame. A signal is a pick, or a click on any element with
`data-prb-intent="<option key>"` (worth half a pick, session only). `data-adapt-min` sets how
many signals it waits for (default 1). Once a slot has learned the visitor's local hour, its
weights include the time features; the script computes them from the browser clock and the
site's night window, which the manifest ships as `night`.

**It never swaps what the visitor is touching.** A slot that is in the viewport, holds focus,
or is under the pointer is left alone. So is every slot on a page whose layout the browser
cannot report. Those slots, and slots not on this page, get the new choice on the next page
view instead. Holdout visitors are never re-scored, and a paused site ships no weights.

Each on-device choice is posted as a `rescore` event and recorded as a decision with
`source = 'client'`, so the bandit learns from it. If the client's score and the server's
posterior mean differ by more than 0.15, the weights were stale. That decision is then never
reused as the sticky one, and the server's next full decision wins.

## The dashboard

Core serves it at `/app` (a static Next.js export in `dashboard/`, pointed to by
`PRB_DASHBOARD_DIR`; the Docker image includes it). Sign in with `PRB_ADMIN_TOKEN`.

- **Every site page** opens with a *Live* banner and a kill switch: one click pauses every
  slot on the site (each serves only its baseline, the page's own content) and records who
  pulled it; one click resumes.
- **Site home**: live tiles over server-sent events (decisions in the last minute,
  conversions in the last hour, lift today against the holdout), an install-status strip, and
  the slot list with each slot's champion, 7-day lift and traffic split. The decisions tile
  turns amber when a busy site goes quiet. Below them, *Connect your coding agent* (the MCP
  endpoint, how to mint a token, a config block for Claude Code, Cursor or any MCP client,
  and the instruction-pack command) and a live tail of the site's MCP tool calls.
- **Slot workbench**: a card per variant with its share, its lift and, if auto-rollback paused
  it, why; the cumulative lift curve with its confidence band; and the last 500 decisions,
  filterable by variant and outcome.
- **Decision detail**: one plain-English sentence on why this visitor saw this variant, then
  the features the bandit saw, every arm's score, whether replaying the stored seed
  reproduces them, a counterfactual (what the baseline or the policy would have shown), the
  event timeline and what reviewers wrote at the approval gate.
- **Results**: the site-level lift headline, slots ranked by lift times traffic, and a
  learnings feed written when the evidence crosses a threshold.
- **Approvals**: each draft with its diff against the champion (word by word for an inline
  payload, line by line as a code diff for a component or any multi-line source; switchable),
  and, for a draft an agent wrote over MCP, a badge with the agent's name, who it ran for, its
  justification and a link to the tool call's audit record. Then the constraint checks, a
  planned-traffic slider capped at 50% and a comment box. A failing check blocks approval.
  There is no bulk approve. You can write a draft by hand here too.
- **Install**: the six-step wizard above, for Next.js, the edge worker or the script tag.
  The agent step is done once an MCP tool call has run on the site.
- **Webhooks**: add an endpoint (a URL and the events it wants; the signing secret is shown
  once), see its delivered, pending and failed counts and its recent deliveries, delete it.
- **Audit record** (`/app/audit/?id=<n>`): one MCP tool call: agent, tool, outcome,
  justification, token, the arguments as sent and any draft it wrote.

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
| `create_slot`, `update_slot` | Declares a swap slot whose baseline is a **draft**: it is in no manifest and decides nothing until a person approves that baseline (which activates the slot). `update_slot` edits only such draft slots. |
| `retire_variant` | **Proposes** retiring a live variant. It keeps serving until a person confirms (`POST /api/retire-requests/:id/confirm {actor}`) or declines (`.../decline {actor, comment}`); open proposals are in the approvals API, and the answer on `list_variants`. |
| `list_plugins` | The compiled-in plugin catalogue as the site has it: kind, `deanonymiser`, install, the agent's latest request and its answer, and the site's warnings. |
| `install_plugin` | **Requests** a plugin (`{plugin, config?}`). Nothing is installed until a person confirms (`POST /api/plugin-requests/:id/confirm {actor}`) or declines (`.../decline {actor, comment}`); open requests are in the approvals API (`plugin_requests`). |
| `list_experiments` | The site's experiments with their arms, weights, window, status and per-arm results. Read-only: creating and stopping one is the owner's, in the dashboard API. |
| `list_audiences`, `create_audience` | The site's segments and what each audience gates; a new segment lands as a **draft** that a person promotes. |
| `simulate` | Runs a synthetic cohort (optionally for N simulated days) in the site's **sandbox**, creating it on first use. |
| `fast_forward_visitor` | Moves the sandbox's clock forward (its worker and auto-rollback run) and decides every slot for one subject at the new time. |

- **An agent cannot approve.** There is no approve tool, `propose_variant` writes `draft`
  whatever it is sent (a `status` or `traffic_share` argument fails the call), and an MCP
  token is not accepted by `/api`. A person approves in the dashboard. The same floor holds for
  slots (an agent's slot stays dark until a person approves its baseline, and its variants
  wait for that) and for retirement (a proposal, never a retirement).
- **Every call is audited.** Each tool takes `agent` and `justification` (and optionally
  `on_behalf_of`); a call without them is refused. Every call, refused or not, writes a row
  to `mcp_calls`, and each result returns that row as `audit`. A draft links to the call that
  wrote it, so the approvals API shows the agent and its justification.
- **The simulator tools are sandbox-only.** `simulate` and `fast_forward_visitor` take no
  site argument; the sandbox id comes from the token's site, so no call can reach real
  decisions, events or models. The sandbox has one clock, so fast-forwarding one visitor moves
  every simulated visitor with it.

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

## The simulator and time-travel

Test personalisation locally without waiting for real visitors. The simulator copies a site
into a **sandbox**, `<site>~sim`, sends it synthetic visitors with a conversion rate you plant
per variant, and moves the sandbox's own clock forward, so "skip 30 days" runs thirty days of
traffic and learning in seconds.

```sh
pratibimba simulate demo --days 30 --visitors 200 --rate hero.gift=0.12 --rate hero.baseline=0.04
# day   1  2026-10-06  conv    9  hero: baseline 55%, gift 45%
# day   2  2026-10-07  conv   14  hero: baseline 18%, gift 82%
# ...
pratibimba simulate demo --days 7 --visitors 0 --keep   # just move the clock a week
```

Options: `--days`, `--visitors` (per day; `0` only moves the clock), `--rate slot.variant=p`
(repeatable), `--default-rate`, `--returning` (fraction of returning visitors), `--seed`, and
`--keep` to continue the sandbox instead of resetting it. The same engine is on the admin API:

| Method and path | What it does |
|---|---|
| `POST /api/sites/:site/sim` | Create or wipe the sandbox. `{at?}` sets its clock (default now). |
| `GET /api/sites/:site/sim` | The clock, decision and conversion counts, and results as of the clock. |
| `POST /api/sites/:site/sim/visitors` | One cohort at the current clock: `{visitors, seed, rates: {slot: {variant: p}}, returning_rates?, default_rate?, returning?, mobile?, from_ad?}`. |
| `POST /api/sites/:site/sim/advance` | `{days \| seconds, cohort?}`. With a cohort, it arrives once per simulated day. |
| `DELETE /api/sites/:site/sim` | Drop the sandbox and everything simulated in it. |

**Simulated traffic never reaches the real site.** The sandbox is its own site row with its
own slots, variants and models, copied from the real site's configuration; it starts with a
fresh model. Every simulated decision, event and model update is keyed by the sandbox's ids,
and results and models are read per site and per slot, so nothing can mix. A sandbox id is not
a valid site slug, so no real site can be named like one. The real worker never touches
sandbox rows; each sandbox learns in its own worker pass at its own clock. `/decide` and `/e`
stamp a sandbox's rows with its clock, so you can also point a local SDK at `<site>~sim` and
see what the simulated month taught it. Sandboxes are left out of the site list.

## Conversion webhooks

For checkouts you control, the platform is the source of truth for a sale, not the browser.

**Stripe.** Add an endpoint in Stripe for `checkout.session.completed` (and
`checkout.session.async_payment_succeeded` if you take delayed payment methods) pointing at
`https://<core>/webhook/stripe/<site>`. When you create the Checkout Session, set
`client_reference_id` to the visitor's Pratibimba subject, and optionally
`metadata.prb_decision_id` (the decision to credit) and `metadata.prb_event` (the goal,
default `purchase`). Core verifies the `Stripe-Signature` header with your endpoint's signing
secret (HMAC-SHA256 over `t.body`, five-minute tolerance), counts only paid sessions, and
answers 200 to events it ignores so Stripe stops retrying.

**Anything else.** `POST /webhook/conversion/<site>` with
`{id, subject, event?, decision_id?, occurred_at?}`, signed exactly like Stripe: header
`Pratibimba-Signature: t=<unix>,v1=<hex HMAC-SHA256(secret, "<t>.<raw body>")>`. `id` is
your order or conversion id.

Configure both with the admin API (secrets are write-only):

```sh
curl -X POST https://<core>/api/sites/demo/conversion-sources -H "Authorization: Bearer $PRB_ADMIN_TOKEN" \
  -d '{"stripe_signing_secret": "whsec_…", "conversion_signing_secret": "…", "goals": ["purchase"], "grace_secs": 3600}'
```

The rules:

- **Duplicates count once.** A webhook's event id is derived from the site, the source and the
  platform's id (the checkout session, your order id), so a retry, a resend, or the same
  checkout reported by two Stripe event types is absorbed.
- **Platform conversions are authoritative.** For a goal listed in `goals`, a browser event of
  that name is a hint, never a conversion: only a webhook credits it. When the webhook names
  no decision, it credits the decision the visitor's latest browser hint named, else the last
  touch. A decision on such a goal waits `grace_secs` past its attribution window before it
  settles as a miss, so a late webhook still lands; the webhook is judged at the time the
  platform says the sale happened.
- **A late webhook corrects a settled miss.** If the webhook arrives after the decision
  settled as a miss (past the grace period) but the sale happened inside the attribution
  window, the decision flips to converted and records `corrected_at`, the `conversion` webhook
  fires with `late: true`, and the model is corrected exactly: turning a miss into a
  conversion adds `w.x` to the log-likelihood, so the posterior mean moves by `cov.x` and the
  covariance is unchanged. Once per decision, never for a browser event, and a holdout
  decision's record is corrected without teaching the model.
- A request with a missing, wrong or stale signature gets 400 and writes nothing.

## Privacy

Each site has a **privacy mode**, set with `"privacy_mode"` in the site file or
`POST /api/sites/:id/privacy`:

| Mode | What Core stores and uses |
|---|---|
| `consent_aware` (default for a new site) | `full` for a request with `consent: true` (`ctx.consent` on decide), `cookieless` otherwise |
| `cookieless` | a pseudonym of the subject that changes every UTC day (the day's salt is deleted when the next is made), and no `returning` |
| `full` | the subject as the client sent it, and every feature |

Sites that existed before this release were moved to `full`, which is what they did before.

**PII features.** A [declared feature](#the-site-file) tagged `pii: true` is dropped before
scoring and storage in `cookieless` mode and in `consent_aware` mode without consent; the
script does not send it in those cases at all.

**Minimum collection.** `"fields_allowed"` lists the optional fields a site collects: any of
`returning`, `mobile`, `from_ad`, `stated`, `tz_offset` (also covers `ctx.tz`), `slot`,
`decision_id`, `picks`, `rescore`. Anything else is dropped at `/decide` and `/e` before it is
written. With no list, every field is collected. Undeclared names (`gender`) are refused as
always.

**Right to erase.** `pratibimba erase <site> <subject> --by <who> [--reason ...]`, or
`POST /api/sites/:id/erase {subject, actor, reason?}`, hides the subject's events from every
read at once and schedules their deletion, which the worker carries out a day later
(`PRB_ERASE_PURGE_AFTER_SECS`, at most 30 days; `0` means its next pass). It deletes the visitor
row and retention weeks and removes the subject from webhook payloads on the spot. Its decisions stay, with their
variant, features and outcome, under a random token, so results and rollups still add up. Any
open decision settles without teaching the model. Only a hash of the subject is kept, as a
tombstone: later requests with that subject get `410 Gone`, and its platform conversions are
ignored. Each erase writes an audit row (`GET /api/sites/:id/erasures`,
`pratibimba erasures <site>`) whose `events_removal` says whether the deletion is pending or done. What a model has already learned stays, because a model's state
is an aggregate. `GET /api/sites/:id/subjects/:subject` lists everything held for a subject.

**Visitors.** Core keeps one `visitors` row per stored subject: when it was first and last seen,
an event count and the last 20 event names.

## Backups

`pratibimba backup [dir-or-file]` writes a timestamped plain-SQL dump
(`pratibimba-backup-20261007T031500Z.sql`) with `pg_dump`. It never overwrites a file, and it
leaves out the privacy salts, so a backup cannot link one day's cookieless visitors to the next.
`pratibimba restore <file>` loads a dump into an **empty** database (`PRB_DB`) in one
transaction. The Docker image includes `pg_dump` and `psql` (Debian's 17, so the server must be
17 or older; `backup` checks this first and names both versions when the server is newer). `examples/compose.yml` runs a backup once a day.

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
| `PRB_PG_BIN` | (`PATH`) | the directory holding `pg_dump` and `psql`, for `backup` and `restore` |
| `RUST_LOG` | `info` | log filter |

## HTTP API

| Method and path | What it does |
|---|---|
| `POST /decide` | `{site, slot, subject, ctx: {returning, mobile, from_ad, stated?, tz_offset?, tz?, consent?, features?}}` returns `{decision_id, variant, payload, explanation}`. The same subject and slot inside the slot's TTL get the same decision. `explanation.audiences` lists the audiences evaluated. |
| `POST /e` | `{site, subject, events: [{id, name, slot?, decision_id?}], tz_offset?}`. Accepts up to 100 events per call. A `pick` adds `picks: [option keys]`. A `rescore` adds `variant`, `features` and `score`. `id` is your idempotency key. `tz_offset` is the visitor's UTC offset in minutes east (`-new Date().getTimezoneOffset()`, -720 to 840); the script sends it. |
| `GET /manifest/:site` | Slots and approved variant keys. Append `.js` for the script form. Edge-cacheable. |
| `GET /sdk.js` | The client script. |
| `POST /decide/batch` | Several slots for one subject in one call: `{site, subject, slots, ctx}`. |
| `GET /health` | 200 when the database answers. |
| `POST /mcp` | The MCP server (JSON-RPC), behind a per-site `prb_mcp_…` token. See above. |
| `POST /webhook/stripe/:site` | Stripe Checkout conversions, verified with the site's signing secret. See above. |
| `POST /webhook/conversion/:site` | Generic signed conversions. See above. |

The dashboard API lives under `/api` and needs `Authorization: Bearer $PRB_ADMIN_TOKEN` (or the
session cookie that `POST /api/session` sets). It covers each site's overview and live stream,
results, slots, decisions (with replay and the counterfactual), drafts, the approve, reject
and comment actions, install probes, model versions with manual rollback, the simulator,
conversion sources and privacy (mode, fields, erase, the erasure log, subject access).
`POST /api/variants/:id/audience {audience: <segment id> | null, actor}`
restricts a variant to a promoted audience, or lifts the restriction. 409 while the variant is an arm of an experiment that has not ended.

### Visitor insights

The worker rolls exposed decisions up into `rollup_hourly` (keyed on UTC hour, local hour and
weekday, segment, slot, variant, context and outcome), `rollup_daily`, `rollup_journey` and
`rollup_retention`, every minute, with retention and segment discovery once a night. The
insight routes read **only** those rollups, never `events` or `decisions`; a test renames both
raw tables away and calls every route. Hourly and daily rollups are partitioned by month.

| Method and path (under `/api`) | What it does |
|---|---|
| `GET /sites/:site/insights/heatmap?axis=local\|utc&by=hour\|dow_hour&segment=&slot=&days=28` | Exposures, conversions and rate per hour (and weekday) per variant. `tz_known_share` says how much of the local axis rests on a reported offset. |
| `GET /sites/:site/insights/journeys?outcome=converted\|bounced&days=28&limit=10` | The top (slot, variant) paths per visitor-day that ended in a conversion, or in none. |
| `GET /sites/:site/insights/retention?segment=&weeks=12` | Cohort retention: per first-seen week, its size and how many came back n weeks later. |
| `GET /sites/:site/insights/status` | When each insight job last ran, and any segment backfill still in progress. |
| `GET /sites/:site/segments` | Segments with their last-28-day numbers; discovered drafts carry their evidence. |
| `POST /sites/:site/segments` | `{predicate, actor, name?}`: a hand-authored segment, active at once, backfilled in day chunks. |
| `POST /segments/:id/promote` | `{actor}`: a draft or active segment becomes an audience, with the promoter recorded. |
| `POST /segments/:id/archive` | `{actor}`: drop a segment from the list and the rollups. |

A predicate is a conjunction over `returning`, `mobile`, `from_ad`, `is_night_local`,
`is_weekend_local`, `is_business_hours_local` (booleans) and `hour_local: [from, to]`.
Demographic names (`gender`, `age`, `income`, ...) and anything else are refused. Segment 0 is
everyone. Discovery scores every one- and two-feature predicate over the last 28 days of
rollups, keeps those whose conversion rate, or whose variant lift over the baseline, differs
from everyone else's at z >= 4, and writes them as drafts. Nothing is used in a decision until
an owner promotes it.

**Local time comes from the browser's offset, not IP geolocation.** Core ships no GeoIP
database, and behind the `/_prb/*` proxy the IP it sees is often the proxy's. The offset is
coarse, DST-correct at the moment of the event, and spoofing it only mislabels the spoofer's
own hour. An event with no offset counts as UTC. The night window is per site
(`sites.night_start_hour` / `night_end_hour`, default 22:00-05:00). Raw events are not pruned.

**The local clock is also a decision feature.** The script sends `ctx.tz_offset` on
`/decide`; with it, the decision carries `sin_hour`, `cos_hour` (so 23:00 sits next to 00:00),
`is_night_local`, `is_weekend_local` and `is_business_hours_local` (Monday-Friday,
09:00-17:00), and the bandit learns whether they predict a variant's lift. No
offset, no time features: an unknown clock is never guessed as UTC on the decision path. The
time features are extra named dimensions after the base four (beside any `stated` tastes), and
the list of names is stored with the model and only appended to, so a model saved before them
keeps everything it learned and gains the new dimensions at the prior (weekend and business
hours joined this way after the first three).

**Server-side SDKs send a time zone name.** A server render has no browser clock, so
`@pratibimba/next` and `@pratibimba/edge` send `ctx.tz`, an IANA zone such as `Asia/Kolkata`.
They prefer the zone the browser names, in `_prb_tz`, a first-party cookie their beacon sets
(a session cookie, or a year with consent), because IP geolocation is wrong behind many
carriers and every VPN. Without it they fall back to the platform's geolocation
(`x-vercel-ip-timezone`, Cloudflare's `request.cf.timezone` or `cf-timezone`,
`cloudfront-viewer-time-zone`). Off those platforms a geo
header is just something the visitor sent; pass `timeZoneHeaders: []` to ignore it. Core
resolves the zone at the site's own clock (so daylight saving and a sandbox's time both hold)
and prefers it over `tz_offset`; an unknown zone is refused with 400.

## Webhooks out

Core POSTs six event types to the endpoints a site subscribes: `decision`, `exposure` (the
first one per decision), `conversion` (a credited conversion), `variant.approved`,
`variant.retired` and `auto_rollback`. An endpoint gets only the types it asked for.

```json
{ "id": "…", "idempotency_key": "…", "type": "conversion", "site": "demo",
  "created_at": "…", "data": { "decision_id": "…", "slot": "hero", "variant": "gift", "metric": "click", … } }
```

- **Signed.** `Pratibimba-Signature: t=<unix seconds>,v1=<hex>`, where `<hex>` is
  HMAC-SHA256 of `"<t>.<raw body>"` keyed with the endpoint's secret. Verify it on the raw
  body and reject an old `t`. `Pratibimba-Event` and `Pratibimba-Idempotency-Key` are also sent.
- **Never lost, never invented.** Each event is written to an outbox in the same transaction
  as the change it reports. The worker sends it.
- **Retried.** Any 2xx is delivered. Anything else (a non-2xx, a timeout, a refused
  connection) is retried after 10 s, doubling to at most an hour; after 8 attempts it is
  `failed`. A retry keeps its idempotency key, so drop repeats on your side.
- **Recorded.** Each delivery keeps its status, attempts and last response. Delivered rows
  are pruned after 7 days.

| Method and path (admin token) | What it does |
|---|---|
| `GET /api/sites/:site/webhooks` | Endpoints with delivery counts, and the event types. |
| `POST /api/sites/:site/webhooks` | `{url, events}`. Returns the endpoint and its `secret`, the only time it is shown. |
| `DELETE /api/webhooks/:id` | Removes the endpoint and its pending deliveries. |
| `GET /api/webhooks/:id/deliveries` | The last 50 delivery records. |

## Plugins

Plugins are compiled into Core: named extensions of known kinds, chosen per site. There is no
dynamic code loading, so a plugin is a build of Core, not a file you drop in. This build has:

| Plugin | Kind | De-anonymiser | What installing it does |
|---|---|---|---|
| `attribution.linear` | `attributor` | no | The site's attributor becomes `linear` (see How it works). A slot that names its own attributor keeps it. Uninstall returns to the site file's choice. No config. |
| `sink.identity_resolver` | `event_sink` | **yes** | Forwards every `exposure` and `conversion` (subject id included) to `config.url`, as a signed webhook endpoint the plugin owns. Uninstall deletes the endpoint. |

Core ships no de-anonymiser switched on. While a plugin flagged `deanonymiser` is installed, the
site carries a warning: `warnings` on the plugin listing and on the overview, shown as a plain
banner on every dashboard page. Uninstalling clears it.

| Method and path (admin token) | What it does |
|---|---|
| `GET /api/sites/:site/plugins` | Every plugin with its kind, `deanonymiser` flag and install (`{config, by, at}` or null), plus the site's `warnings`. |
| `POST /api/plugins/:name/install` | `{site, actor, config?}`. 201 with the install; an event sink also returns its signing `secret`, once. Unknown keys in `config` are refused; installing twice is 409. |
| `POST /api/plugins/:name/uninstall` | `{site, actor}`. Removes the install and anything it owned. |
| `POST /api/plugin-requests/:id/confirm` | `{actor}`. Installs what an agent requested over MCP, with its config. `.../decline {actor, comment}` refuses it. |

## Experiments

The bandit optimises live. When you want a formal answer instead, run an experiment: a
fixed-allocation A/B test laid over one slot, with two or more of its approved variants as arms,
weights, a start, a stop and the slot's own metric as the primary metric. One experiment per slot
at a time.

While it runs, each visitor gets the arm a hash of (experiment, subject) lands on, in proportion
to the weights, so a visitor keeps their arm across visits. The decision says so
(`explanation.allocation: "experiment"` and `explanation.experiment`). The other gates come
first, as always: a paused site serves its baseline, the holdout gets the baseline, and an arm
a planned traffic share closes to a visitor is skipped (with every arm closed, the bandit
decides). Stopping it, or reaching `stops_at`, hands the slot back to the bandit on the next
decision.

Arms carry no audiences: an audience closes an arm by who the visitor is, so its lift would compare different people, and refusing that at `POST /experiments` (and refusing an audience on an arm until the experiment ends) is simpler than restricting every result to visitors eligible for every arm, which would need each decision's eligibility recorded and would throw away the rest of the traffic; a traffic share closes arms by a hash of the subject, so it leaves the arms comparable.

A simulator sandbox copies the experiments production has not ended, windows and all, so it decides as production does; they start, stop and can be stopped on the sandbox's clock, and a stop there touches only the copy.

The bandit learns from experiment traffic: LinTS fits each arm's outcome against the context, and an assignment by a hash of the visitor is independent of the outcome, so the fit stays unbiased and the test's visitors are not wasted.

Results count the experiment's own assignments over closed attribution windows: per arm, the
conversion rate and its lift over the control (the baseline if it is an arm, else the first
arm), each with the same anytime-valid confidence sequences as the rest of the dashboard, so you
may look as often as you like.

| Method and path (admin token) | What it does |
|---|---|
| `GET /api/sites/:site/experiments` | Every experiment, newest first: arms and weights, `control`, `starts_at`, `stops_at`, `status` (`scheduled`, `running`, `stopped`) and `results.arms[]` with `counts`, `rate` and `lift`. |
| `POST /api/experiments` | `{site, slot, name, actor, arms: [{variant, weight}], stops_at, starts_at?, metric?}`. 201. Refuses fewer than two arms, a variant that is not approved on the slot, a repeated arm, a weight that is not positive, a stop not after the start or already past, a metric other than the slot's, an arm gated by an audience, and (409) a window that overlaps another experiment on the slot. |
| `POST /api/experiments/:id/stop` | `{actor}`. Ends it now; 409 if it has already stopped. |

## Development

```sh
# Any Postgres where you can create databases; each test gets a fresh one.
export DATABASE_URL=postgres://postgres@localhost:5432/postgres
cargo test                                  # bandit math, migrations, HTTP, worker, scripted cohorts
cd packages/client && npm install && npm test   # the client script, in jsdom
cd packages/agent && npm install && npm test    # the instruction pack installer
npm run mutate                                  # 27 mutants, each must turn that suite red
# Install the pack into a copy of examples/next, then propose a draft through the MCP SDK client:
PRB_BIN=../../target/debug/pratibimba DATABASE_URL=postgres://…/scratch npm run e2e

# next and edge: unit suites, then the local-time path end to end against real Core + worker
cd packages/next && npm install && npm test && npm run e2e    # needs cargo build first
cd packages/edge && npm install && npm test && npm run e2e

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
