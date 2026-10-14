// F2b: the Worker never decides a sorted slot. Core leaves sorted slots out of the snapshot, so
// the kernel finds no such slot and Core decides (its answer, order and all, passes through).
// A snapshot that did carry one (a future format, a Core bug) must not be decided as a swap.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { snapshot, harness } from './harness.mjs';

const ask = (slot, subject = 'v-1') => ({ site: 'shop', slot, subject, ctx: {} });

function withSortedGrid() {
  const s = snapshot();
  s.slots.grid = { ...structuredClone(s.slots.hero), id: 2, kind: 'sorted' };
  return s;
}

test('a sorted slot missing from the snapshot goes to Core, every time', async () => {
  const h = harness({ kv: snapshot(), beaconSnap: snapshot() });
  for (let i = 0; i < 2; i++) {
    // The second ask finds the sticky record the Worker learned from Core's first answer.
    const r = await h.call(ask('grid'));
    assert.equal(r.headers.get('x-prb-decided-by'), 'core');
    assert.equal(r.headers.get('x-prb-edge-fallback'), 'unknown-slot');
  }
  assert.equal(h.decides().length, 2);
});

test('a batch naming a sorted slot goes to Core whole', async () => {
  const h = harness({ kv: snapshot(), beaconSnap: snapshot() });
  const r = await h.call({ site: 'shop', subject: 'v-1', slots: ['hero', 'grid'], ctx: {} }, '/decide/batch');
  assert.equal(r.headers.get('x-prb-decided-by'), 'core');
  assert.equal(h.decides().length, 1);
  assert.equal(h.ingests().length, 0, 'the Worker decided hero itself');
});

test('a snapshot that carries a sorted slot still sends it to Core', async () => {
  const h = harness({ kv: withSortedGrid(), beaconSnap: withSortedGrid() });
  const r = await h.call(ask('grid'));
  assert.equal(r.headers.get('x-prb-decided-by'), 'core');
  assert.equal(r.headers.get('x-prb-edge-fallback'), 'sorted');
  // The swap slot beside it is still decided at the edge.
  const hero = await h.call(ask('hero', 'v-2'));
  assert.equal(hero.headers.get('x-prb-decided-by'), 'edge');
});
