// F2: score a cohort log's sorted grid against Core. Read-only.
//
//   PRB_DB=<postgres url> PRB_ADMIN_TOKEN=... node scripts/analyze-grid.mjs scripts/out/f2.jsonl [--core https://...]
//
// 1. Reconcile: for every grid decision in the log, Core's exposure and click events (by
//    decision id) must name exactly the items the script sent; a mismatch is listed.
// 2. Learning: the served top item per context (mobile/desktop) in each third of the run,
//    against the planted best (truth.mjs GRID_TRUTH: mobile -> kaapi-decoction, desktop ->
//    monday-espresso). Held-out decisions are left out of this.
// 3. Holdout: every held-out decision must have served the declared default order.
// 4. Explain: Core's explanation for the last treated mobile and desktop decision.
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { GRID_TRUTH } from './truth.mjs';

const file = process.argv[2];
const i = process.argv.indexOf('--core');
const CORE = i > 0 ? process.argv[i + 1] : 'http://localhost:8080';
const DEFAULT = JSON.parse(readFileSync(new URL('../site.json', import.meta.url))).slots.find((s) => s.key === 'grid').variants.map((v) => v.key);

const rows = readFileSync(file, 'utf8').trim().split('\n').map((l) => JSON.parse(l)).filter((r) => !r.header && r.grid);
const ids = rows.map((r) => r.grid.decision);
const sql = (q) => JSON.parse(execFileSync('psql', [process.env.PRB_DB, '-At', '-v', 'ON_ERROR_STOP=1', '-c', q], { maxBuffer: 1 << 28 }).toString().trim() || '[]');
const idList = ids.map((x) => `'${x}'`).join(',');
const decisions = new Map(sql(`SELECT coalesce(json_agg(json_build_object('id', d.id, 'holdout', d.in_holdout,
    'ranking', ARRAY(SELECT v.key FROM unnest(d.ranking) WITH ORDINALITY r(id, n) JOIN variants v ON v.id = r.id ORDER BY r.n))), '[]')
  FROM decisions d WHERE d.id IN (${idList})`).map((d) => [d.id, d]));
const events = sql(`SELECT coalesce(json_agg(json_build_object('decision', e.decision_id, 'name', e.name,
    'items', (SELECT coalesce(json_agg(it->>'id'), '[]') FROM jsonb_array_elements(coalesce(e.items, '[]')) it))), '[]')
  FROM events e WHERE e.decision_id IN (${idList}) AND e.name IN ('exposure', 'click')`);

// 1. Reconcile, per decision and per item.
const key = (d, n, items) => `${d}|${n}|${items.join(',')}`;
const want = new Map();
const bump = (m, k) => m.set(k, (m.get(k) ?? 0) + 1);
for (const r of rows) {
  if (r.grid.exposure?.status === 202 || r.grid.exposure?.status === 200) bump(want, key(r.grid.decision, 'exposure', r.grid.shown));
  if (r.grid.clicked && (r.grid.click?.status === 202 || r.grid.click?.status === 200)) bump(want, key(r.grid.decision, 'click', [r.grid.clicked]));
}
const got = new Map();
for (const e of events) bump(got, key(e.decision, e.name, e.items));
const diffs = [...new Set([...want.keys(), ...got.keys()])].filter((k) => want.get(k) !== got.get(k));
const perItem = (m, name) => {
  const out = {};
  for (const [k, n] of m) {
    const [, nm, items] = k.split('|');
    if (nm === name) for (const it of items.split(',')) out[it] = (out[it] ?? 0) + n;
  }
  return out;
};
console.log(`grid decisions in log: ${rows.length}; found in Core: ${decisions.size}`);
console.log('item exposures  log', perItem(want, 'exposure'), 'core', perItem(got, 'exposure'));
console.log('item clicks     log', perItem(want, 'click'), 'core', perItem(got, 'click'));
console.log(`reconcile: ${diffs.length} (decision, event, items) keys differ`, diffs.slice(0, 5));

// 2. Learning: top served item per context, by thirds of the run.
const best = (mobile) => Object.keys(GRID_TRUTH).sort((a, b) => GRID_TRUTH[b]({ mobile }) - GRID_TRUTH[a]({ mobile }))[0];
const treated = rows.filter((r) => decisions.get(r.grid.decision) && !decisions.get(r.grid.decision).holdout);
const third = Math.ceil(treated.length / 3);
for (const mobile of [true, false]) {
  const parts = [0, 1, 2].map((t) => {
    const xs = treated.slice(t * third, (t + 1) * third).filter((r) => r.ctx.mobile === mobile);
    const top = {};
    for (const r of xs) top[r.grid.top] = (top[r.grid.top] ?? 0) + 1;
    return `${Math.round((100 * (top[best(mobile)] ?? 0)) / Math.max(1, xs.length))}% of ${xs.length}`;
  });
  console.log(`${mobile ? 'mobile ' : 'desktop'} planted best ${best(mobile)} served first: thirds ${parts.join(' -> ')}`);
}

// 3. Holdout gets the declared default order (kept to the items in its pool).
const held = rows.filter((r) => decisions.get(r.grid.decision)?.holdout);
const wrong = held.filter((r) => {
  const rk = decisions.get(r.grid.decision).ranking;
  return rk.join() !== DEFAULT.filter((k) => rk.includes(k)).join() || rk.join() !== r.grid.shown.join();
});
console.log(`holdout: ${held.length} decisions, ${wrong.length} not in the default order`);

// 4. Explain the last treated decision per context.
for (const mobile of [true, false]) {
  const r = treated.filter((x) => x.ctx.mobile === mobile).at(-1);
  if (!r) continue;
  const res = await fetch(`${CORE}/api/decisions/${r.grid.decision}`, { headers: { authorization: `Bearer ${process.env.PRB_ADMIN_TOKEN}` } });
  const j = await res.json();
  console.log(`explain (${mobile ? 'mobile' : 'desktop'}, ${r.grid.decision}):`, JSON.stringify(j.explanation?.order ?? j.explanation ?? j).slice(0, 700));
}
