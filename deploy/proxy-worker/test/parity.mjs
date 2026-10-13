// Runs the edge kernel over cases Core decided (core/tests/edge_parity.rs writes them) and
// compares. Usage: node test/parity.mjs <cases.jsonl>. Prints one JSON line:
// {cases, decided, mismatches, examples, coverage, zones_match}. Exits 0 either way; the caller
// asserts. The first line of the file is {zones: [...]}, Core's accepted zone names.
//
// Also used by test/vectors.test.mjs on the committed subset (test/vectors.jsonl).

import fs from 'node:fs';
import { decide } from '../src/kernel.js';
import { ZONES } from '../src/tables.js';

const TOL = 1e-12;

/** JSON with object keys sorted, so key order (serde_json's vs ours) never counts as a difference. */
function canon(v) {
  if (Array.isArray(v)) return `[${v.map(canon).join(',')}]`;
  if (v && typeof v === 'object') return `{${Object.keys(v).sort().map((k) => `${JSON.stringify(k)}:${canon(v[k])}`).join(',')}}`;
  return JSON.stringify(v);
}

function close(a, b) {
  return a === b || Math.abs(a - b) <= TOL * Math.max(1, Math.abs(a));
}

/** The Worker's own sticky record from Core's `x-prb-edge` record (see src/index.js `fromCore`). */
export function stickyFrom(rec) {
  if (!rec) return null;
  return { ...rec, created_at_ms: Date.parse(rec.created_at) };
}

/** Compare one kernel response with Core's; returns a list of differences. */
export function diff(core, edge, sticky) {
  const out = [];
  const eq = (path, a, b) => {
    if (canon(a) !== canon(b)) out.push(`${path}: core ${JSON.stringify(a)} edge ${JSON.stringify(b)}`);
  };
  if (sticky) eq('decision_id', core.decision_id, edge.decision_id);
  eq('slot', core.slot, edge.slot);
  eq('variant', core.variant, edge.variant);
  eq('payload', core.payload, edge.payload);
  const ce = core.explanation;
  const ee = edge.explanation;
  for (const k of ['policy', 'policy_version', 'seed', 'in_holdout', 'audiences', 'cold_start', 'allocation', 'experiment']) eq(k, ce[k], ee[k]);
  const fk = Object.keys(ce.features).sort();
  eq('feature names', fk, Object.keys(ee.features).sort());
  // Features involve no exp or log (sin and cos come from Core's own table), so they match exactly.
  for (const k of fk) if (ce.features[k] !== ee.features[k]) out.push(`features.${k}: core ${ce.features[k]} edge ${ee.features[k]}`);
  const sk = Object.keys(ce.scores).sort();
  eq('score names', sk, Object.keys(ee.scores).sort());
  for (const k of sk) if (!close(ce.scores[k], ee.scores[k])) out.push(`scores.${k}: core ${ce.scores[k]} edge ${ee.scores[k]}`);
  return out;
}

const bump = (c, k) => { c[k] = (c[k] || 0) + 1; };

