// Builds demo/personas.html and demo/proof.html (static, no Core needed to view) from the JSON the other scripts wrote:
// demo/personas.json (scripts/personas.mjs), proof/sim-haat.json + proof/sim-haatctl.json (scripts/simulate.mjs),
// and a daily treated-vs-holdout series read from the DB.
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const here = import.meta.dirname, rd = (p) => JSON.parse(readFileSync(path.join(here, '..', p), 'utf8'));
const DB = process.env.PRB_DB ?? 'postgres://postgres@localhost:55440/prb_local';
const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
const personas = rd('demo/personas.json'), sim = rd('proof/sim-haat.json'), ctl = rd('proof/sim-haatctl.json'), truth = rd('truth.json');

const CSS = `
:root{--bg:#f6f5f1;--ink:#16211f;--mute:#5d6b68;--line:#dcdad2;--teal:#0a6a61;--pink:#d6336c;--card:#fff}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--ink);font:15px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif}
header{padding:28px 32px 8px;max-width:1360px;margin:0 auto}h1{font-size:28px;margin:0 0 4px;letter-spacing:-.01em}
.sub{color:var(--mute);margin:0 0 8px;max-width:70ch}main{max-width:1360px;margin:0 auto;padding:8px 32px 56px}
a{color:var(--teal)}nav a{margin-right:16px;font-size:14px}
.grid{display:grid;grid-template-columns:repeat(4,1fr);gap:20px;margin-top:16px}
@media(max-width:1100px){.grid{grid-template-columns:repeat(2,1fr)}}@media(max-width:620px){.grid{grid-template-columns:1fr}header,main{padding-left:16px;padding-right:16px}}
.p{background:var(--card);border:1px solid var(--line);border-radius:14px;overflow:hidden;display:flex;flex-direction:column}
.p h2{font-size:17px;margin:0}.p .ab{color:var(--mute);font-size:13px;margin:2px 0 0}.p .hd{padding:14px 16px 10px}
.shot{background:#e9e7df;border-top:1px solid var(--line);border-bottom:1px solid var(--line);height:360px;overflow:hidden}
.shot img{width:100%;display:block;object-position:top}
.chips{padding:12px 16px;display:flex;flex-wrap:wrap;gap:6px}
.chip{font-size:12px;border:1px solid var(--line);border-radius:999px;padding:2px 9px;background:#faf9f6}.chip b{font-weight:600}
details{padding:0 16px 14px;font-size:13px;color:var(--mute)}summary{cursor:pointer;color:var(--teal);font-weight:600}
.big{font-size:64px;font-weight:700;letter-spacing:-.03em;line-height:1}.cap{color:var(--mute);margin-top:6px}
.row{display:grid;grid-template-columns:1fr 1fr;gap:20px;margin:20px 0}@media(max-width:800px){.row{grid-template-columns:1fr}}
.box{background:var(--card);border:1px solid var(--line);border-radius:14px;padding:20px 22px}
table{border-collapse:collapse;width:100%;font-size:13px}th,td{padding:6px 8px;border-bottom:1px solid var(--line);text-align:left}th{color:var(--mute);font-weight:600}
.ok{color:var(--teal);font-weight:700}.no{color:var(--pink);font-weight:700}
`;
const shell = (title, body, here2) => `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${title}</title><style>${CSS}</style></head><body>
<header><nav><a href="/demo/personas.html"${here2 === 'p' ? ' style="font-weight:700"' : ''}>Four shoppers</a><a href="/demo/proof.html"${here2 === 'r' ? ' style="font-weight:700"' : ''}>Proof</a><a href="/">Open the store</a></nav>${body}`;

// ---- personas
const label = (k) => k.replace(/-/g, ' ');
const pcards = personas.map((p) => {
  const chips = [['hero', p.variants.hero], ['promo', p.variants['promo-bar']], ['sort', p.variants['listing-sort']], ['badge', p.variants['product-badge']], ['delivery', p.variants['delivery-message']], ['buy', p.variants['buy-button']]]
    .map(([k, v]) => `<span class="chip">${k}: <b>${esc(v)}</b></span>`).join('') + `<span class="chip">first row: <b>${esc(p.home.rows[0])}</b></span>`;
  return `<section class="p"><div class="hd"><h2>${esc(p.name)}</h2><p class="ab">${esc(p.about)}</p></div>
<div class="shot"><img src="/proof/personas/${p.key}-home.png" alt="Big Market home as seen by ${esc(p.name)}"></div>
<div class="chips">${chips}</div>
<details><summary>Why this hero?</summary><p>${esc(p.why.hero)}</p></details>
<details><summary>Why this badge?</summary><p>${esc(p.why['product-badge'])}</p></details></section>`;
}).join('');
writeFileSync(path.join(here, '../demo/personas.html'), shell('Big Market - four shoppers', `<h1>Same store, four shoppers</h1><p class="sub">Each column is a real browser visit to the live store (phone or desktop, ad or search or direct, new or returning, morning or evening) as Pratibimba served it. Nothing here is mocked: the chips are the decided variants and the notes are Core's own explanation of each decision.</p></header><main><div class="grid">${pcards}</div></main></body></html>`, 'p'));

