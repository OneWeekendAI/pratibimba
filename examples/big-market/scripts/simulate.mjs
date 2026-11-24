// Shopper simulator for Big Market. Synthetic visitors with planted segment preferences (truth.json) hit a live
// Core over its public API (/decide/batch, /e), the way sdk.js does. History is backdated (as the dashboard's
// stack.mjs does) so attribution windows close and the worker learns, in two halves: the first half is decided by
// the untrained model, the second by what it learned. Afterwards fresh probe visitors per segment grade what Core
// serves against the planted winners. `--flat` plants NO preference (every variant and item equally good): the control.
//
//   node scripts/simulate.mjs --site haat --visitors 6000 [--flat] [--out proof/sim-haat.json]
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const arg = (k, d) => { const i = process.argv.indexOf('--' + k); return i < 0 ? d : (process.argv[i + 1]?.startsWith('--') || process.argv[i + 1] === undefined ? true : process.argv[i + 1]); };
const SITE = arg('site', 'haat'), VISITORS = +arg('visitors', 6000), FLAT = process.argv.includes('--flat');
const CORE = process.env.CORE ?? 'http://localhost:8080', TOKEN = process.env.PRB_ADMIN_TOKEN ?? 'local-admin-token';
const DB = process.env.PRB_DB ?? 'postgres://postgres@localhost:55440/prb_local';
const here = import.meta.dirname;
const OUT = path.resolve(here, '..', arg('out', `proof/sim-${SITE}.json`));
const truth = JSON.parse(readFileSync(path.join(here, '../truth.json'), 'utf8'));
const cat = JSON.parse(readFileSync(path.join(here, '../src/data/catalogue.json'), 'utf8'));
const byId = Object.fromEntries(cat.map((p) => [p.id, p]));
const site = JSON.parse(readFileSync(path.join(here, '../site.json'), 'utf8'));
const slotDef = Object.fromEntries(site.slots.map((s) => [s.key, s]));
const SEGS = Object.keys(truth.mix);
const psql = (q) => execFileSync('psql', [DB, '-qAt', '-c', q], { stdio: 'pipe' }).toString().trim();
const H = { 'content-type': 'application/json', authorization: `Bearer ${TOKEN}` };
const post = async (p, body) => {
  const r = await fetch(CORE + p, { method: 'POST', headers: H, body: JSON.stringify(body) });
  if (!r.ok && r.status !== 202) throw new Error(`${p} ${r.status} ${await r.text()}`);
  return r.status === 202 ? null : r.json();
};

