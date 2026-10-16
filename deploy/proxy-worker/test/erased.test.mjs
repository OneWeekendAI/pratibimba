// G5 (edge-cache.md, Decision 22): the snapshot carries a Bloom filter of the site's erasures.
// A subject not in it is decided at the edge with no gone/ read (a new visitor's first decide no
// longer pays a cold KV miss); a subject in it takes EC4's path (PoP hint, then gone/); a snapshot
// with no filter, or a malformed one, takes EC4's path for everyone.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { inErased } from '../src/erased.js';
import { snapshot, harness } from './harness.mjs';

const sha = async (s) => [...new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(s)))].map((b) => b.toString(16).padStart(2, '0')).join('');
/// Core's filters (core/src/edge.rs erased_filter_tests, core/tests/edge_tombstone.rs).
const DEMO_ALICE = { n: 1, m: 64, k: 7, bits: 'BQhAAAIQgAA=' };
const DEMO_ALICE_BOB = { n: 2, m: 64, k: 7, bits: 'BQhAAAIwyZI=' };
const SHOP_ALICE = { n: 1, m: 64, k: 7, bits: 'BIAAGAADIAA=' };
const EMPTY = { n: 0, m: 0, k: 7, bits: '' };
const ALICE = 'gone/shop/9014771bf7a8a38d1f7a37dfd70590bac45345beb671e1825497daf4b5b7f39e';
const req = (subject = 'alice') => ({ site: 'shop', slot: 'hero', subject, ctx: {} });
const withFilter = (erased) => ({ ...snapshot(), erased });
const goneReads = (h) => h.kvReads.filter((x) => x.key.startsWith('gone/')).length;

test('membership agrees with Core: its members are in, others are not', async () => {
  assert.equal(inErased(DEMO_ALICE, await sha('demo\0alice')), true);
  assert.equal(inErased(DEMO_ALICE_BOB, await sha('demo\0alice')), true);
  assert.equal(inErased(DEMO_ALICE_BOB, await sha('demo\0bob')), true);
  assert.equal(inErased(DEMO_ALICE, await sha('demo\0bob')), false);
  assert.equal(inErased(SHOP_ALICE, await sha('shop\0alice')), true);
  for (const s of ['bob', 'carol', 'dave', 'new-visitor']) assert.equal(inErased(SHOP_ALICE, await sha(`shop\0${s}`)), false, s);
  assert.equal(inErased(EMPTY, await sha('shop\0alice')), false);
});

test('forty erasures (16 with an even second word): every one is in, as Core built it', async () => {
  const f = { n: 40, m: 400, k: 7, bits: '9p5OHKP/35GFV+jWjkbmGIU9xMlstZ+iOaEt/GHbzxky93pxGzq3Un2Ex31lsGhEFCM=' };
  for (let i = 0; i < 40; i++) assert.equal(inErased(f, await sha(`demo\0s${i}`)), true, `s${i}`);
});

test('no filter, or a malformed one, is unknown (null), never "not erased"', async () => {
  const h = await sha('shop\0bob');
  for (const f of [undefined, null, 'x', {}, { n: 1, m: 64, k: 7 }, { n: 1, m: 63, k: 7, bits: 'BIAAGAADIAA=' }, { n: 1, m: 64, k: 0, bits: 'BIAAGAADIAA=' },
    { n: 1, m: 64, k: 7, bits: 'BIAAGAAD' }, { n: 1, m: 64, k: 7, bits: '!!!!!!!!!!!=' }, { n: 1, m: 0, k: 7, bits: '' }, { n: 0, m: 8, k: 7, bits: 'AA==' }, { n: 1, m: 64, k: 40, bits: 'BIAAGAADIAA=' }]) {
    assert.equal(inErased(f, h), null, JSON.stringify(f));
  }
});

test('a new subject not in the filter is decided at the edge with no gone/ read, even when KV would hang', async () => {
  const h = harness({ kv: withFilter(SHOP_ALICE), beaconSnap: withFilter(SHOP_ALICE), goneHangs: true });
  const r = await h.call(req('new-visitor'));
  assert.equal(r.status, 200);
  assert.equal(r.headers.get('x-prb-decided-by'), 'edge');
  assert.equal(r.headers.get('x-prb-erase-check'), 'filter');
  assert.equal(goneReads(h), 0, 'gone/ was read for a subject the filter rules out');
  assert.equal(h.decides().length, 0);
});

test('an empty filter (no erasures) needs no gone/ read either', async () => {
  const h = harness({ kv: withFilter(EMPTY), beaconSnap: withFilter(EMPTY), goneHangs: true });
  const r = await h.call(req('alice'));
  assert.equal(r.headers.get('x-prb-decided-by'), 'edge');
  assert.equal(goneReads(h), 0);
});

test('a subject in the filter and in gone/ is refused at the edge', async () => {
  const h = harness({ kv: withFilter(SHOP_ALICE), beaconSnap: withFilter(SHOP_ALICE), gone: [ALICE] });
  const r = await h.call(req('alice'));
  assert.equal(r.status, 410);
  assert.equal(r.headers.get('x-prb-decided-by'), 'edge');
  assert.equal(h.ingests().length, 0);
});

test('a filter hit whose gone/ read does not answer goes to Core (fail closed)', async () => {
  const h = harness({ kv: withFilter(SHOP_ALICE), beaconSnap: withFilter(SHOP_ALICE), goneHangs: true });
  const r = await h.call(req('alice'));
  assert.equal(r.headers.get('x-prb-decided-by'), 'core');
  assert.equal(r.headers.get('x-prb-edge-fallback'), 'erase-check-timeout');
});

test('a false positive (in the filter, not in gone/) is decided at the edge after the gone/ read', async () => {
  const h = harness({ kv: withFilter(SHOP_ALICE), beaconSnap: withFilter(SHOP_ALICE) });
  const r = await h.call(req('alice'));
  assert.equal(r.status, 200);
  assert.equal(r.headers.get('x-prb-decided-by'), 'edge');
  assert.equal(r.headers.get('x-prb-erase-check'), 'kv');
  assert.equal(goneReads(h), 1);
});

test('a snapshot without a filter (an older Core) reads gone/ for every subject, as EC4 did', async () => {
  const h = harness({ kv: snapshot(), beaconSnap: snapshot(), goneHangs: true });
  const r = await h.call(req('new-visitor'));
  assert.equal(r.headers.get('x-prb-edge-fallback'), 'erase-check-timeout');
});

test('a malformed filter is not trusted: gone/ is read', async () => {
  const bad = { n: 1, m: 64, k: 7, bits: 'BIAA' };
  const h = harness({ kv: withFilter(bad), beaconSnap: withFilter(bad), goneHangs: true });
  const r = await h.call(req('new-visitor'));
  assert.equal(r.headers.get('x-prb-edge-fallback'), 'erase-check-timeout');
});

test('the PoP hint still refuses a subject the filter does not hold yet (an erase newer than the snapshot)', async () => {
  const h = harness({ kv: withFilter(EMPTY), beaconSnap: withFilter(EMPTY), goneHangs: true });
  // What a 410 from Core or a "gone" record taught this PoP (index.js goneKey).
  h.cache.entries.set(`https://prb-edge.internal/g/${await sha('shop\0bob')}`, { text: '1', until: h.clock.t + 86_400_000 });
  const r = await h.call(req('bob'));
  assert.equal(r.status, 410, 'the PoP forgot what Core told it');
  assert.equal(r.headers.get('x-prb-decided-by'), 'edge');
  assert.equal(goneReads(h), 0);
});
