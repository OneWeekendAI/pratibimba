// Shared by the Worker tests: a fake Cloudflare around the Worker (Core, KV, the beacon, the
// PoP cache, the events queue, a clock).
import { createWorker, createState } from '../src/index.js';
import { beacon } from '../src/beacon-core.js';
export const ORIGIN = 'https://core.example';
export const TOKEN = 'edge-secret';
export const NOON = Date.parse('2026-10-13T12:00:00.000Z');

export function snapshot({ version = 1, mode = 'full', paused = false, picker = [], ttl = 60 } = {}) {
  const prior = { mean: [0, 0, 0, 0], cov: [[1, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]] };
  return {
    format: 2, site: 'shop', version, generated_at: new Date(NOON).toISOString(),
    settings: { privacy_mode: mode, fields_allowed: null, holdout_fraction: 0, paused, night: { start: 22, end: 5 }, simulated_clock: false },
    slots: {
      hero: {
        id: 1, cold_start_exposures: 0, decision_ttl_secs: ttl, next_experiment_at: null,
        arms: [
          { id: 1, key: 'baseline', payload: null, baseline: true, traffic_share: null },
          { id: 2, key: 'gift', payload: '<h1>Gift</h1>', baseline: false, traffic_share: null },
        ],
        gates: [], picker_keys: picker, declared_features: {}, experiment: null,
        model: { version: 9, state: { arms: { 1: prior, 2: prior }, n: 0 } }, cold_start: null,
      },
    },
  };
}

function memoryStorage() {
  const m = new Map();
  return { get: async (k) => m.get(k), put: async (k, v) => { m.set(k, structuredClone(v)); } };
}

/** A fake Cloudflare around the worker: Core, KV, the beacon, the PoP cache, a clock. */
export function harness({ kv = null, beaconSnap = null, edgeDecide = 'on', coreSubject = null, ingest = 'stored', beaconHangs = false, queueWrites = 'off', queueFails = false, coreBatch = null, gone = [], goneHangs = false } = {}) {
  const clock = { t: NOON };
  const core = [];
  const queued = [];
  const stores = new Map();
  const b = beacon(memoryStorage());
  const calls = { version: 0, snapshot: 0 };
  const kvReads = [];
  const goneKeys = new Set(gone);
  globalThis.fetch = async (url, init) => {
    const u = new URL(String(url));
    const body = init.body ? JSON.parse(new TextDecoder().decode(init.body instanceof ArrayBuffer ? init.body : await new Response(init.body).arrayBuffer())) : null;
    const headers = new Headers(init.headers);
    core.push({ path: u.pathname, body, headers });
    if (u.pathname === '/e') return Response.json({ accepted: body.events.length }, { status: 202 });
    if (u.pathname === '/e/batch') return coreBatch ? coreBatch(body, headers) : Response.json({ accepted: [], duplicate: [], refused: [] });
    if (u.pathname === '/edge/decisions') {
      return Response.json({ results: body.decisions.map((d) => ({ id: d.id, status: ingest })) });
    }
    const id = `core-${core.length}`;
    const res = { decision_id: id, slot: body.slot, variant: 'baseline', payload: null, explanation: { policy: 'lints', seed: '1' } };
    const h = { 'content-type': 'application/json' };
    if (headers.get('x-prb-edge-token') === TOKEN) {
      const rec = { id, variant_id: 1, features: { bias: 1, returning: 0, mobile: 0, from_ad: 0 }, scores: { 1: 0.5, 2: 0.4 }, seed: '1', policy: 'lints',
        policy_version: 9, in_holdout: false, audiences: [], cold_start: null, experiment_id: null, allocation: 'bandit', created_at: new Date(clock.t).toISOString() };
      const info = { subject: coreSubject ?? body.subject, at: new Date(clock.t).toISOString(), records: [rec] };
      h['x-prb-edge'] = [...new TextEncoder().encode(JSON.stringify(info))].map((x) => x.toString(16).padStart(2, '0')).join('');
    }
    return new Response(JSON.stringify(res), { status: 200, headers: h });
  };
  const stub = {
    version: () => { calls.version++; return beaconHangs ? new Promise(() => {}) : b.version(); },
    snapshot: () => { calls.snapshot++; return b.snapshot(); },
    put: (s) => b.put(s),
  };
  const env = {
    ORIGIN, EDGE_DECIDE: edgeDecide, EDGE_TOKEN: TOKEN,
    // KV: the site's snapshot under site/<id>; EC4's tombstones under gone/<site>/<hash>.
    SNAPSHOTS: {
      get: async (k, opts) => {
        kvReads.push({ key: k, opts });
        if (k.startsWith('gone/')) return goneHangs ? new Promise(() => {}) : (goneKeys.has(k) ? { site: k.split('/')[1], erased_at: new Date(NOON).toISOString() } : null);
        return kv;
      },
    },
    BEACON: { idFromName: (n) => n, get: () => stub },
    QUEUE_WRITES: queueWrites,
    EVENTS_QUEUE: { send: async (m) => { if (queueFails) throw new Error('queue down'); queued.push(structuredClone(m)); } },
  };
  const cache = {
    entries: new Map(),
    async match(url) {
      const e = this.entries.get(url);
      return e && e.until > clock.t ? new Response(e.text) : undefined;
    },
    async put(url, res) {
      const age = Number(/max-age=(\d+)/.exec(res.headers.get('cache-control'))[1]);
      this.entries.set(url, { text: await res.text(), until: clock.t + age * 1000 });
    },
  };
  let n = 0;
  const worker = createWorker({ state: createState(), now: () => clock.t, seed: () => 42n, id: () => `edge-${++n}`, cache });
  const waits = [];
  const ctx = { waitUntil: (p) => waits.push(p) };
  const settle = async () => { while (waits.length) await waits.shift(); };
  const ready = beaconSnap ? b.put(beaconSnap) : Promise.resolve();
  const call = async (body, path = '/decide', headers = {}) => {
    await ready;
    const res = await worker.fetch(new Request(`https://api.example${path}`, { method: 'POST', body: JSON.stringify(body), headers: { 'content-type': 'application/json', ...headers } }), env, ctx);
    const text = await res.text();
    await settle();
    return { status: res.status, headers: res.headers, body: JSON.parse(text) };
  };
  const decides = () => core.filter((c) => c.path === '/decide' || c.path === '/decide/batch');
  const ingests = () => core.filter((c) => c.path === '/edge/decisions');
  return { clock, core, queued, kvReads, goneKeys, env, worker, call, decides, ingests, push: (s) => b.put(s), calls, cache, ctx, settle };
}