// ---- proof
const series = execFileSync('psql', [DB, '-qAt', '-F', '|', '-c', `SELECT date_trunc('day', created_at)::date, in_holdout, count(*), avg(CASE WHEN outcome > 0 THEN 1 ELSE 0 END) FROM decisions WHERE site_id = 'haat' AND subject LIKE 'sim-%' AND exposed_at IS NOT NULL AND settled_at IS NOT NULL GROUP BY 1, 2 ORDER BY 1, 2`]).toString().trim().split('\n').map((l) => l.split('|'));
const days = [...new Set(series.map((r) => r[0]))];
const at = (d, h) => { const r = series.find((x) => x[0] === d && (x[1] === 't') === h); return r ? +r[3] : null; };
const T = days.map((d) => at(d, false)), Hh = days.map((d) => at(d, true));
const W = 560, Ht = 220, L = 44, B = 28, mx = Math.max(...T.filter(Boolean), ...Hh.filter(Boolean)) * 1.15;
const X = (i) => L + (i * (W - L - 12)) / Math.max(1, days.length - 1), Y = (v) => Ht - B - (v / mx) * (Ht - B - 12);
const path1 = (a) => a.map((v, i) => (v == null ? '' : `${i && a[i - 1] != null ? 'L' : 'M'}${X(i).toFixed(1)},${Y(v).toFixed(1)}`)).join('');
const chart = `<svg viewBox="0 0 ${W} ${Ht}" width="100%" style="max-width:720px" role="img" aria-label="Conversion per day, treated against holdout">
${[0, 0.5, 1].map((f) => `<line x1="${L}" x2="${W - 12}" y1="${Y(mx * f / 1.15)}" y2="${Y(mx * f / 1.15)}" stroke="#dcdad2"/><text x="${L - 6}" y="${Y(mx * f / 1.15) + 4}" font-size="11" text-anchor="end" fill="#5d6b68">${(100 * mx * f / 1.15).toFixed(0)}%</text>`).join('')}
<path d="${path1(Hh)}" fill="none" stroke="#9aa5a2" stroke-width="2.5" stroke-dasharray="5 4"/><path d="${path1(T)}" fill="none" stroke="#0a6a61" stroke-width="3"/>
${days.map((d, i) => `<text x="${X(i)}" y="${Ht - 8}" font-size="11" text-anchor="middle" fill="#5d6b68">${d.slice(5)}</text>`).join('')}
<text x="${L + 8}" y="${Y(T[0]) - 12}" font-size="12" text-anchor="start" fill="#0a6a61" font-weight="700">with Pratibimba</text><text x="${L + 8}" y="${Y(Hh[0] ?? 0) + 22}" font-size="12" text-anchor="start" fill="#6b7673">holdout (no personalisation)</text></svg>`;
const secondHalf = Object.values(sim.uplift_second_half), pool = (k) => secondHalf.reduce((a, u) => a + (u[k]?.conversion ?? 0) * (u[k]?.n ?? 0), 0) / secondHalf.reduce((a, u) => a + (u[k]?.n ?? 0), 0);
const tr = pool('treated'), ho = pool('holdout');
const scalar = ['hero', 'listing-sort', 'product-badge', 'buy-button', 'delivery-message', 'promo-bar'];
const segs = Object.keys(truth.mix);
const cell = (g) => `<td>${g.found ? '<span class="ok">found</span>' : '<span class="no">missed</span>'} <span style="color:#5d6b68">${esc(g.planted ?? '')}${g.kind !== 'sorted' && !g.found ? ' (served ' + esc(g.served_modal) + ')' : ''}</span></td>`;
const rowsHtml = [...scalar, 'category-rows', 'picks', 'deals', 'cart-cross-sell'].map((s) => `<tr><th>${label(s)}</th>${segs.map((g) => cell(sim.graded.find((x) => x.slot === s && x.segment === g))).join('')}</tr>`).join('');
writeFileSync(path.join(here, '../demo/proof.html'), shell('Big Market - proof', `<h1>Did it find what we planted?</h1><p class="sub">We gave four kinds of shopper a secret preference for each of ten places on the store, then ran ${sim.visitors.toLocaleString()} synthetic visitors through the live Core. Afterwards we asked it what it would show each kind, and compared with the secret list.</p></header><main>
<div class="row"><div class="box"><div class="big">${sim.found} of ${sim.cells}</div><div class="cap">planted winners found. A control run with <b>no</b> preference planted: ${ctl.found} of ${ctl.cells}.</div></div>
<div class="box"><div class="big">${(100 * tr).toFixed(1)}% <span style="font-size:28px;color:#5d6b68">vs ${(100 * ho).toFixed(1)}%</span></div><div class="cap">conversion per exposure once it had learned, against the holdout who saw the plain store (${tr > ho ? '+' + ((100 * (tr - ho)).toFixed(1)) + ' points' : 'no gain'}).</div></div></div>
<div class="box"><h2 style="margin:0 0 8px;font-size:17px">Conversion per day, with and without</h2>${chart}</div>
<div class="box" style="margin-top:20px;overflow-x:auto"><h2 style="margin:0 0 8px;font-size:17px">Planted against served</h2><table><tr><th></th>${segs.map((g) => `<th>${g} (${esc(truth.segments[g].sig)})</th>`).join('')}</tr>${rowsHtml}</table></div>
</main></body></html>`, 'r'));
console.log('demo pages written;', days.length, 'days; treated', tr.toFixed(4), 'holdout', ho.toFixed(4));
