// Kernel properties the generated parity run cannot reach by chance.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { decide } from '../src/kernel.js';
import { inHoldout, withinShare } from '../src/hash.js';

const prior = { mean: [0, 0, 0, 0], cov: [[1, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]] };
function snap({ holdout = 0, share = null } = {}) {
  return {
    format: 2, site: 'shop', version: 1,
    settings: { privacy_mode: 'full', fields_allowed: null, holdout_fraction: holdout, paused: false, night: { start: 22, end: 5 }, simulated_clock: false },
    slots: { hero: {
      id: 1, cold_start_exposures: 0, decision_ttl_secs: 60, next_experiment_at: null, gates: [], picker_keys: [], declared_features: {},
      experiment: null, cold_start: null, model: { version: 1, state: { arms: {}, n: 0 } },
      arms: [{ id: 1, key: 'baseline', payload: null, baseline: true, traffic_share: null }, { id: 2, key: 'gift', payload: '<p>g</p>', baseline: false, traffic_share: share }],
    } },
  };
}
const run = (s, subject) => decide(s, { site: 'shop', slot: 'hero', subject }, { seed: 1n, at: Date.parse('2026-10-13T12:00:00Z'), sticky: null, picks: [], id: 'x' });

// Core stores holdout and shares as REAL and compares in f64 after widening the f32. These
// subjects were searched for (2^28 tries) to land between 0.3 and f32(0.3) = 0.30000001192...,
// where an f64 0.3 and Core disagree. A snapshot may carry the short form "0.3".
test('holdout compares against the f32 Core stores, not the decimal', () => {
  const s = 'f32-30816881';
  assert.equal(inHoldout('shop', s, 0.3), false, 'the vector no longer sits in the gap');
  assert.equal(inHoldout('shop', s, Math.fround(0.3)), true);
  assert.equal(run(snap({ holdout: 0.3 }), s).response.explanation.in_holdout, true);
});

test('a traffic share compares against the f32 Core stores', () => {
  const s = 'f32-319477712';
  assert.equal(withinShare(2, s, 0.3), false);
  assert.equal(withinShare(2, s, Math.fround(0.3)), true);
  assert.deepEqual(run(snap({ share: 0.3 }), s).record.pool, [1, 2]);
});
