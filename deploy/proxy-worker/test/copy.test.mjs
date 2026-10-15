// F4: the Worker never decides a copy slot: Core returns its fields as `copy`, which the kernel
// does not build. Core leaves copy slots out of the snapshot; a snapshot carrying one anyway
// must still go to Core rather than be decided as a swap (which would hand the JSON payload out).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { snapshot, harness } from './harness.mjs';

const ask = (slot, subject = 'v-1') => ({ site: 'shop', slot, subject, ctx: {} });

function withCopyHeadline() {
  const s = snapshot();
  s.slots.hero_copy = { ...structuredClone(s.slots.hero), id: 3, kind: 'copy' };
  return s;
}

test('a snapshot that carries a copy slot still sends it to Core', async () => {
  const h = harness({ kv: withCopyHeadline(), beaconSnap: withCopyHeadline() });
  const r = await h.call(ask('hero_copy'));
  assert.equal(r.headers.get('x-prb-decided-by'), 'core');
  assert.equal(r.headers.get('x-prb-edge-fallback'), 'copy');
  const hero = await h.call(ask('hero', 'v-2'));
  assert.equal(hero.headers.get('x-prb-decided-by'), 'edge');
});

test('a batch naming a copy slot goes to Core whole', async () => {
  const h = harness({ kv: withCopyHeadline(), beaconSnap: withCopyHeadline() });
  const r = await h.call({ site: 'shop', subject: 'v-1', slots: ['hero', 'hero_copy'], ctx: {} }, '/decide/batch');
  assert.equal(r.headers.get('x-prb-decided-by'), 'core');
  assert.equal(r.headers.get('x-prb-edge-fallback'), 'copy');
});
