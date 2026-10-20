// Gates the `comeback` slot's offer on visitor history: "last order 30+ days ago". Run once after
// the site file is applied; running it again changes nothing.
//
//   PRB_ADMIN_TOKEN=... node scripts/audiences.mjs [--core http://localhost:8080]
//
// History needs an identified visitor (full mode, or consent_aware with consent). Anyone else,
// and every cookieless visitor, matches nobody and keeps the baseline line.
const arg = (k, d) => { const i = process.argv.indexOf(k); return i > 0 ? process.argv[i + 1] : d; };
const CORE = arg('--core', process.env.PRB_CORE_URL ?? 'http://localhost:8080');
const TOKEN = process.env.PRB_ADMIN_TOKEN;
if (!TOKEN) throw new Error('set PRB_ADMIN_TOKEN');
const SITE = 'shop';
const PREDICATE = { days_since_last_purchase: { gte: 30 } };
const ACTOR = 'examples/shop';

async function call(method, path, body) {
  const r = await fetch(`${CORE}/api${path}`, {
    method,
    headers: { authorization: `Bearer ${TOKEN}`, ...(body ? { 'content-type': 'application/json' } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  const j = await r.json().catch(() => ({}));
  return { status: r.status, j };
}

let seg = await call('POST', `/sites/${SITE}/segments`, { predicate: PREDICATE, name: 'Last order 30+ days ago', actor: ACTOR });
let id = seg.j.id;
if (seg.status === 409) {
  const all = await call('GET', `/sites/${SITE}/segments`);
  const rows = Array.isArray(all.j) ? all.j : all.j.segments ?? [];
  id = rows.find((s) => JSON.stringify(s.predicate) === JSON.stringify(PREDICATE))?.id;
} else if (seg.status !== 201) throw new Error(`segment: ${seg.status} ${JSON.stringify(seg.j)}`);
if (!id) throw new Error('no segment id');
const p = await call('POST', `/segments/${id}/promote`, { actor: ACTOR });
if (p.status !== 200) throw new Error(`promote: ${p.status} ${JSON.stringify(p.j)}`);
const slot = await call('GET', `/sites/${SITE}/slots/comeback`);
const variant = slot.j.variants?.find((v) => v.key === 'come_back');
if (!variant) throw new Error('no come_back variant: apply site.json first');
const g = await call('POST', `/variants/${variant.id}/audience`, { audience: id, actor: ACTOR });
if (g.status !== 200) throw new Error(`gate: ${g.status} ${JSON.stringify(g.j)}`);
console.log(`come_back on comeback is gated on audience ${id} (last order 30+ days ago)`);
