// The dashboard's test stack: a fresh database, Core serving the built dashboard at /app, a
// stand-in owner site for the install probes, a seeded history, then the real worker.
// Ready when http://localhost:3400/__ready answers 200. Stops everything on SIGTERM.
import { execFileSync, spawn } from 'node:child_process';
import { writeSync } from 'node:fs';
import { createServer } from 'node:http';
import path from 'node:path';

const here = import.meta.dirname;
const ADMIN_DB = process.env.PRB_E2E_ADMIN_DB ?? 'postgres://postgres@localhost:55440/postgres';
const NAME = 'prb_dash_e2e';
const DB = ADMIN_DB.replace(/\/[^/]*$/, `/${NAME}`);
const BIN = path.resolve(here, '../../target/debug/pratibimba');
export const CORE = 'http://127.0.0.1:18083';
export const TOKEN = 'dash-admin-token';
const VISITORS = Number(process.env.PRB_SEED_VISITORS ?? 8000);
const env = {
  ...process.env, PRB_DB: DB, RUST_LOG: 'warn', PRB_ADMIN_TOKEN: TOKEN, PRB_BIND: '127.0.0.1:18083',
  PRB_DASHBOARD_DIR: path.resolve(here, '../out'), PRB_WORKER_TICK_MS: '300', PRB_DB_MAX_CONNECTIONS: '20',
};
const psql = (sql, db = ADMIN_DB) => execFileSync('psql', [db, '-qAt', '-c', sql], { stdio: 'pipe' }).toString().trim();
// Core runs as children. They must die with this script however it ends: a crash here used
// to leave `serve` holding 18083, and the next run's tests then quietly drove that old binary.
const kids = [];
let ready = false;
process.on('exit', () => {
  for (const k of kids) if (k.exitCode === null) k.kill('SIGTERM');
});
process.on('SIGTERM', () => process.exit(0));
process.on('SIGINT', () => process.exit(0));
const fail = (why) => {
  writeSync(2, `stack: ${why}\n`); // console.error to a pipe is async on macOS: lost on exit
  process.exit(1);
};
process.on('unhandledRejection', (e) => fail(e?.stack ?? e));
function child(cmd) {
  const k = spawn(BIN, [cmd], { env, stdio: 'inherit' });
  k.on('exit', (code, sig) => {
    if (!ready || code) fail(`pratibimba ${cmd} exited (${code ?? sig})`);
  });
  kids.push(k);
}
// A loop that waits on the worker names what it waited for instead of running into the
// webServer timeout with nothing said.
async function until(what, ok, secs = 120) {
  const end = Date.now() + secs * 1000;
  while (!(await ok())) {
    if (Date.now() > end) fail(`gave up after ${secs}s waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 500));
  }
}
for (const port of [18083, 3400]) {
  const taken = await fetch(`http://127.0.0.1:${port}/`).then(() => true, () => false);
  if (taken) fail(`port ${port} is already in use, likely by an earlier run (pkill -f 'target/debug/pratibimba')`);
}

psql(`DROP DATABASE IF EXISTS ${NAME} WITH (FORCE)`);
psql(`CREATE DATABASE ${NAME}`);
execFileSync(BIN, ['apply', path.join(here, 'site.json')], { env, stdio: 'pipe' });
child('serve');

// The stand-in owner site: what the install probes look at. /__mode?reach=off unmounts its
// /_prb route, so a test can watch the wizard notice.
let reach = true;
createServer(async (req, res) => {
  const url = new URL(req.url, 'http://x');
  if (url.pathname === '/__ready') return res.writeHead(ready ? 200 : 503).end();
  if (url.pathname === '/__mode') {
    reach = url.searchParams.get('reach') !== 'off';
    return res.writeHead(204).end();
  }
  if (url.pathname.startsWith('/_prb/')) {
    if (!reach) return res.writeHead(404).end('not found');
    const up = await fetch(CORE + url.pathname.slice(5) + url.search);
    res.writeHead(up.status, { 'content-type': up.headers.get('content-type') ?? 'text/plain' });
    return res.end(Buffer.from(await up.arrayBuffer()));
  }
  res.writeHead(200, { 'content-type': 'text/html', 'x-prb-sdk': 'next/0.1.0' });
  res.end('<!doctype html><title>Kaapi Co.</title><h1>Kaapi Co.</h1>');
}).listen(3400, '127.0.0.1');

const H = { 'content-type': 'application/json', authorization: `Bearer ${TOKEN}` };
async function call(p, body, admin = false) {
  const r = await fetch(CORE + p, { method: 'POST', headers: admin ? H : { 'content-type': 'application/json' }, body: JSON.stringify(body) });
  if (!r.ok && r.status !== 202) throw new Error(`${p} ${r.status} ${await r.text()}`);
  return r.status === 202 || r.status === 204 ? null : r.json();
}
await until('Core /health', () => fetch(CORE + '/health').then((r) => r.ok, () => false), 30);

// A variant approved through the gate, so the install wizard's last step has something to see.
const draft = await call('/api/sites/shop/slots/banner/variants', { key: 'ten_off', html: '<p class="banner">10% off your first bag.</p>' }, true);
await call(`/api/variants/${draft.id}/approve`, { approver: 'Ritwik', traffic_share: 0.5, comment: 'Copy checked against the pricing page' }, true);

// Planted truth, seeded deterministically.
let s = 20261006;
const rand = () => ((s = (s * 1103515245 + 12345) % 2147483648) / 2147483648);
const truth = { hero: { baseline: 0.08, gift: 0.2, free_ship: 0.005 }, banner: { plain: 0.04, ten_off: 0.05 } };
const uuid = () => crypto.randomUUID();
async function visit(i) {
  const subject = `v-${i.toString(36)}-${Math.floor(rand() * 1e9).toString(36)}`;
  const ctx = { returning: rand() < 0.35, mobile: rand() < 0.55, from_ad: rand() < 0.2 };
  const { decisions } = await call('/decide/batch', { site: 'shop', subject, slots: ['hero', 'banner'], ctx });
  const events = [];
  for (const d of decisions) {
    if (rand() < 0.04) continue; // a few never render: decided, not exposed
    events.push({ id: uuid(), name: 'exposure', slot: d.slot, decision_id: d.decision_id });
    if (rand() < truth[d.slot][d.variant]) events.push({ id: uuid(), name: 'click', slot: d.slot, decision_id: d.decision_id });
  }
  if (events.length) await call('/e', { site: 'shop', subject, events });
}
async function visits(from, to) {
  let next = from;
  await Promise.all(Array.from({ length: 16 }, async () => {
    while (next < to) await visit(next++);
  }));
}
// Move the decisions made since `since` back in time, oldest to `fromDays` ago and the newest
// `keep` of them still at now, so their attribution windows have closed.
function backdate(since, fromDays, toDays, keep = 0) {
  psql(
    `CREATE TEMP TABLE shift AS
       SELECT id, make_interval(secs => ${toDays * 86400} + greatest(0, (1 - (row_number() OVER (ORDER BY created_at))::float / count(*) OVER () - ${keep})) * ${(fromDays - toDays) * 86400}) AS by
         FROM decisions WHERE created_at >= '${since}';
     UPDATE decisions d SET created_at = d.created_at - s.by FROM shift s WHERE s.id = d.id;
     UPDATE events e SET received_at = e.received_at - s.by + interval '2 seconds' FROM shift s WHERE s.id = e.decision_id;`,
    DB,
  );
}
const settled = () =>
  until('the worker to settle the backdated decisions', () =>
    Number(psql('SELECT count(*) FROM pending_events', DB)) === 0 &&
    Number(psql("SELECT count(*) FROM decisions WHERE settled_at IS NULL AND created_at < now() - interval '1 day'", DB)) === 0);
const now = () => psql('SELECT now()::text', DB);

// Two halves, so the history shows learning: the first half is decided by the untrained model
// (days 6 to 3 ago) and the worker learns from it; the second half (days 3 to 0) is decided by
// what it learned. The newest 3% sit two minutes ago, windows still open. Not at "now": the
// seed would then fill the "last minute" tile and drain out of it while the live-tile test
// counts its own seven decisions in, so the count could fall short of before + 7.
// Auto-rollback waits for the second half. Pausing free_ship restores the slot's model to its
// approval checkpoint (version 0, the prior); if that happened during the first settle, the
// second half would be decided by an untrained model and the champion would be a coin flip.
psql("UPDATE sites SET rollback_min_exposures = 1000000 WHERE id = 'shop'", DB);
const t1 = now();
await visits(0, VISITORS / 2);
backdate(t1, 6, 3);
child('worker');
await settled();
psql("UPDATE sites SET rollback_min_exposures = 200 WHERE id = 'shop'", DB);
const t2 = now();
await visits(VISITORS / 2, VISITORS);
backdate(t2, 3, 120 / 86400, 0.03);
await settled();
await until('auto-rollback to pause a variant', () => Number(psql("SELECT count(*) FROM variants WHERE status = 'paused'", DB)) > 0);

// Two drafts for the approvals view: an edit of the champion, and one that fails a check.
await call('/api/sites/shop/slots/hero/variants', { key: 'gift_ribbon', html: '<h1>Gift sets, wrapped in kraft and ribbon</h1><p>Three roasts, a tasting card and a note in your words.</p><a href="#gifts">Choose a gift</a>', note: 'Warmer wording for the holiday weeks', author_name: 'Asha' }, true);
await call('/api/sites/shop/slots/hero/variants', { key: 'popup_offer', html: '<h1 onmouseover="openPopup()">Wait! 15% off</h1>' }, true);

// A coding agent over MCP: a read, a refused call (no justification), and a multi-line draft
// the approvals card shows with the agent badge and the code diff.
const { token: mcpToken } = await call('/api/sites/shop/mcp-tokens', { name: 'ritwik-laptop' }, true);
let rpc = 0;
async function tool(name, args) {
  const r = await fetch(CORE + '/mcp', {
    method: 'POST',
    headers: { 'content-type': 'application/json', authorization: `Bearer ${mcpToken}` },
    body: JSON.stringify({ jsonrpc: '2.0', id: ++rpc, method: 'tools/call', params: { name, arguments: args } }),
  });
  if (!r.ok) throw new Error(`mcp ${name} ${r.status} ${await r.text()}`);
  return r.json();
}
const who = { agent: 'Claude Code', on_behalf_of: 'Ritwik' };
await tool('get_results', { ...who, justification: 'Read the hero results before proposing a variant' });
await tool('list_variants', { agent: 'Claude Code', slot: 'hero' });
await tool('propose_variant', {
  ...who,
  justification: 'gift leads on hero; a box-builder CTA should lift clicks from returning buyers',
  slot: 'hero',
  key: 'gift_box',
  html: '<section class="hero">\n  <h1>Gift sets, wrapped and ready</h1>\n  <p>Three roasts, a tasting card and a note in your words.</p>\n  <a href="#gifts">Build a gift box</a>\n</section>',
  note: 'Same headline and copy as gift; only the call to action changes.',
});
ready = true;
console.log('stack ready');
