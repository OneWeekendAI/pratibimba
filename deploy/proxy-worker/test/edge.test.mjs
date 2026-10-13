// EC2: the Worker decides /decide itself from the snapshot, falls back to Core whenever it cannot
// be sure, sends its decisions to Core, and never decides from a snapshot older than the
// beacon's version as of BEACON_FRESH_MS ago. Run: node --test test/
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createWorker, createState, BEACON_FRESH_MS, BEACON_WAIT_MS, MIDNIGHT_GUARD_MS } from '../src/index.js';

import { snapshot, harness, TOKEN, NOON } from './harness.mjs';

const req = (subject = 'v-1', ctx = {}) => ({ site: 'shop', slot: 'hero', subject, ctx });

test('decides at the edge from a current snapshot, and sends the record to Core', async () => {
  const h = harness({ kv: snapshot(), beaconSnap: snapshot() });
  const r = await h.call(req());
  assert.equal(r.headers.get('x-prb-decided-by'), 'edge');
  assert.equal(h.decides().length, 0, 'Core was asked to decide');
  assert.equal(r.body.decision_id, 'edge-1');
  assert.ok(['baseline', 'gift'].includes(r.body.variant));
  assert.equal(r.body.explanation.seed, '42');
  const [ing] = h.ingests();
  assert.ok(ing, 'the record never reached Core');
  assert.equal(ing.headers.get('authorization'), `Bearer ${TOKEN}`);
  assert.equal(ing.body.subject, 'v-1');
  assert.equal(ing.body.snapshot_version, 1);
  assert.equal(ing.body.decisions[0].id, 'edge-1');
  assert.equal(ing.body.decisions[0].subject, 'v-1');
});

test('the switch: EDGE_DECIDE=off sends every decide to Core', async () => {
  const h = harness({ kv: snapshot(), beaconSnap: snapshot(), edgeDecide: 'off' });
  const r = await h.call(req());
  assert.equal(h.decides().length, 1);
  assert.equal(r.headers.get('x-prb-decided-by'), null);
  assert.equal(h.ingests().length, 0);
});

test('no snapshot in the beacon: Core decides', async () => {
  const h = harness({ kv: snapshot() });
  const r = await h.call(req());
  assert.equal(r.headers.get('x-prb-edge-fallback'), 'snapshot-missing');
  assert.equal(h.decides().length, 1);
});

test('a beacon that does not answer costs at most BEACON_WAIT_MS, then Core decides', async () => {
  const h = harness({ kv: snapshot(), beaconHangs: true });
  const t0 = Date.now();
  const r = await h.call(req());
  assert.ok(Date.now() - t0 < BEACON_WAIT_MS + 300);
  assert.equal(r.headers.get('x-prb-edge-fallback'), 'beacon-unreachable');
  assert.equal(h.decides().length, 1);
});

test('KV behind the beacon: the snapshot comes from the beacon, never the stale KV copy', async () => {
  const h = harness({ kv: snapshot({ version: 1 }), beaconSnap: snapshot({ version: 2, paused: true }) });
  const r = await h.call(req());
  assert.equal(r.headers.get('x-prb-decided-by'), 'edge');
  assert.equal(r.headers.get('x-prb-snapshot-version'), '2');
  assert.equal(r.body.variant, 'baseline');
  assert.deepEqual(Object.keys(r.body.explanation.scores), ['baseline'], 'decided from the unpaused KV copy');
});

test('the 5 s rule: a kill switch pushed after a reading reaches the edge once the reading is BEACON_FRESH_MS old', async () => {
  const h = harness({ kv: snapshot({ version: 1 }), beaconSnap: snapshot({ version: 1 }) });
  const first = await h.call(req('a'));
  assert.equal(first.headers.get('x-prb-snapshot-version'), '1');
  await h.push(snapshot({ version: 2, paused: true }));
  // Past BEACON_FRESH_MS, no request may still use version 1.
  h.clock.t += BEACON_FRESH_MS + 1;
  const late = await h.call(req('b'));
  assert.equal(late.headers.get('x-prb-snapshot-version'), '2');
  assert.deepEqual(Object.keys(late.body.explanation.scores), ['baseline']);
  assert.ok(BEACON_FRESH_MS <= 4000, 'the bound plus Core\'s push must stay inside ~5 s');
});

test('between readings the beacon is refreshed in the background', async () => {
  const h = harness({ kv: snapshot(), beaconSnap: snapshot() });
  await h.call(req('a'));
  const before = h.calls.version;
  h.clock.t += 1500;
  await h.call(req('b'));
  assert.equal(h.calls.version, before + 1, 'no background refresh between 1 s and BEACON_FRESH_MS');
});

test('a cookieless visitor not seen today goes to Core; the pseudonym Core reports is kept and used', async () => {
  const h = harness({ kv: snapshot({ mode: 'cookieless' }), beaconSnap: snapshot({ mode: 'cookieless' }), coreSubject: 'p1_abc' });
  const first = await h.call(req('raw-1'));
  assert.equal(first.headers.get('x-prb-edge-fallback'), 'unseen-cookieless');
  assert.equal(first.headers.get('x-prb-edge'), null, 'the edge header reached the client');
  assert.equal(h.decides()[0].headers.get('x-prb-edge-token'), TOKEN);
  // A new slot decision later the same day (the first one is sticky; move past its TTL).
  h.clock.t += 61_000;
  const second = await h.call(req('raw-1'));
  assert.equal(second.headers.get('x-prb-decided-by'), 'edge');
  assert.equal(h.decides().length, 1);
  assert.equal(h.ingests()[0].body.decisions[0].subject, 'p1_abc');
  assert.equal(h.ingests()[0].body.subject, 'raw-1');
  // The cache never holds the raw id.
  for (const [url, e] of h.cache.entries) assert.ok(!url.includes('raw-1') && !e.text.includes('raw-1'));
});

