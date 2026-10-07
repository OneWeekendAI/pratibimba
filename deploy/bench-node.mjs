// /decide latency as the Next SDK sees it: Node's fetch with keep-alive, N sequential calls,
// a fresh subject each, the first call dropped (it pays the TLS handshake).
// Usage: node deploy/bench-node.mjs <base-url> [n] [tag]
const [base, n = '20', tag = String(Date.now())] = process.argv.slice(2);
if (!base) throw new Error('usage: node deploy/bench-node.mjs <base-url> [n] [tag]');
const times = [];
let bad = 0;
for (let i = 0; i <= Number(n); i++) {
  const t0 = performance.now();
  const r = await fetch(`${base}/decide`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ site: 'shop', slot: 'hero', subject: `latbench-node-${tag}-${i}`, ctx: {} }),
  });
  await r.text();
  if (r.status !== 200) bad++;
  if (i > 0) times.push(performance.now() - t0);
}
times.sort((a, b) => a - b);
const at = (q) => times[Math.min(times.length - 1, Math.ceil(times.length * q) - 1)];
console.log(`node n=${times.length} p50=${at(0.5).toFixed(0)}ms p95=${at(0.95).toFixed(0)}ms min=${times[0].toFixed(0)}ms max=${times.at(-1).toFixed(0)}ms${bad ? ` NON-200=${bad}` : ''}`);
