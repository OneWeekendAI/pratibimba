// F3 live check: a cohort with planted visitor histories, then renders of the home page, so the
// `comeback` slot is decided by @pratibimba/next exactly as for a browser. `come_back` is gated on
// "last order 30+ days ago" (scripts/audiences.mjs).
//
//   PRB_TIMEOUT_MS=3000 PRB_PERSIST=1 npx next start -p 3202
//   PRB_ADMIN_TOKEN=... PRB_DB=... node scripts/history.mjs --base http://localhost:3202 --out scripts/out/history.jsonl
//
// GROUND TRUTH (planted; every group is consented unless it says otherwise):
//   lapsed      one order through /checkout, moved 35-60 days back      -> in the audience
//   recent      one order, moved 2-10 days back                         -> not in it
//   abandoner   an add_to_cart (a filters item), moved 2-5 days back, no order -> not in it
//   new         a first render, nothing else                            -> not in it
//   cookieless  a lapsed history, but the visit carries no consent      -> not in it, no history
// Planting goes through the store like any visit; only the timestamps are moved, with SQL on the
// visitor's own rows (PRB_DB), since Core stamps received_at itself.
//
// Each subject then renders the page RENDERS times. The log has one line per render: group,
// subject, the comeback decision id, whether the offer was on the page, and the decision detail
// from /api/decisions/{id} (pool, features). The summary checks:
//   - the audience matches every decided lapsed render and nothing else; `come_back` is in a pool
//     and on a page only for lapsed subjects (it is approved at a 50% share, so not every pool)
//   - consented decisions list feature:history.*; cookieless ones list none
import { appendFileSync, mkdirSync } from 'node:fs';
import { dirname } from 'node:path';
import { randomUUID } from 'node:crypto';
import { execFileSync } from 'node:child_process';

const arg = (k, d) => { const i = process.argv.indexOf(`--${k}`); return i > 0 ? process.argv[i + 1] : d; };
const BASE = arg('base', 'http://localhost:3202');
const CORE = arg('core', 'http://localhost:8080');
const OUT = arg('out', `scripts/out/history-${Date.now()}.jsonl`);
const PER = Number(arg('per', '10'));
const RENDERS = Number(arg('renders', '3'));
const TOKEN = process.env.PRB_ADMIN_TOKEN;
const DB = process.env.PRB_DB;
if (!TOKEN || !DB) throw new Error('set PRB_ADMIN_TOKEN and PRB_DB');
const OFFER = 'WELCOME15';
const UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Safari/537.36';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
mkdirSync(dirname(OUT), { recursive: true });
const log = (o) => appendFileSync(OUT, JSON.stringify(o) + '\n');

function cookieOf(res, name) {
  for (const c of res.headers.getSetCookie?.() ?? []) {
    const m = new RegExp(`^${name}=([^;]*)`).exec(c);
    if (m) return decodeURIComponent(m[1]);
  }
  return null;
}

// Keep-alive sockets the server closed while the script waited on Postgres fail (reset or broken
// pipe), one per pooled socket: send again, a few times. Safe for every call here: event ids and
// order ids are idempotent.
async function req(url, init, tries = 6) {
  try { return await fetch(url, init); } catch (e) {
    if (tries > 1 && ['ECONNRESET', 'EPIPE'].includes(e.cause?.code)) return req(url, init, tries - 1);
    throw e;
  }
}