/** Run every case; `report` is what the Rust test reads. */
export function run(lines) {
  const head = JSON.parse(lines[0]);
  const zonesMatch = head.zones.length === ZONES.size && head.zones.every((z) => ZONES.has(z));
  const coverage = {};
  const examples = [];
  let cases = 0;
  let mismatches = 0;
  let exactScores = 0;
  let nearScores = 0;
  const exactBy = { base: [0, 0], extended: [0, 0] };
  for (const line of lines.slice(1)) {
    if (!line.trim()) continue;
    cases++;
    const c = JSON.parse(line);
    const at = Date.parse(c.at);
    const batch = c.path === '/decide/batch';
    const slots = batch ? c.request.slots : [c.request.slot];
    if (batch) bump(coverage, 'batch');
    const coreOut = c.core.status === 200 ? (batch ? c.core.body.decisions : [c.core.body]) : null;
    const fail = (why) => {
      mismatches++;
      if (examples.length < 4) examples.push({ case: cases, why, request: c.request, explanation: c.core.body && (c.core.body.explanation || c.core.body) });
    };
    if (!coreOut) {
      // Core refused the whole request: the edge must not decide any of it.
      bump(coverage, 'core_refused');
      for (const slot of slots) {
        const req = { site: c.request.site, slot, subject: c.request.subject, ctx: c.request.ctx };
        const k = decide(c.snapshot, req, { stored: c.stored, seed: 1n, at, sticky: null, picks: [], id: 'x' });
        if (k.ok) fail(`Core answered ${c.core.status} but the edge decided ${k.response.variant}`);
      }
      continue;
    }
    slots.forEach((slot, i) => {
      const core = coreOut[i];
      const req = { site: c.request.site, slot, subject: c.request.subject, ctx: c.request.ctx };
      const sticky = stickyFrom(c.sticky[i]);
      if (!core.decision_id) {
        // A per-slot error inside a batch.
        bump(coverage, 'core_refused');
        const k = decide(c.snapshot, req, { stored: c.stored, seed: 1n, at, sticky, picks: [], id: 'x' });
        if (k.ok) fail(`Core refused slot ${slot} (${core.error}) but the edge decided`);
        return;
      }
      const reused = !!sticky && sticky.id === core.decision_id;
      const k = decide(c.snapshot, req, { stored: c.stored, seed: BigInt(core.explanation.seed), at, sticky, picks: [], id: core.decision_id });
      if (!k.ok) {
        fail(`edge declined (${k.reason}) a decision Core made`);
        return;
      }
      const d = diff(core, k.response, reused);
      if (d.length) {
        fail(d.join('; '));
        return;
      }
      if (reused !== (k.record === null)) {
        fail(`sticky: core reused=${reused}, edge reused=${k.record === null}`);
        return;
      }
      bump(coverage, 'decided');
      const ex = core.explanation;
      const group = ex.policy === 'lints' ? exactBy.base : exactBy.extended;
      for (const key of Object.keys(ex.scores)) {
        const same = ex.scores[key] === k.response.explanation.scores[key];
        if (same) exactScores++;
        else nearScores++;
        group[same ? 0 : 1]++;
      }
      if (reused) bump(coverage, 'sticky');
      if (c.identified) bump(coverage, 'identified');
      else bump(coverage, 'cookieless');
      if (ex.policy === 'lints') bump(coverage, 'base_lints');
      if (ex.policy === 'lints_stated') bump(coverage, 'stated');
      if (ex.policy === 'lints_features') bump(coverage, 'features');
      if ('sin_hour' in ex.features) bump(coverage, 'time');
      if (ex.in_holdout) bump(coverage, 'holdout');
      if (ex.allocation === 'experiment') bump(coverage, 'experiment');
      if (ex.cold_start) bump(coverage, 'cold_start');
      if (ex.audiences.some((a) => !a.matched)) bump(coverage, 'gate_shut');
      if (ex.audiences.some((a) => a.matched)) bump(coverage, 'gate_open');
      if (c.snapshot.settings.paused) bump(coverage, 'paused');
      if (c.request.ctx && c.request.ctx.tz) bump(coverage, 'zone');
      const snapSlot = c.snapshot.slots[slot];
      if (k.record && k.record.pool.length < snapSlot.arms.length) bump(coverage, 'share_excluded');
      // The edge's own clock rule: past the experiment's end or the next one's start it asks Core.
      // A paused site ignores experiments, so its clock does not matter.
      const edges = c.snapshot.settings.paused ? [] : [snapSlot.experiment && snapSlot.experiment.ends_at, snapSlot.next_experiment_at].filter(Boolean).map(Date.parse);
      for (const t of edges) {
        const late = decide(c.snapshot, req, { stored: c.stored, seed: 1n, at: t, sticky: null, picks: [], id: 'x' });
        if (late.ok || late.reason !== 'experiment-clock') fail(`at the experiment boundary the edge still decided (${late.reason})`);
        else bump(coverage, 'experiment_clock');
      }
    });
  }
  // Scores may differ from Core's in the last bit only through Math.exp and Math.log, which the
  // 4-dimension policy and the extended one call alike. So the extended policy's share of
  // inexact scores must stay near the base policy's: an arithmetic-order slip in the extended
  // sampler (summing a row before adding it, or not) roughly triples it (4% -> 14% measured),
  // while staying inside the 1e-12 tolerance every single comparison allows.
  const rate = ([same, diff]) => diff / Math.max(1, same + diff);
  const orderOk = rate(exactBy.extended) <= 2 * rate(exactBy.base) + 0.01;
  return { cases, mismatches, examples, coverage, zones_match: zonesMatch, order_ok: orderOk, scores_exact: exactScores, scores_within_tolerance: nearScores, inexact_by_policy: exactBy };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const lines = fs.readFileSync(process.argv[2], 'utf8').split('\n');
  console.log(JSON.stringify(run(lines)));
}
