# Adding Pratibimba to an existing site

Use this when the user wants Pratibimba added to a site that already exists and that someone
else probably wrote. The site keeps working as it does today: Pratibimba only changes regions
you mark (**slots**), only to versions a person approved (**variants**), and the page's own
content is always the **baseline**. Change as little of their code as you can, and never
rewrite a component just to make it fit.

Work through the steps in order. Stop and tell the user when a step needs something only they
can do (a token, a decision, an approval).

## 1. Run Core

Core is one Rust binary (`pratibimba serve` and `pratibimba worker`) on Postgres 16 or 17.
From a clone of the Pratibimba repo:

    export PRB_ADMIN_TOKEN=$(openssl rand -hex 32)   # the dashboard password; the user keeps it
    export COMPOSE_FILE=examples/compose.yml
    docker compose up -d                             # first run builds the image
    curl -s localhost:8080/health                    # -> ok

Without Docker: `cargo build --release`, then run `./target/release/pratibimba serve` and
`./target/release/pratibimba worker` with `PRB_DB` (a Postgres URL) and `PRB_ADMIN_TOKEN` set
on both (and `PRB_DASHBOARD_DIR=dashboard/out` after building the dashboard). Both migrate
on start. **The worker must run**: it closes attribution windows and
does all the learning; without it the dashboard fills with decisions and nothing is learned.

The dashboard is `http://localhost:8080/app/`; the user signs in with `PRB_ADMIN_TOKEN`.
Without that variable `/api` answers 503 and nobody can sign in. Never print, commit or ask
for the admin token; the user types it themselves.

## 2. Declare the site in `site.json`

Keep a `site.json` in the user's repo. The fields Core accepts (unknown fields are refused):

- `site`: the id, a lowercase slug (`a-z`, `0-9`, `-`, `_`, at most 64). Public, like an
  analytics key.
- `origin`: the site's URL (the dashboard's install wizard probes it).
- `privacy_mode`: `consent_aware` (the default: a cookie only with consent), `cookieless`
  (a daily-rotating pseudonym, no `returning`) or `full`.
- `currency`: the ISO 4217 code revenue is counted in (a new site gets `USD`).
- `holdout_fraction`: the share of visitors who always get the baseline, 0 to 0.5 (default
  0.1). Leave it at the default unless the user has a reason.
- `fields_allowed`: which optional context fields are stored, from `returning`, `mobile`,
  `from_ad`, `stated`, `tz_offset`, `slot`, `decision_id`, `picks`, `rescore`, `history`.
  Omitted, a new site stores all of them.
- `features`: up to 16 boolean features the page may send, each `{"key": "...", "pii": true|false}`.
  `pii` has no default: ask the user. A `pii: true` feature is used only for an identified
  visitor (`full`, or `consent_aware` with consent) and dropped otherwise.
- `slots`: see step 4. Each slot lists its variants; exactly one is `"baseline": true` and
  carries no `html` or `copy` (sorted slots differ, see step 4).

Optional: `attribution` (`last_touch` or `linear`), `history` (visitor history as features),
`facts` (the only figures copy may claim).

**Put only baselines in the file.** Applying the file approves every variant it lists, as the
owner's own act, so new variants belong in drafts (step 6), not here. Applying also **retires
every approved variant of a listed slot that the file no longer names**, including ones a
person approved in the dashboard. Tell the user this before they re-apply.

The user applies it (re-apply after every edit):

    docker compose run --rm --no-deps -v "$PWD/site.json:/site.json:ro" serve apply /site.json

or `pratibimba apply site.json` with `PRB_DB` set.

## 3. Add the client

For a site that is not Next.js (or not behind a Worker), use the script tag. Two plain tags,
as early in `<head>` as possible, no `async` or `defer`, the manifest first:

    <script>window.__prb_features = { wholesale_buyer: false };</script>
    <script src="/_prb/manifest/SITE.js"></script>
    <script src="/_prb/sdk.js" data-site="SITE"></script>

- `window.__prb_features` is optional and must be set **before** the tags. Only names declared
  in `features` and only boolean values are sent; anything else is ignored.