test('the pseudonym is forgotten at midnight UTC, and the last minute before it goes to Core', async () => {
  const h = harness({ kv: snapshot({ mode: 'cookieless' }), beaconSnap: snapshot({ mode: 'cookieless' }), coreSubject: 'p1_abc' });
  await h.call(req('raw-1'));
  h.clock.t = Date.parse('2026-10-14T00:00:00.000Z') - MIDNIGHT_GUARD_MS + 1;
  const near = await h.call(req('raw-1'));
  assert.equal(near.headers.get('x-prb-edge-fallback'), 'midnight');
  h.clock.t = Date.parse('2026-10-14T00:00:05.000Z');
  const next = await h.call(req('raw-2'));
  assert.equal(next.headers.get('x-prb-edge-fallback'), 'unseen-cookieless');
});

test('a consented visitor on a consent_aware site is identified, so the edge decides at once', async () => {
  const h = harness({ kv: snapshot({ mode: 'consent_aware' }), beaconSnap: snapshot({ mode: 'consent_aware' }) });
  const r = await h.call(req('raw-9', { consent: true }));
  assert.equal(r.headers.get('x-prb-decided-by'), 'edge');
  const no = await h.call(req('raw-10', { consent: false }));
  assert.equal(no.headers.get('x-prb-edge-fallback'), 'unseen-cookieless');
});

test('sticky: a reload inside the TTL gets the same decision and sends nothing new', async () => {
  const h = harness({ kv: snapshot({ ttl: 60 }), beaconSnap: snapshot({ ttl: 60 }) });
  const a = await h.call(req('s'));
  h.clock.t += 30_000;
  const b = await h.call(req('s'));
  assert.equal(b.body.decision_id, a.body.decision_id);
  assert.equal(h.ingests().length, 1);
  h.clock.t += 31_000;
  const c = await h.call(req('s'));
  assert.notEqual(c.body.decision_id, a.body.decision_id);
  assert.equal(h.ingests().length, 2);
});

test('a decision Core made is sticky at the edge too', async () => {
  const h = harness({ kv: snapshot({ mode: 'cookieless' }), beaconSnap: snapshot({ mode: 'cookieless' }), coreSubject: 'p1_x' });
  const a = await h.call(req('raw'));
  const b = await h.call(req('raw'));
  assert.equal(b.headers.get('x-prb-decided-by'), 'edge');
  assert.equal(b.body.decision_id, a.body.decision_id);
});

test('a site with a picker goes to Core (it reads each visitor\'s picks)', async () => {
  const h = harness({ kv: snapshot({ picker: ['kitchen'] }), beaconSnap: snapshot({ picker: ['kitchen'] }) });
  const r = await h.call(req());
  assert.equal(r.headers.get('x-prb-edge-fallback'), 'picker');
});

test('anything Core would refuse goes to Core, which answers it', async () => {
  const h = harness({ kv: snapshot(), beaconSnap: snapshot() });
  for (const body of [req('bad subject'), { ...req(), slot: 'nope' }, req('v', { gender: 'x' }), req('v', { tz: 'Mars/Base' })]) {
    const r = await h.call(body);
    assert.equal(r.headers.get('x-prb-decided-by'), 'core', JSON.stringify(body));
  }
});

test('an erased visitor: once Core says gone, the PoP sends them to Core', async () => {
  const h = harness({ kv: snapshot(), beaconSnap: snapshot(), ingest: 'gone' });
  await h.call(req('erased-1'));
  h.clock.t += 61_000;
  const r = await h.call(req('erased-1'));
  assert.equal(r.headers.get('x-prb-edge-fallback'), 'erased');
});

test('batch: decided at the edge as one answer', async () => {
  const h = harness({ kv: snapshot(), beaconSnap: snapshot() });
  const r = await h.call({ site: 'shop', subject: 'v', slots: ['hero'], ctx: {} }, '/decide/batch');
  assert.equal(r.headers.get('x-prb-decided-by'), 'edge');
  assert.equal(r.body.decisions.length, 1);
  assert.equal(r.body.decisions[0].slot, 'hero');
});

test('a client cannot pose as the Worker to Core', async () => {
  const h = harness({ kv: snapshot(), beaconSnap: snapshot(), edgeDecide: 'off' });
  await h.call(req(), '/decide', { 'x-prb-edge-token': 'guess' });
  assert.equal(h.decides()[0].headers.get('x-prb-edge-token'), null);
});

test('push: Core\'s snapshot reaches the beacon only with the token, and never goes backwards', async () => {
  const h = harness();
  const push = (body, token) => h.worker.fetch(new Request('https://api.example/_prb/edge/snapshot', {
    method: 'POST', body: JSON.stringify(body), headers: token ? { authorization: `Bearer ${token}` } : {} }), h.env, h.ctx);
  assert.equal((await push(snapshot({ version: 3 }), 'wrong')).status, 401);
  assert.equal((await push(snapshot({ version: 3 }))).status, 401);
  assert.deepEqual(await (await push(snapshot({ version: 3 }), TOKEN)).json(), { kept: true, version: 3 });
  assert.deepEqual(await (await push(snapshot({ version: 2 }), TOKEN)).json(), { kept: false, version: 3 });
  assert.equal(h.decides().length, 0, 'a push was forwarded to Core');
});
