// F3: visitor history lives in Core's Postgres; the snapshot has none of it. So an identified
// request on a slot that reads history (the site has a `history:` block, or a variant of the slot
// is gated on a history rule) goes to Core, which reads the facts. Found live in Seoul: a lapsed,
// consented buyer decided at the edge matched no audience and lost the win-back variant.
// A request Core would read no history for (cookieless, unconsented, `history` not allowed) is
// still the edge's to decide.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { snapshot, harness } from './harness.mjs';

const ask = (subject = 'v-1', ctx = {}) => ({ site: 'shop', slot: 'hero', subject, ctx });

function gatedOnHistory(opts) {
  const s = snapshot(opts);
  s.slots.hero.gates = [{ audience_id: 23, audience: 'Last order 30+ days ago', status: 'audience',
    predicate: { days_since_last_purchase: { gte: 30 } }, variant_id: 2, variant: 'gift' }];
  return s;
}

function withHistoryBlock(opts) {
  const s = snapshot(opts);
  s.settings.history = { enabled: true, ltv_edges: null, cart_window_hours: null, views: [] };
  return s;
}

test('an identified visitor on a slot gated on a history rule goes to Core', async () => {
  const h = harness({ kv: gatedOnHistory(), beaconSnap: gatedOnHistory() });
  const r = await h.call(ask());
  assert.equal(r.headers.get('x-prb-decided-by'), 'core');
  assert.equal(r.headers.get('x-prb-edge-fallback'), 'history');
});

test('a batch naming one slot gated on history goes to Core whole', async () => {
  const s = gatedOnHistory();
  s.slots.promo = { ...structuredClone(snapshot().slots.hero), id: 3 };
  const h = harness({ kv: s, beaconSnap: s });
  const r = await h.call({ site: 'shop', subject: 'v-1', slots: ['promo', 'hero'], ctx: {} }, '/decide/batch');
  assert.equal(r.headers.get('x-prb-edge-fallback'), 'history');
  assert.equal(h.ingests().length, 0, 'the Worker decided promo itself');
});

test('an identified visitor on a site with a history block goes to Core', async () => {
  const h = harness({ kv: withHistoryBlock(), beaconSnap: withHistoryBlock() });
  const r = await h.call(ask());
  assert.equal(r.headers.get('x-prb-edge-fallback'), 'history');
});

test('a consented visitor on a consent_aware site reads history; an unconsented one does not', async () => {
  const s = gatedOnHistory({ mode: 'consent_aware' });
  const h = harness({ kv: s, beaconSnap: s });
  const r = await h.call(ask('v-1', { consent: true }));
  assert.equal(r.headers.get('x-prb-edge-fallback'), 'history');
  const anon = await h.call(ask('v-2', { consent: false }));
  assert.notEqual(anon.headers.get('x-prb-edge-fallback'), 'history');
});

test('history not in fields_allowed: Core reads none, so the edge decides', async () => {
  const s = gatedOnHistory();
  s.settings.fields_allowed = ['mobile', 'returning'];
  const h = harness({ kv: s, beaconSnap: s });
  const r = await h.call(ask());
  assert.equal(r.headers.get('x-prb-decided-by'), 'edge');
});

test('a slot whose gates read no history is still decided at the edge', async () => {
  const s = snapshot();
  s.slots.hero.gates = [{ audience_id: 4, audience: 'Mobile', status: 'audience', predicate: { mobile: true }, variant_id: 2, variant: 'gift' }];
  s.settings.history = null;
  const h = harness({ kv: s, beaconSnap: s });
  const r = await h.call(ask());
  assert.equal(r.headers.get('x-prb-decided-by'), 'edge');
});