async function render(subject, consent, path = '/') {
  const cookies = [];
  if (subject) cookies.push(`_prb_subject=${encodeURIComponent(subject)}`);
  if (consent) cookies.push('_prb_consent=granted');
  const t0 = Date.now();
  const res = await req(`${BASE}${path}`, { headers: { 'user-agent': UA, ...(cookies.length ? { cookie: cookies.join('; ') } : {}) } });
  const html = await res.text();
  // The comeback slot's wrapper carries its decision id.
  const m = /data-prb-slot="comeback"[^>]*data-prb-decision="([^"]+)"|data-prb-decision="([^"]+)"[^>]*data-prb-slot="comeback"/.exec(html);
  return { status: res.status, ms: Date.now() - t0, subject: cookieOf(res, '_prb_subject') ?? subject, decision: m ? m[1] ?? m[2] : null, offer: html.includes(OFFER) };
}

async function event(subject, name, items) {
  const body = { site: 'shop', subject, consent: true, events: [{ id: randomUUID(), name, ...(items ? { items } : {}) }] };
  const r = await req(`${BASE}/_prb/e`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
  return r.status;
}

async function checkout(subject) {
  const form = new URLSearchParams({ slug: 'estate-filter', quantity: '1', order_id: `hist-${randomUUID()}` });
  const r = await req(`${BASE}/checkout`, { method: 'POST', redirect: 'manual', body: form,
    headers: { cookie: `_prb_subject=${encodeURIComponent(subject)}; _prb_consent=granted`, 'user-agent': UA } });
  return r.status;
}

const sql = (q) => execFileSync('psql', [DB, '-v', 'ON_ERROR_STOP=1', '-tAc', q], { encoding: 'utf8' }).trim();
const lit = (s) => `'${String(s).replace(/'/g, "''")}'`;

async function detail(id) {
  const r = await fetch(`${CORE}/api/decisions/${id}`, { headers: { authorization: `Bearer ${TOKEN}` } });
  return r.status === 200 ? r.json() : { error: r.status };
}

// 1. Plant.
const GROUPS = ['lapsed', 'recent', 'abandoner', 'new', 'cookieless'];
const subjects = [];
for (const group of GROUPS) {
  for (let k = 0; k < PER; k++) {
    // The first visit is a product page, so the subject exists before any `comeback` decision:
    // a decision is sticky for its slot's TTL, and one made before planting would be reused.
    const first = await render(null, true, '/p/estate-filter');
    const s = { group, subject: first.subject, days: null, planted: [] };
    if (group === 'lapsed' || group === 'recent' || group === 'cookieless') s.planted.push(['order', await checkout(s.subject)]);
    if (group === 'abandoner') s.planted.push(['add_to_cart', await event(s.subject, 'add_to_cart', [{ id: 'estate-filter', category: 'filters' }])]);
    s.days = group === 'recent' ? 2 + Math.floor(Math.random() * 9)
      : group === 'abandoner' ? 2 + Math.floor(Math.random() * 4)
      : group === 'new' ? null : 35 + Math.floor(Math.random() * 26);
    subjects.push(s);
  }
}
// Writes may go through the edge queue (EC3): wait until every planted row is in Postgres.
const planted = subjects.filter((s) => s.days != null);
const want = (s) => (s.group === 'abandoner' ? "name = 'add_to_cart'" : 'order_id IS NOT NULL');
for (let tries = 0; ; tries++) {
  const missing = planted.filter((s) => sql(`SELECT count(*) FROM events WHERE site_id = 'shop' AND subject = ${lit(s.subject)} AND ${want(s)}`) === '0');
  if (!missing.length) break;
  if (tries > 30) throw new Error(`${missing.length} planted rows never landed`);
  await sleep(2000);
}
// Move each subject's whole history back (its first render included), so nothing sits after it.
for (const s of planted) {
  sql(`UPDATE events SET received_at = received_at - interval '${s.days} days' WHERE site_id = 'shop' AND subject = ${lit(s.subject)}`);
}
log({ header: true, at: new Date().toISOString(), per: PER, renders: RENDERS, subjects });

// 2. Render and read back.
const rows = [];
for (let r = 0; r < RENDERS; r++) {
  for (const s of subjects) {
    const consent = s.group !== 'cookieless';
    const got = await render(consent ? s.subject : null, consent);
    const d = got.decision ? await detail(got.decision) : null;
    const feats = Object.keys(d?.features ?? {}).filter((k) => k.startsWith('feature:history.'));
    const row = { group: s.group, subject: s.subject, days: s.days, ...got, served: typeof d?.variant === 'string' ? d.variant : d?.variant?.key ?? null,
      holdout: d?.in_holdout ?? null, matched: d?.audiences?.find((a) => a.name === 'Last order 30+ days ago')?.matched ?? null,
      in_pool: (d?.scores ?? []).some((x) => x.variant === 'come_back'), history_features: feats,
      history: Object.fromEntries(feats.map((k) => [k, d.features[k]])), sentence: d?.sentence ?? null };
    rows.push(row);
    log(row);
  }
}

// 3. Summary.
const by = (g) => rows.filter((x) => x.group === g);
for (const g of GROUPS) {
  const xs = by(g);
  console.log(`${g.padEnd(10)} renders ${xs.length}  decided ${xs.filter((x) => x.decision).length}  offer ${xs.filter((x) => x.offer).length}  `
    + `matched ${xs.filter((x) => x.matched).length}  come_back in pool ${xs.filter((x) => x.in_pool).length}  holdout ${xs.filter((x) => x.holdout).length}  `
    + `with history features ${xs.filter((x) => x.history_features.length).length}`);
}
const ms = rows.map((x) => x.ms).sort((a, b) => a - b);
// The facts themselves: days since the order (feature scaled by 365), and the abandoned cart.
const days = (x) => Math.round((x.history['feature:history.days_since_last_purchase'] ?? NaN) * 365);
const off = rows.filter((x) => x.decision && ((x.group === 'lapsed' || x.group === 'recent') && days(x) !== x.days
  || x.group === 'abandoner' && x.history['feature:history.cart_abandoned'] !== 1
  || x.group === 'new' && (days(x) !== 365 || x.history['feature:history.cart_abandoned'] !== 0)));
console.log(off.length ? `FAIL: ${off.length} renders carried history facts that differ from the plant` : 'ok: every consented render carried the planted facts');
console.log(`render p50 ${ms[Math.floor(ms.length / 2)]} ms  p95 ${ms[Math.floor(ms.length * 0.95)]} ms`);
const wrong = rows.filter((x) => x.group !== 'lapsed' && (x.offer || x.matched || x.in_pool));
const missed = rows.filter((x) => x.group === 'lapsed' && x.decision && !x.matched);
const leaked = rows.filter((x) => x.group === 'cookieless' && x.history_features.length);
console.log(missed.length ? `FAIL: ${missed.length} lapsed renders did not match the audience` : 'ok: every decided lapsed render matched the audience');
console.log(leaked.length ? `FAIL: ${leaked.length} cookieless renders carried history features` : 'ok: no cookieless render carried history features');
console.log(wrong.length ? `FAIL: ${wrong.length} renders outside the lapsed group matched, pooled or showed the offer` : 'ok: only lapsed subjects matched, pooled or saw the offer');
console.log(`log: ${OUT}`);
