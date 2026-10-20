// /decide latency as the Next SDK sees it: Node's fetch with keep-alive, N sequential calls,
// the first call dropped (it pays the TLS handshake).
// Usage: node deploy/checks/bench-node.mjs <base-url> [n] [tag]
// Env (EC2): CONSENT=1 sends ctx.consent=true (an identified visitor, which the edge can decide
// at once); POOL=k reuses k subjects instead of a fresh one per call (a cookieless visitor the
// edge has seen); GAP_MS waits between calls (past the 60 s sticky TTL needs a long gap or a
// fresh subject); SITE / SLOT pick the target (default shop / hero).
// Reports who decided (x-prb-decided-by) and why the edge did not (x-prb-edge-fallback).
const [base, n = '20', tag = String(Date.now())] = process.argv.slice(2);
if (!base) throw new Error('usage: node deploy/checks/bench-node.mjs <base-url> [n] [tag]');
const { CONSENT, POOL, GAP_MS, SITE = 'shop', SLOT = 'hero' } = process.env;
const times = [];
const by = {};
let bad = 0;
for (let i = 0; i <= Number(n); i++) {
  const subject = POOL ? `latbench-node-${tag}-${i % Number(POOL)}` : `latbench-node-${tag}-${i}`;
  const ctx = CONSENT ? { consent: true } : {};
  const t0 = performance.now();
  const r = await fetch(`${base}/decide`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ site: SITE, slot: SLOT, subject, ctx }),
  });
  await r.text();
  if (r.status !== 200) bad++;
  const who = `${r.headers.get('x-prb-decided-by') ?? 'core(no edge)'}${r.headers.get('x-prb-edge-fallback') ? `:${r.headers.get('x-prb-edge-fallback')}` : ''}`;
  if (i > 0) {
    times.push(performance.now() - t0);
    by[who] = (by[who] || 0) + 1;
  }
  if (GAP_MS) await new Promise((res) => setTimeout(res, Number(GAP_MS)));
}
times.sort((a, b) => a - b);
const at = (q) => times[Math.min(times.length - 1, Math.ceil(times.length * q) - 1)];
console.log(`node n=${times.length} p50=${at(0.5).toFixed(0)}ms p95=${at(0.95).toFixed(0)}ms min=${times[0].toFixed(0)}ms max=${times.at(-1).toFixed(0)}ms${bad ? ` NON-200=${bad}` : ''} by=${JSON.stringify(by)}`);
