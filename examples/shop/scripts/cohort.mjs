// A scripted cohort for examples/shop: real HTTP renders of the home page (so the hero slot
// is decided by @pratibimba/next inside the Next server, exactly as for a browser), then the
// beacon's own calls (exposure, click) to the store's /_prb/e door. No browser: the script
// does what PrbBeacon does, so it can run thousands of visits.
//
//   cd examples/shop && npm run build
//   PRB_TIMEOUT_MS=1000 PRB_PERSIST=1 npx next start -p 3202
//   node scripts/cohort.mjs --base http://localhost:3202 --plan full --out scripts/out/run.jsonl
//
// Every visit is one JSON line in --out (gitignored): who it was, what the server could see,
// what it rendered, the planted click probabilities, and what was sent back. The first line
// is the run header with the ground truth below, so a log is self-describing.
//
// GROUND TRUTH (planted; the click probability of the hero, per exposure):
//   baseline       5%, or 15% for a returning visitor (regulars like the familiar hero)
//   filter-angle   5%, or 16% for a visitor from an ad (utm_*: the filter-coffee campaign)
//   price-upfront  4% on desktop, 20% on mobile
//   tasting-notes  4% on mobile; on desktop 22% in the local evening (17:00-23:59), else 7%
// So the winner is: returning -> baseline; from an ad -> filter-angle (unless mobile, where
// price-upfront's 20% beats 16%); mobile -> price-upfront; desktop evening -> tasting-notes.
// The hero's metric is `click`, so a click is the conversion.
//
// GRID (F2): the product grid is the sorted slot `grid`. The visitor scans the cards in the
// order the page rendered them and clicks the first card that passes its GRID_TRUTH draw (see
// truth.mjs), or none. The exposure names the items the beacon names (the decision's order);
// a click names its card. analyze-grid.mjs reconciles both against Core's per-item counts.
//
// ORDERS (F1): after a served hero, a visitor buys with probability BUY (more after a click),
// through the store's own /checkout route (prb.purchase in @pratibimba/next): one product, 1-3
// bags, INR. A RETRY share of orders is sent twice with the same order_id, which Core must count
// once. analyze.mjs reconciles every order's value against the revenue Core credited.
//
// What the server can see: mobile (user agent), from_ad (utm_* on the URL), returning (a
// subject cookie with consent: the site is consent_aware and the store runs PRB_PERSIST=1),
// and the local clock through the `_prb_tz` cookie the beacon would have set. A referrer is
// sent for some organic visits and should change nothing: Core has no referrer feature.
// Weekend is not reachable: the run happens on a weekday everywhere (see the tracker).

import { appendFileSync, mkdirSync } from 'node:fs';
import { dirname } from 'node:path';
import { randomUUID } from 'node:crypto';
import { GRID_TRUTH, TRUTH_TABLE } from './truth.mjs';

const arg = (name, dflt) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : dflt;
};
const BASE = arg('base', 'http://localhost:3202');
const OUT = arg('out', `scripts/out/cohort-${Date.now()}.jsonl`);
const PLAN = arg('plan', 'full');
const SEED = Number(arg('seed', '7'));

const TRUTH = TRUTH_TABLE;
const BUY = Number(arg('buy', '0.06'));
const BUY_AFTER_CLICK = Number(arg('buy-after-click', '0.35'));
const RETRY = 0.25;
const PRODUCTS = [{ slug: 'monday-espresso', price: 549 }, { slug: 'estate-filter', price: 699 }];

// Phases: `rate` visits per second (Poisson), or `gap` [min, max] seconds between visits
// (the sparse phases, to see the first render after an idle keep-alive socket).
const PLANS = {
  smoke: [{ name: 'smoke', rate: 1, secs: 20 }],
  // The local demo (demo-record.mjs): 90 s of steady traffic against the local stack.
  demo: [{ name: 'demo', rate: 4, secs: 90 }],
  // E5 (2026-10-08): one steady block, then estimates.mjs while the windows close.
  short: [{ name: 'steady', rate: 2.5, secs: 1200 }],
  // EC3 (2026-10-13): four minutes through the events queue, to reconcile against Core.
  ec3: [{ name: 'steady', rate: 2.5, secs: 240 }],
  // F1 (2026-10-14): five minutes with orders, to reconcile revenue against Core.
  f1: [{ name: 'steady', rate: 2.5, secs: 300 }],
  // F2 (2026-10-14): 45 minutes, so the first windows (1800 s) close and learning shows in the
  // orders served in the last quarter hour. Run with --buy 0: the grid's metric is click.
  f2: [{ name: 'steady', rate: 1.5, secs: 2700 }],
  // Only idle gaps: every render comes 4.5-9 s after the last one.
  sparse: [{ name: 'sparse', gap: [4.5, 9], secs: 300 }],
  full: [
    ...Array.from({ length: 4 }, (_, i) => [
      { name: `steady${i + 1}`, rate: 2.5, secs: 480 },
      { name: `sparse${i + 1}`, gap: [3, 9], secs: 150 },
    ]).flat(),
  ],
};

