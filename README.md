# Pratibimba

Open-source, self-hostable website personalisation for developers. You mark the regions of
your page that may change (slots), declare a few versions of each (variants), and Pratibimba
picks the version each visitor is most likely to act on. It learns from what visitors actually
do, keeps the data on your own Postgres, and can tell you why it showed what it showed.

*Pratibimba* (प्रतिबिम्ब, Sanskrit) means reflection: the page takes the shape of whoever is
looking at it.

> **Status: Phase 0 (walking skeleton).** One Rust binary, Postgres, a per-slot contextual
> bandit and the script-tag install. It has no dashboard, no Next.js package and no MCP server
> yet. Those are the next phases. Expect breaking changes.

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
- **Only approved variants are ever served.** In Phase 0 the variants are the ones in the site
  file you apply yourself. Phase 1 adds the dashboard approval gate.
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
docker run -d -e PRB_DB -p 8080:8080 pratibimba            # serve (the default command)
docker run -d -e PRB_DB pratibimba worker
```

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

### Install on a page: the script fallback

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
the timeout. The server-side installs planned for Phase 1 (`@pratibimba/next` and the edge
worker) decide before any HTML is sent, so they have no flicker at all.

Clicks inside a slot are reported automatically. For other goals, call
`window.prb.track('purchase')`. The subject id lives in `sessionStorage` by default. Add
`data-cookie="on"` to use a one-year first-party `_prb_subject` cookie, which turns on the
`returning` feature, and collect consent before you do.

To try it locally, run `serve` with `PRB_SITE_ORIGIN=http://localhost:5173`, then serve
`examples/plain/` on port 5173.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `PRB_DB` (or `DATABASE_URL`) | (required) | Postgres URL |
| `PRB_BIND` | `0.0.0.0:8080` | listen address for `serve` |
| `PRB_SITE_ORIGIN` | (none) | comma-separated CORS origins; only needed when the page is not same-origin with Core |
| `PRB_WORKER_TICK_MS` | `1000` | how often `worker` closes attribution windows (events wake it immediately) |
| `PRB_DB_MAX_CONNECTIONS` | `10` | pool size |
| `RUST_LOG` | `info` | log filter |

## HTTP API (Phase 0)

| Method and path | What it does |
|---|---|
| `POST /decide` | `{site, slot, subject, ctx: {returning, mobile, from_ad}}` returns `{decision_id, variant, payload, explanation}`. The same subject and slot inside the slot's TTL get the same decision. |
| `POST /e` | `{site, subject, events: [{id, name, slot?, decision_id?}]}`. Accepts up to 100 events per call. `id` is your idempotency key. |
| `GET /manifest/:site` | Slots and approved variant keys. Append `.js` for the script form. Edge-cacheable. |
| `GET /sdk.js` | The client script. |
| `GET /health` | 200 when the database answers. |

## Development

```sh
# Any Postgres where you can create databases; each test gets a fresh one.
export DATABASE_URL=postgres://postgres@localhost:5432/postgres
cargo test                                  # bandit math, migrations, HTTP, worker, scripted cohorts
cd packages/client && npm install && npm test   # the client script, in jsdom
```

The scripted-cohort tests (`core/tests/cohort.rs`) push 2,000 synthetic visitors with a
planted ground truth through the real decide, event and worker loop. They check that the
better variant takes the traffic, and that a returning-versus-new split is learned per
context. A control test runs the same cohort with learning switched off and checks that it
stays near an even split.

## Licence

MIT. See [LICENSE](LICENSE).
