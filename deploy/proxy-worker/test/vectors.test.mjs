// The kernel against decisions Core made: a committed subset of core/tests/edge_parity.rs's cases
// (regenerate with PRB_WRITE_VECTORS=1 cargo test --test edge_parity), so this suite checks
// parity with no database. The full run, thousands of cases, is the Rust test.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { run } from './parity.mjs';

test('the edge kernel makes the decisions Core made (committed vectors)', () => {
  const lines = fs.readFileSync(new URL('./vectors.jsonl', import.meta.url), 'utf8').split('\n');
  const r = run(lines);
  assert.ok(r.cases >= 80, `only ${r.cases} vectors`);
  assert.equal(r.mismatches, 0, JSON.stringify(r.examples, null, 1));
  assert.equal(r.zones_match, true, 'src/tables.js ZONES differs from the list Core accepts');
  for (const branch of ['decided', 'holdout', 'experiment', 'cold_start', 'gate_shut', 'sticky', 'cookieless', 'identified', 'core_refused', 'time', 'stated', 'features']) {
    assert.ok((r.coverage[branch] || 0) > 0, `no vector takes branch ${branch}: ${JSON.stringify(r.coverage)}`);
  }
});