// A small seeded PRNG, so a run's visitor mix can be reproduced.
let s = SEED >>> 0;
const rnd = () => ((s = (s + 0x6d2b79f5) >>> 0), ((t) => ((t = Math.imul(t ^ (t >>> 15), t | 1)), (t ^= t + Math.imul(t ^ (t >>> 7), t | 61)), ((t ^ (t >>> 14)) >>> 0) / 4294967296))(s));
const pick = (xs) => xs[Math.floor(rnd() * xs.length)];

const ZONES = ['Asia/Kolkata', 'Europe/London', 'Europe/Berlin', 'America/New_York', 'America/Los_Angeles', 'Asia/Tokyo', 'Australia/Sydney', 'America/Sao_Paulo', 'Asia/Dubai'];
const UA = {
  mobile: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1',
  desktop: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 14_6) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Safari/537.36',
};
const UTM = ['utm_source=google&utm_medium=cpc&utm_campaign=filter', 'utm_source=instagram&utm_medium=paid', 'gclid=abc123'];

function localClock(tz, at = new Date()) {
  const parts = Object.fromEntries(new Intl.DateTimeFormat('en-US', { timeZone: tz, hour: 'numeric', hourCycle: 'h23', weekday: 'short' }).formatToParts(at).map((p) => [p.type, p.value]));
  const hour = Number(parts.hour);
  const asUtc = new Date(at.toLocaleString('en-US', { timeZone: 'UTC' }));
  const asTz = new Date(at.toLocaleString('en-US', { timeZone: tz }));
  return { hour, weekday: parts.weekday, offset: Math.round((asTz - asUtc) / 60000) };
}

const returners = []; // consented visitors who may come back: {subject, mobile, tz, sendsTz, last}

function newVisitor() {
  const tz = pick(ZONES);
  return { subject: null, mobile: rnd() < 0.5, tz, sendsTz: rnd() < 0.85, consent: rnd() < 0.4, from_ad: rnd() < 0.3, returning: false };
}

function nextVisitor(now) {
  const ready = returners.filter((r) => now - r.last > 75_000); // past the 60 s sticky window
  if (ready.length && rnd() < 0.15) {
    const r = pick(ready);
    r.last = now;
    return { ...r, consent: true, from_ad: rnd() < 0.1, returning: true, ref: r };
  }
  return newVisitor();
}

function cookieOf(res, name) {
  for (const c of res.headers.getSetCookie?.() ?? []) {
    const m = new RegExp(`^${name}=([^;]*)`).exec(c);
    if (m) return decodeURIComponent(m[1]);
  }
  return null;
}

