// EC4: erased subjects at the edge. Core pushes each erasure's tombstone hash to KV under
// gone/<site>/<sha256(site 0 subject)>; the Worker refuses to decide for that subject or to queue
// its events, and remembers it per PoP. A cookieless subject this PoP has not seen still goes to
// Core (which holds the authority, and fails closed). A KV read that does not answer means
// "unknown": Core decides.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { GONE_CACHE_TTL_S } from '../src/index.js';
import { snapshot, harness } from './harness.mjs';

/// sha256("shop" 0x00 "alice"); Core's tests pin the same formula (core/tests/edge_tombstone.rs).
const ALICE = 'gone/shop/9014771bf7a8a38d1f7a37dfd70590bac45345beb671e1825497daf4b5b7f39e';
const req = (subject = 'alice', ctx = {}) => ({ site: 'shop', slot: 'hero', subject, ctx });
const uuid = (n) => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
const events = (subject = 'alice') => ({ site: 'shop', subject, events: [{ id: uuid(1), name: 'click' }], consent: true });

for (const queueWrites of ['on', 'off']) {
  test(`queue ${queueWrites}: a subject in the KV tombstone set is refused at the edge, and nothing is decided, sent or queued`, async () => {
    const h = harness({ kv: snapshot(), beaconSnap: snapshot(), gone: [ALICE], queueWrites });
    const r = await h.call(req());
    assert.equal(r.status, 410);
    assert.equal(r.headers.get('x-prb-decided-by'), 'edge');
    assert.equal(r.headers.get('x-prb-erased'), 'yes');
    assert.match(r.body.error, /erased/);
    assert.equal(h.decides().length, 0, 'Core was asked to decide');
    assert.equal(h.ingests().length, 0, 'a decision record was sent');
    assert.equal(h.queued.length, 0, 'a decision record was queued');
  });
}

test('the tombstone key is gone/<site>/sha256(site 0 subject), read with a 30 s cache', async () => {
  const h = harness({ kv: snapshot(), beaconSnap: snapshot() });
  await h.call(req());
  const reads = h.kvReads.filter((x) => x.key.startsWith('gone/'));
  assert.deepEqual(reads.map((x) => x.key), [ALICE]);
  assert.equal(GONE_CACHE_TTL_S, 30);
  assert.equal(reads[0].opts.cacheTtl, GONE_CACHE_TTL_S);
});

test('a subject not in the set is decided at the edge as before', async () => {
  const h = harness({ kv: snapshot(), beaconSnap: snapshot(), gone: [ALICE] });
  const r = await h.call(req('bob'));
  assert.equal(r.status, 200);
  assert.equal(r.headers.get('x-prb-decided-by'), 'edge');
});

test('the PoP keeps the hint: a second decide for the erased subject reads KV no more', async () => {
  const h = harness({ kv: snapshot(), beaconSnap: snapshot(), gone: [ALICE] });
  await h.call(req());
  h.goneKeys.clear(); // even if KV forgot it, this PoP remembers for a day
  h.clock.t += 3600_000;
  const before = h.kvReads.filter((x) => x.key.startsWith('gone/')).length;
  const r = await h.call(req());
  assert.equal(r.status, 410);
  assert.equal(h.kvReads.filter((x) => x.key.startsWith('gone/')).length, before, 'KV was read again');
});

test('a tombstone read that does not answer: Core decides (fail closed), the edge does not', async () => {
  const h = harness({ kv: snapshot(), beaconSnap: snapshot(), goneHangs: true });
  const r = await h.call(req());
  assert.equal(r.headers.get('x-prb-decided-by'), 'core');
  assert.equal(r.headers.get('x-prb-edge-fallback'), 'erase-check-timeout');
  assert.equal(h.decides().length, 1);
});

test('a cookieless subject the PoP has not seen goes to Core even when KV has no tombstone for it', async () => {
  const h = harness({ kv: snapshot({ mode: 'cookieless' }), beaconSnap: snapshot({ mode: 'cookieless' }) });
  const r = await h.call(req('carol'));
  assert.equal(r.headers.get('x-prb-edge-fallback'), 'unseen-cookieless');
  assert.equal(h.decides().length, 1);
});

test('queue on: /e of a subject in the tombstone set is not queued, and the PoP then answers 410', async () => {
  const h = harness({ queueWrites: 'on', gone: [ALICE] });
  const first = await h.call(events(), '/e');
  assert.equal(first.status, 202, 'the first /e must not wait for KV');
  assert.equal(h.queued.length, 0, 'an erased subject\'s event was queued');
  assert.equal(h.core.length, 0, 'an erased subject\'s event was sent to Core');
  const again = await h.call(events(), '/e');
  assert.equal(again.status, 410);
  assert.equal(again.headers.get('x-prb-queued'), 'no; erased');
  assert.equal(h.queued.length, 0);
});

test('queue on: /e of a live subject is still queued', async () => {
  const h = harness({ queueWrites: 'on', gone: [ALICE] });
  const r = await h.call(events('bob'), '/e');
  assert.equal(r.status, 202);
  assert.equal(h.queued.length, 1);
});

test('queue on: a tombstone read that does not answer still queues the event (Core refuses it per event)', async () => {
  const h = harness({ queueWrites: 'on', goneHangs: true });
  await h.call(events(), '/e');
  assert.equal(h.queued.length, 1);
});

test('queue on: a decide Core refused as gone teaches the PoP, so the next /e is refused (EC3 lost this)', async () => {
  // Core says 410 for a cookieless subject it has erased, KV not yet caught up.
  const h = harness({ kv: snapshot({ mode: 'cookieless' }), beaconSnap: snapshot({ mode: 'cookieless' }), queueWrites: 'on' });
  const realFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => (new URL(String(url)).pathname === '/decide' ? Response.json({ error: 'this subject was erased; start a new one' }, { status: 410 }) : realFetch(url, init));
  const d = await h.call(req());
  assert.equal(d.status, 410);
  const e = await h.call(events(), '/e');
  assert.equal(e.status, 410);
  assert.equal(h.queued.length, 0);
});
