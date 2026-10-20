// EC2's 5 s rule, measured live: how long after the kill switch commits does the edge stop
// serving variants? Polls /decide (identified visitors, so the edge decides) every POLL_MS while
// it pauses and resumes a site through the admin API, and reports, per toggle, the lag from the
// admin call's return to the first response that shows the new state, plus any response that
// still showed the old state more than 5 s later.
// Usage: PRB_ADMIN_TOKEN=... node deploy/checks/edge-killswitch.mjs <base-url> [site] [slot] [cycles]
// A response "shows paused" when only the baseline was scored (a paused site scores nothing else).
const [base, site = 'demo', slot = 'hero', cycles = '3'] = process.argv.slice(2);
const token = process.env.PRB_ADMIN_TOKEN;
if (!base || !token) throw new Error('usage: PRB_ADMIN_TOKEN=... node deploy/checks/edge-killswitch.mjs <base-url> [site] [slot] [cycles]');
const POLL_MS = Number(process.env.POLL_MS || 200);
const tag = Date.now();
let n = 0;
const log = [];
let stop = false;

async function poll() {
  while (!stop) {
    const t = Date.now();
    try {
      const r = await fetch(`${base}/decide`, {
        method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ site, slot, subject: `ec2-ks-${tag}-${n++}`, ctx: { consent: true } }),
      });
      const b = await r.json();
      const keys = b.explanation ? Object.keys(b.explanation.scores) : [];
      log.push({ t, done: Date.now(), by: r.headers.get('x-prb-decided-by'), why: r.headers.get('x-prb-edge-fallback'), version: r.headers.get('x-prb-snapshot-version'), paused: keys.length === 1, colo: r.headers.get('cf-ray')?.split('-')[1] });
    } catch (e) {
      log.push({ t, error: String(e) });
    }
    await new Promise((res) => setTimeout(res, Math.max(0, POLL_MS - (Date.now() - t))));
  }
}

async function pause(paused) {
  const r = await fetch(`${base}/api/sites/${site}/pause`, {
    method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${token}` },
    body: JSON.stringify({ actor: 'Claude (EC2 kill-switch check)', paused }),
  });
  if (!r.ok) throw new Error(`pause ${paused}: ${r.status} ${await r.text()}`);
  return Date.now();
}

const poller = poll();
await new Promise((res) => setTimeout(res, 6000));
const results = [];
for (let c = 0; c < Number(cycles); c++) {
  for (const want of [true, false]) {
    const committed = await pause(want);
    await new Promise((res) => setTimeout(res, 9000));
    // Requests sent after the admin call returned.
    const after = log.filter((l) => l.t >= committed && !l.error);
    const first = after.find((l) => l.paused === want);
    const lastWrong = [...after].reverse().find((l) => l.paused !== want);
    const lateWrong = after.filter((l) => l.paused !== want && l.t - committed > 5000);
    const edge = after.filter((l) => l.by === 'edge').length;
    results.push({ paused: want, lag_first_ms: first ? first.t - committed : null, lag_last_wrong_ms: lastWrong ? lastWrong.t - committed : 0, wrong_after_5s: lateWrong.length, requests: after.length, edge_decided: edge, fallbacks: [...new Set(after.map((l) => l.why).filter(Boolean))] });
    console.log(JSON.stringify(results.at(-1)));
  }
}
stop = true;
await poller;
const before = log.filter((l) => !l.error);
console.log(JSON.stringify({ total: before.length, edge: before.filter((l) => l.by === 'edge').length, colos: [...new Set(before.map((l) => l.colo))], worst_last_wrong_ms: Math.max(...results.map((r) => r.lag_last_wrong_ms)), wrong_after_5s: results.reduce((a, r) => a + r.wrong_after_5s, 0) }));