- `/_prb/*` must reach Core with the `/_prb` prefix stripped, on the site's own origin
  (nginx: `location /_prb/ { proxy_pass http://127.0.0.1:8080/; }`). Then there is no CORS and
  the cookie is first-party. Only when the page calls Core on another origin set
  `PRB_SITE_ORIGIN` on `serve`.
- **Dev server.** Add the same proxy there. In Vite, `server.proxy` (and `preview.proxy` for
  `vite preview`): `'/_prb': { target: 'http://localhost:8080', rewrite: (p) => p.replace(/^\/_prb/, '') }`.
- Tag attributes: `data-timeout` (ms a slot may stay hidden waiting for its decision, default
  400), `data-cookie="on"` (the page says it already has consent), `data-base` (Core's URL, when
  it is not where `sdk.js` came from).

**Consent.** In `consent_aware` mode the SDK reads consent once per page, in this order:
`data-cookie="on"` on the tag, IAB TCF v2 purpose 1 (`__tcfapi`), the latest Google Consent
Mode `analytics_storage` in `window.dataLayer`, OneTrust's `C0002` group. If the site has a
consent platform, make sure it loads before the tags. If it has its own banner, write an
inline script before the tags that reads the site's own consent cookie and pushes
`['consent', 'default', { analytics_storage: 'granted' }]` (or `'denied'`) onto
`window.dataLayer`. A choice made on the page applies from the next page load, not this one.
Never treat a visitor as consented to make the numbers look better.

**Single-page apps.** The script hides only the slot selectors before first paint, asks for
every decision at once, and fills each slot element as it mounts, also when React or Vue mounts
it later (a MutationObserver, before the next paint). An element that mounts before its
decision stays hidden for at most `data-timeout`, then shows the baseline. A remount gets the
same decision. A slot the visitor has already seen, or has the pointer on, is not swapped on
that page. Decisions are made per page load, not per client-side route change.

For Next.js use `@pratibimba/next` (`<prb.Slot>`, `<prb.Sorted>`, `<prb.Copy>`); for a Worker
in front of any origin, `@pratibimba/edge` with `<!-- prb:slot="key" -->…<!-- /prb:slot -->`
comments. Follow the README for those; the rest of this file still applies.

## 4. Choose and mark slots

Start with `npx @pratibimba/agent scan` in the user's repo (it writes nothing without
`--write`). It points; you judge. Pick two or three slots where a visitor makes a choice: a
hero headline and its call to action, a promo line, a product list. Show the user your
picks and why before marking anything.

Pick the kind:

- **`swap`** (default): a variant is an HTML fragment that replaces the element's contents.
  Use for layout changes.
- **`copy`**: named typed fields, written with `textContent` (or `src`/`alt` for images), never
  as markup. Up to 8 fields, text `max` at most 500. Mark each with `data-prb-field="name"`.
  Prefer this for headlines and labels: it is the safest kind inside a framework.
- **`sorted`**: the variants are items (2 to 50); mark each item, a direct child of the slot
  element, with `data-prb-item="key"`. The order in the file is the default order. No item is
  flagged baseline.

Mark the element with a stable attribute, for example `data-prb-slot="hero"`, and give the slot
`"selector": "[data-prb-slot=hero]"`. To the script tag only the `selector` matters; the
attribute is a convention. The baseline is whatever the page already renders there; do not
copy it into the file.

**Never let the framework re-render over a slot.** The script replaces or reorders DOM the
framework thinks it owns. Render a swap slot's contents once and leave them static (in React,
an element with a constant `dangerouslySetInnerHTML`), keep copy fields as plain text in
their `data-prb-field` elements, and do not mark a list that re-renders on filter, sort or
state changes as a sorted slot. If a slot's region re-renders with state, choose another region.

Slot settings: `metric` (the goal event, default `click`), `attribution_window_secs` (default
1800), `decision_ttl_secs` (default 60), `cold_start_exposures` (default 100: equal turns before
the bandit chooses). Keys are lowercase slugs.

