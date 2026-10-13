// EC5: the self-hoster's direct path, end to end through the real binary with no edge at all.
// SDK-shaped calls go straight to Core (no Worker, no KV, no queue), the worker drains them, and
// an erase is complete when it returns. Run by ci/parity.sh after `cargo build`; it refuses to
// run if any PRB_EDGE_* variable is set, so it cannot quietly test the edge path instead.
//
// Checks, each of which only the direct path can satisfy:
// - Core says at startup that its edge publisher is off;
// - /decide answers from Core; /e (not /e/batch) is accepted and the worker writes both events
//   and marks the decision exposed;
// - an erase answers `edge: null` (no edge window to wait for), and at once /decide and /e are
//   refused 410;
// - nothing edge-shaped was written: no snapshot row, no tombstone marked pushed.
import { execFileSync, spawn } from 'node:child_process';
import path from 'node:path';
import assert from 'node:assert/strict';

const leaked = Object.keys(process.env).filter((k) => k.startsWith('PRB_EDGE_'));
if (leaked.length) {
  console.error(`direct: ${leaked.join(', ')} set; this run must have no edge (ci/parity.sh unsets them)`);
  process.exit(1);
}
const ADMIN_DB = process.env.PRB_E2E_ADMIN_DB ?? process.env.DATABASE_URL ?? 'postgres://postgres@localhost:55440/postgres';
const DB = ADMIN_DB.replace(/\/[^/]*$/, '/prb_parity');
const BIN = path.resolve(import.meta.dirname, '../target/debug/pratibimba');
const PORT = Number(process.env.PRB_PARITY_PORT ?? 18190);
const CORE = `http://127.0.0.1:${PORT}`;
const TOKEN = 'parity-admin-token';
const psql = (sql, db = ADMIN_DB) => execFileSync('psql', [db, '-qAt', '-c', sql], { stdio: 'pipe' }).toString().trim();

psql('DROP DATABASE IF EXISTS prb_parity WITH (FORCE)');
psql('CREATE DATABASE prb_parity');
const env = { ...process.env, PRB_DB: DB, RUST_LOG: 'info', PRB_ADMIN_TOKEN: TOKEN, PRB_BIND: `127.0.0.1:${PORT}`, PRB_WORKER_TICK_MS: '100' };
execFileSync(BIN, ['migrate'], { env, stdio: 'pipe' });
execFileSync(BIN, ['apply', path.resolve(import.meta.dirname, 'site.json')], { env, stdio: 'pipe' });

let log = '';
const kids = ['serve', 'worker'].map((cmd) => {
  const k = spawn(BIN, [cmd], { env, stdio: ['ignore', 'pipe', 'pipe'] });
  k.stdout.on('data', (d) => { log += d; });
  k.stderr.on('data', (d) => { log += d; });
  return k;
});
const stop = () => { for (const k of kids) if (k.exitCode === null) k.kill('SIGTERM'); };
process.on('exit', stop);

async function until(what, fn, seconds = 30) {
  const end = Date.now() + seconds * 1000;
  while (Date.now() < end) {
    if (await fn()) return;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`direct: timed out waiting for ${what}\n--- core log ---\n${log.slice(-3000)}`);
}
const post = (p, body, admin = false) => fetch(CORE + p, {
  method: 'POST',
  headers: { 'content-type': 'application/json', ...(admin ? { authorization: `Bearer ${TOKEN}` } : {}) },
  body: JSON.stringify(body),
});

try {
  await until('Core /health', () => fetch(CORE + '/health').then((r) => r.ok, () => false));
  // tracing's fmt may colour the field: strip ANSI escapes before matching.
  await until('the startup line about the edge', () => /edge snapshot publisher/.test(log), 10);
  const plain = log.replace(/\x1b\[[0-9;]*m/g, '');
  assert.match(plain, /edge snapshot publisher.*on=false/, 'Core started with an edge publisher on');

  const subject = 'parity-visitor-1';
  const d = await post('/decide', { site: 'parity', slot: 'hero', subject, ctx: { returning: false } });
  assert.equal(d.status, 200, `decide ${d.status} ${await d.clone().text()}`);
  assert.equal(d.headers.get('x-prb-edge'), null, 'Core told an anonymous caller what it tells the edge');
  const decision = await d.json();
  assert.ok(decision.decision_id, 'no decision id');

  const id = () => crypto.randomUUID();
  const e = await post('/e', { site: 'parity', subject, events: [
    { id: id(), name: 'exposure', slot: 'hero', decision_id: decision.decision_id },
    { id: id(), name: 'click', slot: 'hero', decision_id: decision.decision_id },
  ] });
  assert.equal(e.status, 202, `/e ${e.status} ${await e.clone().text()}`);
  await until('the worker to write both events', () => Number(psql(`SELECT count(*) FROM events WHERE subject = '${subject}'`, DB)) === 2);
  await until('the decision to be exposed', () => psql(`SELECT exposed_at IS NOT NULL FROM decisions WHERE id = '${decision.decision_id}'`, DB) === 't');

  const er = await post('/api/sites/parity/erase', { subject, actor: 'parity' }, true);
  assert.equal(er.status, 200, `erase ${er.status} ${await er.clone().text()}`);
  const erased = await er.json();
  assert.equal(erased.edge, null, `a Core with no edge answered an edge window: ${JSON.stringify(erased.edge)}`);
  const again = await post('/decide', { site: 'parity', slot: 'hero', subject, ctx: {} });
  assert.equal(again.status, 410, 'an erased subject was decided for');
  const late = await post('/e', { site: 'parity', subject, events: [{ id: id(), name: 'click', slot: 'hero' }] });
  assert.equal(late.status, 410, 'an erased subject\'s event was accepted');
  assert.equal(psql(`SELECT count(*) FROM decisions WHERE subject = '${subject}'`, DB), '0', 'the erased subject still has decisions under its id');

  assert.equal(psql('SELECT count(*) FROM edge_snapshots', DB), '0', 'a snapshot was published with no edge');
  assert.equal(psql('SELECT count(*) FROM erasures WHERE edge_pushed_at IS NOT NULL', DB), '0', 'a tombstone was pushed with no edge');
  console.log('direct: ok (decide, /e, worker, erase, 410 after erase; edge off, nothing edge-shaped written)');
} catch (err) {
  console.error(err.message ?? err);
  process.exitCode = 1;
} finally {
  stop();
  await new Promise((r) => setTimeout(r, 300));
  try { psql('DROP DATABASE IF EXISTS prb_parity WITH (FORCE)'); } catch {}
}
