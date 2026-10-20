// EC4, measured live: erase a subject while traffic for it (and for others) is flowing through the
// Worker, and report how long after the erase answer the edge stops serving it (the promise is
// 60 s). Every WORKERS visitors run decide -> exposure -> click loops (consented, so they are
// identified and stored); visitor 0 is erased after WARM_S seconds and keeps sending for
// AFTER_S seconds more. Prints the erase answer's edge block, the lag to the first 410, any
// non-410 answer after that, and the 410's source (edge KV/PoP hint vs Core). The stored-rows
// check is separate (rows for the subject must be zero once the queue drains): this prints the
// subject and the erase time for it.
// Usage: PRB_ADMIN_TOKEN=... node deploy/checks/edge-erase.mjs <base-url> [site] [slot]
import { randomUUID } from 'node:crypto';
const [base, site = 'shop', slot = 'hero'] = process.argv.slice(2);
const token = process.env.PRB_ADMIN_TOKEN;
if (!base || !token) throw new Error('usage: PRB_ADMIN_TOKEN=... node deploy/checks/edge-erase.mjs <base-url> [site] [slot]');
const WORKERS = Number(process.env.WORKERS || 6), WARM_S = Number(process.env.WARM_S || 15), AFTER_S = Number(process.env.AFTER_S || 75);
const tag = `ec4-erase-${Date.now()}`;
const victim = `${tag}-v0`;
const post = (p, body, admin) => fetch(base + p, {
  method: 'POST',
  headers: { 'content-type': 'application/json', ...(admin ? { authorization: `Bearer ${token}` } : {}) },
  body: JSON.stringify(body),
});
const log = [];
let stop = false;
async function visitor(i) {
  const subject = i === 0 ? victim : `${tag}-v${i}`;
  while (!stop) {
    const t = Date.now();
    try {
      const r = await post('/decide', { site, slot, subject, ctx: { consent: true } });
      const rec = { i, t, status: r.status, by: r.headers.get('x-prb-decided-by'), erasedHdr: r.headers.get('x-prb-erased'), why: r.headers.get('x-prb-edge-fallback'), colo: r.headers.get('cf-ray')?.split('-')[1], ms: Date.now() - t };
      const b = await r.json().catch(() => ({}));
      if (r.status === 200 && b.decision_id) {
        const e = await post('/e', { site, subject, consent: true, events: [
          { id: randomUUID(), name: 'exposure', slot, decision_id: b.decision_id },
          { id: randomUUID(), name: 'click', slot, decision_id: b.decision_id },
        ] });
        rec.e = e.status;
      }
      log.push(rec);
    } catch (err) { log.push({ i, t, error: String(err) }); }
    await new Promise((r) => setTimeout(r, 250));
  }
}
const runs = Array.from({ length: WORKERS }, (_, i) => visitor(i));
await new Promise((r) => setTimeout(r, WARM_S * 1000));
const er = await post(`/api/sites/${site}/erase`, { subject: victim, actor: 'Claude (EC4 live erase check)' }, true);
const erasedAt = Date.now();
const answer = await er.json().catch(() => ({}));
console.log(`erase ${er.status} at ${new Date(erasedAt).toISOString()} subject=${victim} edge=${JSON.stringify(answer.edge)}`);
await new Promise((r) => setTimeout(r, AFTER_S * 1000));
stop = true;
await Promise.all(runs);
const mine = log.filter((x) => x.i === 0 && x.t >= erasedAt);
const first410 = mine.find((x) => x.status === 410);
const after = first410 ? mine.filter((x) => x.t > first410.t && x.status !== 410) : [];
const served = mine.filter((x) => !first410 || x.t < first410.t);
console.log(`victim requests after erase: ${mine.length}; served before first 410: ${served.length} (statuses ${[...new Set(served.map((x) => x.status))]}, e ${[...new Set(served.map((x) => x.e))]})`);
console.log(first410 ? `first 410 after ${first410.t - erasedAt} ms (by=${first410.by} erased=${first410.erasedHdr} colo=${first410.colo})` : 'NO 410 in the window');
console.log(`non-410 after the first 410: ${after.length}`);
const src = {};
for (const x of mine.filter((x) => x.status === 410)) { const k = `${x.by}/${x.erasedHdr}/${x.why}`; src[k] = (src[k] || 0) + 1; }
console.log('410 sources (by/x-prb-erased/fallback):', JSON.stringify(src));
const others = log.filter((x) => x.i !== 0);
console.log(`others: ${others.length} requests, statuses ${JSON.stringify(others.reduce((a, x) => ((a[x.status ?? 'err'] = (a[x.status ?? 'err'] || 0) + 1), a), {}))}, colos ${[...new Set(log.map((x) => x.colo))]}`);
console.log(`ERASED_AT=${new Date(erasedAt).toISOString()} SUBJECT=${victim}`);
