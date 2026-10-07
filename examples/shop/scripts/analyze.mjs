// Reads cohort.mjs logs and holds them against what Core says, over the admin API.
//
//   PRB_ADMIN_TOKEN=... node scripts/analyze.mjs --logs scripts/out/smoke.jsonl,scripts/out/run1.jsonl
//
// Prints: render health (served share, by idle gap), decisions (holdout share, cold start,
// policy), learning (share of bandit picks that were the planted winner among the arms the
// visitor was eligible for, by context and by quarter of the run), and a decision-by-decision
// reconciliation of exposures and clicks against Core's exposed_at and outcome. With --results
// it also recomputes the results page's counts and lift sequences from the log.
// Decision details are cached in --cache (default scripts/out/details.json): fetch once.

import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { TRUTH_TABLE } from './truth.mjs';

const arg = (name, dflt) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : dflt;
};
const CORE = process.env.PRB_CORE_URL ?? 'https://pratibimbaapi.meetdev.in';
const TOKEN = process.env.PRB_ADMIN_TOKEN;
if (!TOKEN) throw new Error('set PRB_ADMIN_TOKEN');
const LOGS = arg('logs', 'scripts/out/run1.jsonl').split(',');
const CACHE = arg('cache', 'scripts/out/details.json');
const REFRESH = process.argv.includes('--refresh');

const api = async (p) => {
  for (let attempt = 0; ; attempt++) {
    const r = await fetch(`${CORE}/api${p}`, { headers: { authorization: `Bearer ${TOKEN}` } });
    if (r.ok) return r.json();
    if (attempt >= 3) throw new Error(`${p} -> ${r.status}`);
    await new Promise((s) => setTimeout(s, 500 * (attempt + 1)));
  }
};

const visits = LOGS.flatMap((f) => readFileSync(f, 'utf8').trim().split('\n').map((l) => JSON.parse(l)).filter((r) => !r.header));
const served = visits.filter((r) => r.served);
const pct = (a, b) => (b ? `${((100 * a) / b).toFixed(1)}%` : '-');
const q = (xs, p) => xs.slice().sort((a, b) => a - b)[Math.min(xs.length - 1, Math.floor(p * xs.length))];

// ---- render health
console.log(`\n== Renders: ${visits.length} visits, ${served.length} served (${pct(served.length, visits.length)}), errors ${visits.filter((r) => r.error).length}`);
const ms = visits.filter((r) => r.render_ms).map((r) => r.render_ms);
console.log(`render ms p50 ${q(ms, 0.5)} p95 ${q(ms, 0.95)} max ${Math.max(...ms)}`);
const byGap = {};
for (const r of visits) {
  if (r.gap_ms === null) continue;
  const b = r.gap_ms < 1000 ? '<1s' : r.gap_ms < 4000 ? '1-4s' : r.gap_ms < 6000 ? '4-6s' : '>=6s';
  (byGap[b] ??= []).push(r);
}
for (const [b, rs] of Object.entries(byGap)) {
  const m = rs.map((r) => r.render_ms).filter(Boolean);
  console.log(`  gap ${b.padEnd(5)} n=${String(rs.length).padStart(5)} served ${pct(rs.filter((r) => r.served).length, rs.length).padStart(6)} render p50 ${q(m, 0.5)} p95 ${q(m, 0.95)}`);
}