async function postEvent(subject, v, decision, name, clock, slot = 'hero', items) {
  const ev = { id: randomUUID(), name, slot, decision_id: decision, ...(items?.length ? { items: items.map((id) => ({ id })) } : {}) };
  const body = { site: 'shop', subject, events: [ev], tz_offset: clock.offset, consent: v.consent === true };
  try {
    const r = await fetch(`${BASE}/_prb/e`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
    const j = await r.json().catch(() => null);
    // `accepted` when Core wrote it directly; `queued` when the edge put it on the events queue (EC3).
    return { status: r.status, accepted: j?.accepted ?? null, queued: j?.queued ?? null };
  } catch (e) {
    return { status: 0, error: String(e) };
  }
}

async function checkout(subject, v, order) {
  const cookies = [`_prb_subject=${encodeURIComponent(subject)}`];
  if (v.consent) cookies.push('_prb_consent=granted');
  const form = new URLSearchParams({ slug: order.slug, quantity: String(order.quantity), order_id: order.order_id });
  try {
    const r = await fetch(`${BASE}/checkout`, { method: 'POST', redirect: 'manual', body: form,
      headers: { cookie: cookies.join('; '), 'user-agent': v.mobile ? UA.mobile : UA.desktop } });
    return { status: r.status, at: new Date().toISOString() };
  } catch (e) {
    return { status: 0, error: String(e) };
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let n = 0;
let lastStart = 0;
const pending = new Set();

async function visit(phase) {
  const i = n++;
  const started = Date.now();
  const gap = lastStart ? started - lastStart : null;
  lastStart = started;
  const v = nextVisitor(started);
  const clock = localClock(v.tz);
  const ctx = { mobile: v.mobile, from_ad: v.from_ad, returning: v.returning, evening: clock.hour >= 17 };
  const cookies = [];
  if (v.returning) cookies.push(`_prb_subject=${encodeURIComponent(v.subject)}`);
  if (v.consent) cookies.push('_prb_consent=granted');
  if (v.sendsTz) cookies.push(`_prb_tz=${encodeURIComponent(v.tz)}`);
  const qs = v.from_ad ? `?${pick(UTM)}` : '';
  const headers = { 'user-agent': v.mobile ? UA.mobile : UA.desktop };
  if (cookies.length) headers.cookie = cookies.join('; ');
  if (!v.from_ad && rnd() < 0.3) headers.referer = 'https://www.google.com/';
  const rec = { i, phase, at: new Date(started).toISOString(), gap_ms: gap, ctx, tz: v.tz, sends_tz: v.sendsTz, consent: v.consent, local_hour: clock.hour, weekday: clock.weekday, referer: headers.referer ?? null, url: `/${qs}` };
  try {
    const res = await fetch(`${BASE}/${qs}`, { headers, redirect: 'manual' });
    const html = await res.text();
    rec.status = res.status;
    rec.render_ms = Date.now() - started;
    const subject = cookieOf(res, '_prb_subject') ?? v.subject;
    rec.subject = subject;
    const m = /data-prb-slot="hero" data-prb-variant="([^"]+)" data-prb-decision="([^"]+)"/.exec(html);
    rec.served = !!m;
    if (m) {
      rec.variant = m[1];
      rec.decision = m[2];
      rec.p = TRUTH[m[1]](ctx);
      rec.exposure = await postEvent(subject, v, m[2], 'exposure', clock);
      rec.clicked = rnd() < rec.p;
      if (rec.clicked) {
        await sleep(300 + rnd() * 2000);
        rec.click = await postEvent(subject, v, m[2], 'click', clock);
      }
      if (rnd() < (rec.clicked ? BUY_AFTER_CLICK : BUY)) {
        await sleep(500 + rnd() * 3000);
        const p = pick(PRODUCTS);
        const quantity = 1 + Math.floor(rnd() * 3);
        rec.order = { order_id: randomUUID(), slug: p.slug, quantity, value: p.price * quantity, currency: 'INR' };
        rec.order.sent = [await checkout(subject, v, rec.order)];
        if (rnd() < RETRY) rec.order.sent.push(await checkout(subject, v, rec.order));
      }
    }
    const g = /data-prb-slot="grid" data-prb-variant="([^"]+)" data-prb-decision="([^"]+)"/.exec(html);
    if (g) {
      // What the beacon would report: its `items` prop (the decision's order) from the flight
      // data, and the cards in page order (unplaced items trail the ordered ones).
      const esc = g[2].replace(/-/g, '\\-');
      const props = new RegExp(`decision\\\\":\\\\"${esc}\\\\"[^\\]]*?items\\\\":\\[([^\\]]*)\\]`).exec(html);
      const shown = props ? [...props[1].matchAll(/([a-z0-9_-]+)\\"/g)].map((x) => x[1]) : [];
      const start = html.indexOf(g[0]);
      const rendered = [...html.slice(start).matchAll(/data-prb-item="([^"]+)"/g)].map((x) => x[1]).filter((k, j, a) => a.indexOf(k) === j);
      rec.grid = { decision: g[2], top: g[1], shown, rendered };
      rec.grid.exposure = await postEvent(subject, v, g[2], 'exposure', clock, 'grid', shown);
      for (const k of rendered) {
        if (rnd() < GRID_TRUTH[k](ctx)) {
          rec.grid.clicked = k;
          await sleep(300 + rnd() * 1500);
          rec.grid.click = await postEvent(subject, v, g[2], 'click', clock, 'grid', [k]);
          break;
        }
      }
    }
    if (!v.returning && v.consent) returners.push({ subject, mobile: v.mobile, tz: v.tz, sendsTz: v.sendsTz, last: started });
    else if (v.ref) v.ref.subject = subject;
  } catch (e) {
    rec.error = String(e);
  }
  appendFileSync(OUT, JSON.stringify(rec) + '\n');
}

mkdirSync(dirname(OUT), { recursive: true });
appendFileSync(OUT, JSON.stringify({ header: true, base: BASE, plan: PLAN, seed: SEED, started: new Date().toISOString(), truth: Object.fromEntries(Object.entries(TRUTH).map(([k, f]) => [k, f.toString()])) }) + '\n');
for (const ph of PLANS[PLAN]) {
  const end = Date.now() + ph.secs * 1000;
  console.log(new Date().toISOString(), 'phase', ph.name, 'visits so far', n);
  while (Date.now() < end) {
    const p = visit(ph.name);
    pending.add(p);
    p.finally(() => pending.delete(p));
    const wait = ph.rate ? -Math.log(1 - rnd()) / ph.rate : ph.gap[0] + rnd() * (ph.gap[1] - ph.gap[0]);
    await sleep(wait * 1000);
  }
}
await Promise.all(pending);
console.log(new Date().toISOString(), 'done', n, 'visits ->', OUT);