## 5. Report outcomes

A click anywhere inside a slot element is sent automatically as `click` with that slot's
decision (and the item for a sorted slot). For other goals, call the SDK where the action
really happens, guarding for a page where the script did not load:

- `window.prb && window.prb.track('signup')`: a goal named by any slot's `metric`. Event names
  are lowercase letters, digits, `_` and `:`.
- `window.prb && window.prb.addToCart({ id: 'sku-1', price: 220, quantity: 1 })`: the
  `add_to_cart` goal. Use the same ids as the `data-prb-item` keys if a sorted slot measures it.
- `window.prb && window.prb.purchase({ order_id, value, currency, items })` where the order is
  confirmed: the store's real `order_id`, `value` in major units (49.99), `currency` the site's
  ISO code, `items` as `[{ id, price?, quantity? }]`. One order is counted once however often it
  is sent. If the store already sends a signed conversion webhook for purchases, do not also
  send `purchase` from the browser.

Bad arguments throw a `TypeError`; never invent an order id or a value.

## 6. Draft variants over MCP

Run `npx @pratibimba/agent install` so the `pratibimba` MCP server (`{{MCP_URL}}`) and its
working guide are in the repo. The user mints a per-site token (dashboard: "Connect your coding
agent", or `pratibimba mcp-token <site> "<agent name>"`) and exports it as `{{TOKEN_ENV}}`; never
write it into a file. Then follow the `pratibimba` skill: read results first, write variants
from the repo's own material, and pass `agent` and `justification` on every call.

`create_slot` adds a slot as a draft; `propose_variant` (swap) and `propose_copy` (copy) add
variants as drafts; `create_audience` adds a draft segment. **There is no approve tool. Never
approve, and never say a draft is live.**

## 7. A person approves, promotes and gates

All of this happens in the dashboard, by the user:

- Approve each draft variant (the approval queue), and a draft slot's baseline to activate it.
- Promote a draft segment to an audience, then gate variants with it (never the baseline).

Audiences are flat predicates (every condition must hold) over a closed list of keys:
`returning`, `mobile`, `from_ad`, `is_night_local`, `is_weekend_local`,
`is_business_hours_local`, `hour_local: [from, to]`, and the visitor-history keys
`days_since_last_purchase`, `purchase_count`, `lifetime_value`, `cart_abandoned`, `viewed`
(history matches identified visitors only, and only when `fields_allowed` admits `history`). **Custom `features` are not audience keys**: they
only feed the model. Demographic keys (`gender`, `age`, `income` and the like) are refused, and
you must not try to infer them some other way.

## 8. Verify with real traffic only

- Check the page in a private window: the slot element gets `data-prb-variant` when it is
  filled, and the dashboard's site page shows decisions arriving. A visitor in the holdout
  always gets the baseline; that is correct, not a bug.
- Use the dashboard's decision page (or `explain_decision`) to see why a visitor got what they
  got; the replay check re-runs the stored scores.
- Results count only decisions whose attribution window has closed. Right after launch every
  `n` can be 0, `learned` is false, and cold start is still giving equal turns. Say so, and do
  not read a winner until an interval against the holdout excludes zero.
- Never send synthetic visits or events to the real site to "test" it. The `simulate` tool runs
  in a sandbox copy for that.

## Pitfalls

- Re-applying `site.json` retires approved variants it does not list (step 2).
- The worker is not running: decisions, but no learning and no results.
- The tags are `async`/`defer` or sit after the app bundle: the slot paints before it is hidden
  and flickers.
- The `/_prb` proxy keeps the prefix, or the dev server has none: every call 404s and the page
  quietly shows baselines.
- A framework re-render wipes the variant, or a stale baseline reappears after state changes.
- A selector that matches nothing, or matches more than the intended element.
- `window.__prb_features` set after the tags, or a feature not declared in `site.json`.
- `currency` in `purchase` differs from the site's (the order is stored but not summed into
  revenue), or `value` is in minor units (4999 for 49.99).
- Treating custom features, or anything demographic, as audience keys.
- Reading early numbers as results.