// ---- details
let cache = existsSync(CACHE) && !REFRESH ? JSON.parse(readFileSync(CACHE, 'utf8')) : {};
const need = served.map((r) => r.decision).filter((d) => !cache[d] || !cache[d].settled_at);
for (let i = 0; i < need.length; i += 16) {
  const got = await Promise.all(need.slice(i, i + 16).map((d) => api(`/decisions/${d}`).catch((e) => ({ error: String(e) }))));
  need.slice(i, i + 16).forEach((d, j) => (cache[d] = got[j]));
}
writeFileSync(CACHE, JSON.stringify(cache));
const rows = served.map((r) => ({ r, d: cache[r.decision] }));
const missing = rows.filter((x) => !x.d || x.d.error);
console.log(`\n== Decisions: ${rows.length} served, ${missing.length} not found in Core`);
const mismatchVariant = rows.filter((x) => x.d && !x.d.error && x.d.variant !== x.r.variant);
console.log(`variant in page != variant in Core: ${mismatchVariant.length}`);
const ok = rows.filter((x) => x.d && !x.d.error);
const held = ok.filter((x) => x.d.in_holdout);
console.log(`holdout ${held.length}/${ok.length} = ${pct(held.length, ok.length)} (configured 10%); holdout variants: ${JSON.stringify(count(held.map((x) => x.d.variant)))}`);
const cold = ok.filter((x) => x.d.cold_start);
console.log(`cold start chose ${cold.length}: ${JSON.stringify(count(cold.map((x) => x.d.variant)))}; exposures seen by the last cold one: ${cold.at(-1)?.d.cold_start?.exposures}`);
console.log(`policies: ${JSON.stringify(count(ok.map((x) => `${x.d.policy} v${x.d.policy_version}`)))}`);
console.log(`replay_matches false: ${ok.filter((x) => x.d.replay_matches === false).length}`);
// What Core recorded about the visitor against what the script sent.
const featureMismatch = { mobile: 0, from_ad: 0, returning: 0, hour: 0, tz_missing: 0, tz_unexpected: 0 };
for (const { r, d } of ok) {
  const f = d.features;
  for (const k of ['mobile', 'from_ad', 'returning']) if (Boolean(f[k]) !== r.ctx[k]) featureMismatch[k]++;
  if (r.sends_tz && f.sin_hour === undefined) featureMismatch.tz_missing++;
  if (!r.sends_tz && f.sin_hour !== undefined) featureMismatch.tz_unexpected++;
  if (r.sends_tz && f.sin_hour !== undefined) {
    // The render may cross an hour boundary between the script's clock and Core's: allow either.
    const ok1 = [r.local_hour, (r.local_hour + 1) % 24].some((h) => Math.abs(Math.sin((2 * Math.PI * h) / 24) - f.sin_hour) < 1e-9 && Math.abs(Math.cos((2 * Math.PI * h) / 24) - f.cos_hour) < 1e-9);
    if (!ok1) featureMismatch.hour++;
  }
}
console.log(`features Core recorded vs the script's visitor (mismatches): ${JSON.stringify(featureMismatch)}`);

// ---- learning: bandit picks (not holdout, not cold start) vs the planted winner among eligible arms
const bandit = ok.filter((x) => !x.d.in_holdout && !x.d.cold_start).sort((a, b) => a.d.created_at.localeCompare(b.d.created_at));
const ctxKey = (c) => (c.returning ? 'returning' : c.mobile ? (c.from_ad ? 'mobile+ad' : 'mobile') : c.from_ad ? 'desktop+ad' : c.evening ? 'desktop evening' : 'desktop other');
const best = (x) => {
  const elig = x.d.scores.map((s) => s.variant);
  const p = Object.fromEntries(elig.map((v) => [v, TRUTH_TABLE[v](x.r.ctx)]));
  const top = Math.max(...Object.values(p));
  return { elig, p, winners: elig.filter((v) => p[v] === top), alone: elig.length === 1 };
};
console.log(`\n== Learning: ${bandit.length} bandit decisions; "hit" = chose a planted winner among the arms it could show (decisions with one eligible arm excluded)`);
const quarters = [0, 1, 2, 3].map((k) => bandit.slice(Math.floor((k * bandit.length) / 4), Math.floor(((k + 1) * bandit.length) / 4)));
const header = ['context', ...quarters.map((_, k) => `Q${k + 1}`), 'random'];
console.log(header.map((h) => h.padEnd(18)).join(''));
for (const c of ['returning', 'mobile', 'mobile+ad', 'desktop+ad', 'desktop evening', 'desktop other', 'ALL']) {
  const cells = quarters.map((qs) => {
    const xs = qs.filter((x) => (c === 'ALL' || ctxKey(x.r.ctx) === c)).map((x) => ({ x, b: best(x) })).filter((y) => !y.b.alone);
    return `${pct(xs.filter((y) => y.b.winners.includes(y.x.d.variant)).length, xs.length)} (${xs.length})`;
  });
  const all = bandit.filter((x) => c === 'ALL' || ctxKey(x.r.ctx) === c).map((x) => best(x)).filter((b) => !b.alone);
  const rnd = all.reduce((s, b) => s + b.winners.length / b.elig.length, 0) / (all.length || 1);
  console.log([c, ...cells, `${(100 * rnd).toFixed(1)}%`].map((h) => String(h).padEnd(18)).join(''));
}
console.log('\nper context, variant chosen when eligible (last half of the run):');
const late = bandit.slice(Math.floor(bandit.length / 2));
for (const c of ['returning', 'mobile', 'mobile+ad', 'desktop+ad', 'desktop evening', 'desktop other']) {
  const xs = late.filter((x) => ctxKey(x.r.ctx) === c);
  const parts = Object.keys(TRUTH_TABLE).map((v) => {
    const e = xs.filter((x) => x.d.scores.some((s) => s.variant === v) && x.d.scores.length > 1);
    return `${v} ${pct(e.filter((x) => x.d.variant === v).length, e.length)} of ${e.length}`;
  });
  console.log(`  ${c.padEnd(16)} ${parts.join(' | ')}`);
}

