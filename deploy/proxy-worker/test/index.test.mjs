// EC1: the proxy reads the site's edge snapshot on /decide and reports its age, and still
// forwards every request to Core unchanged. Run: node --test test/
import { test } from 'node:test';
import assert from 'node:assert/strict';
import worker, { SNAPSHOT_WAIT_MS } from '../src/index.js';

const ORIGIN = 'https://core.example';

/// A fake KV namespace and a fake Core; records what Core received.
function harness({ snapshots = {}, kvDelayMs = 0, kvThrows = false } = {}) {
  const seen = [];
  const reads = [];
  globalThis.fetch = async (url, init) => {
    const body = init.body ? await new Response(init.body).text() : null;
    seen.push({ url: String(url), method: init.method, body });
    return new Response(JSON.stringify({ ok: true }), { status: 200, headers: { 'content-type': 'application/json' } });
  };
  const env = {
    ORIGIN,
    SNAPSHOTS: {
      async get(key, type) {
        reads.push([key, type]);
        if (kvDelayMs) await new Promise((r) => setTimeout(r, kvDelayMs));
        if (kvThrows) throw new Error('kv down');
        return snapshots[key] ?? null;
      },
    },
  };
  return { env, seen, reads };
}

const decideBody = JSON.stringify({ site: 'shop', slot: 'hero', subject: 's-1' });
const post = (path, body = decideBody) => new Request(`https://pratibimbaapi.example${path}`, { method: 'POST', body, headers: { 'content-type': 'application/json' } });

test('decide: reports the snapshot version and age, and still forwards to Core', async () => {
  const generated = new Date(Date.now() - 5000).toISOString();
  const { env, seen, reads } = harness({ snapshots: { 'site/shop': { version: 7, generated_at: generated } } });
  const res = await worker.fetch(post('/decide'), env);
  assert.equal(res.status, 200);
  assert.deepEqual(reads, [['site/shop', 'json']]);
  assert.equal(res.headers.get('x-prb-snapshot'), 'ok');
  assert.equal(res.headers.get('x-prb-snapshot-version'), '7');
  const age = Number(res.headers.get('x-prb-snapshot-age-ms'));
  assert.ok(age >= 5000 && age < 7000, `age ${age}`);
  assert.equal(seen.length, 1, 'Core was not called');
  assert.equal(seen[0].url, `${ORIGIN}/decide`);
  assert.equal(seen[0].body, decideBody, 'the body reached Core changed');
});

test('decide/batch is covered too', async () => {
  const { env, seen } = harness({ snapshots: { 'site/shop': { version: 2, generated_at: new Date().toISOString() } } });
  const body = JSON.stringify({ site: 'shop', subject: 's', slots: ['hero'] });
  const res = await worker.fetch(post('/decide/batch', body), env);
  assert.equal(res.headers.get('x-prb-snapshot-version'), '2');
  assert.equal(seen[0].body, body);
});

test('a missing snapshot is said so, and Core still answers', async () => {
  const { env, seen } = harness();
  const res = await worker.fetch(post('/decide'), env);
  assert.equal(res.headers.get('x-prb-snapshot'), 'missing');
  assert.equal(res.headers.get('x-prb-snapshot-version'), null);
  assert.equal(seen.length, 1);
});

test('a failing KV read never fails the request', async () => {
  const { env, seen } = harness({ kvThrows: true });
  const res = await worker.fetch(post('/decide'), env);
  assert.equal(res.status, 200);
  assert.equal(res.headers.get('x-prb-snapshot'), 'error');
  assert.equal(seen.length, 1);
});

test('a slow KV read is cut off at SNAPSHOT_WAIT_MS', async () => {
  const { env } = harness({ kvDelayMs: SNAPSHOT_WAIT_MS * 6, snapshots: { 'site/shop': { version: 1, generated_at: new Date().toISOString() } } });
  const t0 = Date.now();
  const res = await worker.fetch(post('/decide'), env);
  const took = Date.now() - t0;
  assert.equal(res.headers.get('x-prb-snapshot'), 'timeout');
  assert.ok(took < SNAPSHOT_WAIT_MS * 4, `took ${took} ms`);
});

test('other paths do not read KV and carry no snapshot header', async () => {
  const { env, reads, seen } = harness({ snapshots: { 'site/shop': { version: 1, generated_at: new Date().toISOString() } } });
  const res = await worker.fetch(post('/e', JSON.stringify({ site: 'shop', subject: 's', events: [] })), env);
  assert.equal(reads.length, 0);
  assert.equal(res.headers.get('x-prb-snapshot'), null);
  assert.equal(seen.length, 1);
});

test('without the KV binding the proxy forwards as before', async () => {
  const { env, seen } = harness();
  delete env.SNAPSHOTS;
  const res = await worker.fetch(post('/decide'), env);
  assert.equal(res.status, 200);
  assert.equal(res.headers.get('x-prb-snapshot'), 'unbound');
  assert.equal(seen[0].body, decideBody);
});