let seed = +arg('seed', 20261124);
const rand = () => ((seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648);
const pickW = (w) => { let x = rand(), acc = 0; for (const [k, p] of Object.entries(w)) if (x < (acc += p)) return k; return Object.keys(w).at(-1); };
const uuid = () => crypto.randomUUID();

// What a visitor of each kind looks like to the SDK. Fixed signatures, plus noise on the rest.
function context(seg) {
  const utcH = new Date().getUTCHours();
  const localHour = seg === 'gift' ? 19 + Math.floor(rand() * 4) : 8 + Math.floor(rand() * 9); // gift: evening; others: working day
  let tz = ((localHour - utcH) * 60 + 1440) % 1440; if (tz > 840) tz -= 1440;
  const c = { returning: false, mobile: rand() < 0.5, from_ad: false, from_search: false, from_direct: false };
  if (seg === 'bargain') { c.from_ad = true; c.mobile = true; c.returning = rand() < 0.3; }
  if (seg === 'gift') { c.returning = true; c.from_direct = rand() < 0.5; c.from_search = !c.from_direct; }
  if (seg === 'premium') { c.mobile = false; c.from_direct = true; c.returning = rand() < 0.5; }
  if (seg === 'newbie') { c.returning = false; c.from_search = true; }
  const features = { from_search: c.from_search, from_direct: c.from_direct, evening: localHour >= 17 };
  for (const k of ['electronics', 'kitchen', 'books', 'fashion', 'home', 'toys', 'grocery', 'beauty']) features['viewed_' + k] = false;
  const fav = truth.segments[seg].fav_categories;
  if (rand() < 0.5) features['viewed_' + fav[Math.floor(rand() * fav.length)]] = true;
  return { returning: c.returning, mobile: c.mobile, from_ad: c.from_ad, tz_offset: tz, consent: true, features };
}

// Behaviour. Planted: the segment's winner converts at `win`, everything else at `other`. Flat: all the same.
const rate = (slot, seg, variant) => {
  const m = truth.metric[slot], r = truth.rates[m];
  if (FLAT) return r.other;
  return truth.winners[slot][seg] === variant ? r.win : r.other;
};
const isFav = (seg, id) => truth.segments[seg].fav_categories.includes(byId[id]?.category ?? id);
function itemClick(seg, order) { // first item the visitor acts on, or null
  const R = truth.rates.item;
  for (let i = 0; i < Math.min(R.visible, order.length); i++) {
    const p = (FLAT ? R.other : isFav(seg, order[i]) ? R.fav : R.other) * R.position_decay ** i;
    if (rand() < p) return order[i];
  }
  return null;
}

async function visit(n, stage) {
  const seg = pickW(truth.mix), ctx = context(seg);
  const subject = `sim-${stage}-${n.toString(36)}-${Math.floor(rand() * 1e9).toString(36)}`;
  const events = [];
  const shown = (d) => ({ id: uuid(), name: 'exposure', slot: d.slot, decision_id: d.decision_id, ...(d.order ? { items: d.order.slice(0, 100).map((id) => ({ id })) } : {}) });
  const decide = async (slots) => (await post('/decide/batch', { site: SITE, subject, slots, ctx })).decisions;
  const atc = (id) => events.push({ id: uuid(), name: 'add_to_cart', items: [{ id, price: byId[id].price, quantity: 1 }] });
  // home
  const home = await decide(['promo-bar', 'hero', 'category-rows', 'picks', 'deals']);
  for (const d of home) {
    events.push(shown(d));
    if (d.slot === 'hero' && rand() < rate('hero', seg, d.variant)) events.push({ id: uuid(), name: 'click', slot: 'hero', decision_id: d.decision_id });
    if (d.slot === 'category-rows') { const it = itemClick(seg, d.order); if (it) events.push({ id: uuid(), name: 'click', slot: d.slot, decision_id: d.decision_id, items: [{ id: it }] }); }
    if (d.slot === 'picks' || d.slot === 'deals') { const it = itemClick(seg, d.order); if (it) atc(it); }
  }
  // listing, product
  const listing = await decide(['listing-sort']);
  events.push(shown(listing[0]));
  const pid = site.slots.find((s) => s.key === 'picks').variants[Math.floor(rand() * 12)].key;
  if (rand() < rate('listing-sort', seg, listing[0].variant)) atc(pid);
  const prod = await decide(['product-badge', 'buy-button', 'delivery-message']);
  const vOf = {};
  for (const d of prod) { events.push(shown(d)); vOf[d.slot] = d.variant; }
  for (const s of ['product-badge', 'buy-button']) if (rand() < rate(s, seg, vOf[s])) atc(pid);
  // cart
  const cart = await decide(['cart-cross-sell']);
  events.push(shown(cart[0]));
  { const it = itemClick(seg, cart[0].order); if (it) atc(it); }
  // purchase: either planted cause can tip the visitor into buying
  const pP = rate('promo-bar', seg, home.find((d) => d.slot === 'promo-bar').variant), pD = rate('delivery-message', seg, vOf['delivery-message']);
  const buys = rand() < 1 - (1 - pP) * (1 - pD);
  if (buys) { const price = byId[pid].price; events.push({ id: uuid(), name: 'purchase', order_id: `SIM-${subject}`, value: price, currency: 'INR', items: [{ id: pid, price, quantity: 1 }] }); }
  await post('/e', { site: SITE, subject, events });
  return { seg, buys };
}

async function run(from, to, stage) {
  let next = from; const tally = {};
  await Promise.all(Array.from({ length: 12 }, async () => {
    while (next < to) { const r = await visit(next++, stage); (tally[r.seg] ??= { n: 0, buys: 0 }); tally[r.seg].n++; tally[r.seg].buys += r.buys; }
  }));
  return tally;
}

function backdate(since, fromDays, toDays, keep = 0) {
  for (let attempt = 1; ; attempt++) {
    try {
      psql(`CREATE TEMP TABLE shift AS
         SELECT id, make_interval(secs => ${toDays * 86400} + greatest(0, (1 - (row_number() OVER (ORDER BY created_at))::float / count(*) OVER () - ${keep})) * ${(fromDays - toDays) * 86400}) AS by
           FROM decisions WHERE site_id = '${SITE}' AND created_at >= '${since}';
       UPDATE decisions d SET created_at = d.created_at - s.by FROM shift s WHERE s.id = d.id;
       UPDATE events e SET received_at = e.received_at - s.by + interval '2 seconds' FROM shift s WHERE s.id = e.decision_id;
       UPDATE events e SET received_at = e.received_at - s.by + interval '2 seconds' FROM shift s, decisions d WHERE d.id = s.id AND e.decision_id IS NULL AND e.site_id = '${SITE}' AND e.subject = d.subject AND e.received_at >= '${since}';`);
      return;
    } catch (e) {
      if (!/deadlock detected/.test(String(e.stderr ?? e.message)) || attempt >= 5) throw e;
    }
  }
}
const settled = async () => {
  const end = Date.now() + 600_000;
  for (;;) {
    const pe = +psql(`SELECT count(*) FROM pending_events p JOIN events e ON e.id = p.event_id WHERE e.site_id = '${SITE}' AND e.subject LIKE 'sim-%'`);
    const un = +psql(`SELECT count(*) FROM decisions WHERE site_id = '${SITE}' AND settled_at IS NULL AND created_at < now() - interval '1 day'`);
    if (!pe && !un) return;
    if (Date.now() > end) throw new Error(`worker did not settle (pending_events ${pe}, unsettled ${un})`);
    await new Promise((r) => setTimeout(r, 1000));
  }
};
const now = () => psql('SELECT now()::text');

// ---- run ----
const t0 = Date.now();
const log = (...a) => console.error(`[${((Date.now() - t0) / 1000).toFixed(0)}s]`, ...a);
const half = Math.floor(VISITORS / 2);
const tA = now(); const mixA = await run(0, half, 'a'); log('first half in', JSON.stringify(mixA));
backdate(tA, 6, 3); await settled(); log('first half settled');
const tB = now(); const mixB = await run(half, VISITORS, 'b'); log('second half in');
backdate(tB, 3, 120 / 86400, 0.03); await settled(); log('second half settled');

// grade: fresh probe visitors per segment, decided by the model as it stands now
const PROBES = 120;
const slots = Object.keys(slotDef);
const graded = [];
for (const seg of SEGS) {
  const tallies = Object.fromEntries(slots.map((s) => [s, {}]));
  const top1fav = Object.fromEntries(slots.filter((s) => slotDef[s].kind === 'sorted').map((s) => [s, 0]));
  let live = 0;
  for (let i = 0; i < PROBES; i++) {
    const ctx = context(seg);
    const { decisions } = await post('/decide/batch', { site: SITE, subject: `probe-${seg}-${i}-${Math.floor(rand() * 1e9)}`, slots, ctx });
    if (decisions[0]?.explanation?.in_holdout) continue;
    live++;
    for (const d of decisions) {
      if (slotDef[d.slot].kind === 'sorted') { if (isFav(seg, d.order[0])) top1fav[d.slot]++; (tallies[d.slot][d.order[0]] ??= 0); tallies[d.slot][d.order[0]]++; }
      else tallies[d.slot][d.variant] = (tallies[d.slot][d.variant] ?? 0) + 1;
    }
  }
  for (const s of slots) {
    const modal = Object.entries(tallies[s]).sort((a, b) => b[1] - a[1])[0];
    if (slotDef[s].kind === 'sorted') {
      const share = top1fav[s] / live;
      const poolFav = slotDef[s].variants.filter((v) => isFav(seg, v.key)).length / slotDef[s].variants.length;
      graded.push({ segment: seg, slot: s, kind: 'sorted', found: share > 0.5, top1_favourite_share: +share.toFixed(2), chance: +poolFav.toFixed(2), probes: live });
    } else {
      const want = truth.winners[s][seg];
      graded.push({ segment: seg, slot: s, kind: slotDef[s].kind ?? 'swap', planted: want, served_modal: modal[0], served_share: +(modal[1] / live).toFixed(2), found: modal[0] === want, probes: live });
    }
  }
}
const found = graded.filter((g) => g.found).length;

// uplift: second half only (decided by the learned model), treated vs holdout, per slot and overall, as closed exposed decisions
const rows = psql(`SELECT s.key, d.in_holdout, count(*), avg(CASE WHEN d.outcome > 0 THEN 1 ELSE 0 END), coalesce(sum(d.revenue), 0)
  FROM decisions d JOIN slots s ON s.id = d.slot_id WHERE d.site_id = '${SITE}' AND d.subject LIKE 'sim-b-%' AND d.exposed_at IS NOT NULL AND d.settled_at IS NOT NULL
  GROUP BY 1, 2 ORDER BY 1, 2`).split('\n').map((l) => l.split('|'));
const uplift = {};
for (const [k, h, n, cr, rev] of rows) (uplift[k] ??= {})[h === 't' ? 'holdout' : 'treated'] = { n: +n, conversion: +(+cr).toFixed(4), revenue_per_decision: +(+rev / +n).toFixed(2) };
for (const u of Object.values(uplift)) if (u.treated && u.holdout) u.lift_abs = +(u.treated.conversion - u.holdout.conversion).toFixed(4);
const core = await (await fetch(`${CORE}/api/sites/${SITE}/results`, { headers: H })).json();
const report = {
  site: SITE, flat_control: FLAT, visitors: VISITORS, seconds: Math.round((Date.now() - t0) / 1000), segments_run: { first_half: mixA, second_half: mixB },
  winners_found: `${found}/${graded.length}`, found, cells: graded.length,
  by_slot: Object.fromEntries(slots.map((s) => [s, `${graded.filter((g) => g.slot === s && g.found).length}/${SEGS.length}`])),
  uplift_second_half: uplift,
  core_site_lift: core.results.lift, core_revenue_lift: core.results.revenue_lift, core_holdout: core.results.holdout, core_treated: core.results.treated,
  graded,
};
writeFileSync(OUT, JSON.stringify(report, null, 1));
console.log(JSON.stringify({ site: SITE, flat: FLAT, found: report.winners_found, by_slot: report.by_slot, uplift: Object.fromEntries(Object.entries(uplift).map(([k, v]) => [k, v.lift_abs])), core_lift: report.core_site_lift }, null, 1));