// ---- reconciliation, decision by decision
const settled = ok.filter((x) => x.d.settled_at || x.d.outcome !== null);
const recon = { exposure_sent_not_exposed: 0, exposed_not_sent: 0, click_sent_not_converted: 0, converted_no_click: 0, unsettled: ok.length - settled.length };
for (const { r, d } of ok) {
  const sent = r.exposure?.status === 202 && r.exposure.accepted === 1;
  if (sent && !d.exposed_at) recon.exposure_sent_not_exposed++;
  if (!sent && d.exposed_at) recon.exposed_not_sent++;
  const clicked = r.click?.status === 202 && r.click.accepted === 1;
  if (d.settled_at || d.outcome !== null) {
    if (clicked && d.outcome !== 1) recon.click_sent_not_converted++;
    if (!clicked && d.outcome === 1) recon.converted_no_click++;
  }
}
console.log(`\n== Reconciliation (log vs Core, per decision): ${JSON.stringify(recon)}`);
console.log(`log: exposures accepted ${ok.filter((x) => x.r.exposure?.accepted === 1).length}, clicks accepted ${ok.filter((x) => x.r.click?.accepted === 1).length}; Core: exposed ${ok.filter((x) => x.d.exposed_at).length}, converted ${ok.filter((x) => x.d.outcome === 1).length}`);

// ---- results page, recomputed from the log
if (process.argv.includes('--results')) {
  const res = await api('/sites/shop/results');
  const hero = res.results.slots.find((s) => s.key === 'hero');
  const pre = existsSync(arg('pre', 'scripts/out/pre-results.json')) ? JSON.parse(readFileSync(arg('pre', 'scripts/out/pre-results.json'), 'utf8')) : null;
  const preHero = pre?.results.slots.find((s) => s.key === 'hero');
  const mine = (pred) => {
    const xs = ok.filter((x) => x.d.exposed_at && pred(x));
    return { n: xs.length, conversions: xs.filter((x) => x.d.outcome === 1).length, clicks_in_log: xs.filter((x) => x.r.click?.accepted === 1).length };
  };
  console.log('\n== Results page vs log (Core minus the pre-run snapshot)');
  const line = (name, core, preC, m) => console.log(`  ${name.padEnd(14)} Core n=${core.n - (preC?.n ?? 0)} conv=${core.conversions - (preC?.conversions ?? 0)} credit=${(core.credit - (preC?.credit ?? 0)).toFixed(2)} | log n=${m.n} conv=${m.conversions} clicks=${m.clicks_in_log}`);
  line('holdout', hero.holdout, preHero?.holdout, mine((x) => x.d.in_holdout));
  line('treated', hero.treated, preHero?.treated, mine((x) => !x.d.in_holdout));
  for (const v of hero.variants) line(v.key, v.counts, preHero?.variants.find((p) => p.key === v.key)?.counts, mine((x) => !x.d.in_holdout && x.d.variant === v.key));
  console.log(`  Core lift ${JSON.stringify(hero.lift)}`);
  console.log(`  recomputed from Core's own counts: ${JSON.stringify(liftCs([hero.treated.credit, hero.treated.n], [hero.holdout.credit, hero.holdout.n]))}`);
}

function count(xs) {
  const m = {};
  for (const x of xs) m[x] = (m[x] ?? 0) + 1;
  return m;
}

// stats.rs, ported: the asymptotic confidence sequence, alpha/2 per group, Minkowski difference.
function liftCs([tc, tn], [cc, cn], alpha = 0.05) {
  const rho2 = (a) => {
    const l = -2 * Math.log(a);
    return (l + Math.log(l + 1)) / 100;
  };
  const hw = (n, v, a) => {
    const r2 = rho2(a);
    const w = n * v * r2 + 1;
    return Math.sqrt(((2 * w) / (n * n * r2)) * Math.log(Math.sqrt(w) / a));
  };
  const cs = (c, n, a) => {
    const p = c / n;
    const h = hw(n, Math.max(p * (1 - p), 1e-4), a);
    return { estimate: p, lo: Math.max(0, p - h), hi: Math.min(1, p + h) };
  };
  if (!tn || !cn) return null;
  const t = cs(tc, tn, alpha / 2);
  const c = cs(cc, cn, alpha / 2);
  return { estimate: t.estimate - c.estimate, lo: t.lo - c.hi, hi: t.hi - c.lo };
}
