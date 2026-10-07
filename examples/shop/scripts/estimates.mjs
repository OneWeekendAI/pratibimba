// What the hero's model believes now, held against the planted truth (truth.mjs), for the
// visitors of a cohort.mjs log. Reads the live model and the decisions straight from Postgres
// (psql on PATH; the URL comes from PRB_DB_URL and is never printed).
//
//   PRB_DB_URL=... node scripts/estimates.mjs --logs scripts/out/e5.jsonl [--slot 3] [--json]
//
// For every served, treated decision in the log it computes the model's posterior-mean click
// rate for each arm the slot has, at that decision's stored features, exactly as Core's
// explanations do (stated::mean_score: the base arm, or the extension arm over the time
// features). Per context and arm it prints the mean estimate, the mean planted rate, and the
// empirical click rate of the shown decisions whose windows have closed. It also prints how
// much the model has learned against how many treated, exposed windows have closed: after the
// E4 fix the two are equal, and an open window teaches nothing.

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { TRUTH_TABLE } from './truth.mjs';

const arg = (name, dflt) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : dflt;
};
const DB = process.env.PRB_DB_URL;
if (!DB) throw new Error('set PRB_DB_URL');
const SLOT = Number(arg('slot', '3'));
const LOGS = arg('logs', 'scripts/out/e5.jsonl').split(',');
const JSON_OUT = process.argv.includes('--json');

const sql = (q) => execFileSync('psql', [DB, '-Atc', q], { encoding: 'utf8', maxBuffer: 1 << 28 }).trim();

const visits = LOGS.flatMap((f) => readFileSync(f, 'utf8').trim().split('\n').map((l) => JSON.parse(l)).filter((r) => !r.header && r.served && r.decision));
const ids = visits.map((v) => `'${v.decision}'`).join(',') || "'00000000-0000-0000-0000-000000000000'";

const model = JSON.parse(sql(`SELECT json_build_object('version', version, 'state', state, 'now', now()) FROM models WHERE slot_id = ${SLOT}`));
const arms = JSON.parse(sql(`SELECT json_agg(json_build_object('id', id, 'key', key) ORDER BY id) FROM variants WHERE slot_id = ${SLOT} AND status = 'approved'`));
const window = Number(sql(`SELECT attribution_window_secs FROM slots WHERE id = ${SLOT}`));
const rows = JSON.parse(
  sql(`SELECT coalesce(json_agg(json_build_object('id', d.id, 'variant', d.variant_id, 'features', d.features, 'held', d.in_holdout,
         'exposed', d.exposed_at IS NOT NULL, 'outcome', d.outcome, 'closed', d.created_at + make_interval(secs => ${window}) < now(),
         'pending', d.learn_pending)), '[]') FROM decisions d WHERE d.id IN (${ids})`),
);
const byId = new Map(rows.map((r) => [r.id, r]));

// ---- Core's posterior mean (bandit.rs / stated.rs), in JS
const sigmoid = (z) => 1 / (1 + Math.exp(-z));
const dot = (a, b) => a.reduce((s, x, i) => s + x * (b[i] ?? 0), 0);
const BASE = ['bias', 'returning', 'mobile', 'from_ad'];
const TIME = ['sin_hour', 'cos_hour', 'is_night_local', 'is_weekend_local', 'is_business_hours_local'];
const state = model.state ?? {};
function meanRate(armId, f) {
  const x = BASE.map((n) => f[n] ?? 0);
  const ext = Object.fromEntries(TIME.filter((n) => typeof f[n] === 'number').map((n) => [`time:${n}`, f[n]]));
  const base = state.arms?.[armId]?.mean ?? [0, 0, 0, 0];
  if (!Object.keys(ext).length) return sigmoid(dot(base, x));
  // model_for: the stored extension (keys only ever appended), then any new key; an arm the
  // extension has not seen is the base arm with zeros on the new dimensions.
  const keys = [...(state.stated?.keys ?? [])];
  const stored = state.stated?.arms?.[armId];
  const dim = 4 + keys.length;
  const mean = stored && stored.mean.length === dim ? stored.mean : [...base, ...keys.map(() => 0)];
  const v = [...x, ...keys.map((k) => ext[k] ?? 0)];
  return sigmoid(dot(mean, v));
}

// ---- contexts: the planted winners' segments (truth.mjs)
const context = (c) => (c.returning ? 'returning' : c.from_ad ? (c.mobile ? 'mobile ad' : 'desktop ad') : c.mobile ? 'mobile new' : c.evening ? 'desktop evening' : 'desktop other');
const cells = new Map();
let treated = 0, closedTreated = 0, pendingCredits = 0;
for (const v of visits) {
  const d = byId.get(v.decision);
  if (!d || d.held || !d.exposed) continue;
  treated++;
  if (d.closed) closedTreated++;
  if (d.pending) pendingCredits++;
  const ctx = context(v.ctx);
  for (const a of arms) {
    const key = `${ctx}|${a.key}`;
    const c = cells.get(key) ?? { ctx, arm: a.key, n: 0, est: 0, truth: 0, shown: 0, shownClosed: 0, conv: 0 };
    c.n++;
    c.est += meanRate(a.id, d.features);
    c.truth += TRUTH_TABLE[a.key](v.ctx);
    if (d.variant === a.id) {
      c.shown++;
      if (d.closed) {
        c.shownClosed++;
        c.conv += d.outcome === 1 ? 1 : 0;
      }
    }
    cells.set(key, c);
  }
}

const n = state.n ?? 0;
const out = {
  at: model.now,
  version: model.version,
  learned: n,
  treated_exposed: treated,
  treated_exposed_closed: closedTreated,
  credits_waiting_for_close: pendingCredits,
  cells: [...cells.values()].map((c) => ({
    ctx: c.ctx,
    arm: c.arm,
    est: c.est / c.n,
    truth: c.truth / c.n,
    shown: c.shown,
    closed: c.shownClosed,
    empirical: c.shownClosed ? c.conv / c.shownClosed : null,
  })),
};
if (JSON_OUT) {
  console.log(JSON.stringify(out));
} else {
  console.log(`model v${out.version}: learned ${n} | treated exposed ${treated}, windows closed ${closedTreated}, credits waiting for close ${pendingCredits}`);
  const pct = (x) => (x == null ? '   -  ' : `${(100 * x).toFixed(1).padStart(5)}%`);
  console.log(`${'context'.padEnd(16)} ${'arm'.padEnd(14)} estimate  planted  empirical (closed shown)`);
  for (const c of out.cells.sort((a, b) => a.ctx.localeCompare(b.ctx) || a.arm.localeCompare(b.arm))) {
    console.log(`${c.ctx.padEnd(16)} ${c.arm.padEnd(14)} ${pct(c.est)}   ${pct(c.truth)}  ${pct(c.empirical)} (${c.closed}/${c.shown})`);
  }
}
