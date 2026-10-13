// Proxies every path on the public API hostname to Core on Cloud Run (asia-northeast3, which
// has no Cloud Run domain mappings). Method, query, headers and body pass through; request and
// response bodies stream, so SSE (/api/sites/*/stream) and /mcp work. Nothing is cached except
// /sdk.js, which keeps whatever Cache-Control Core sends.
//
// EC1 (edge-cache.md): with edge decide off, /decide and /decide/batch also read the site's
// snapshot from KV (binding SNAPSHOTS, key `site/<id>`, written by Core) and report its age as
// response headers, capped at SNAPSHOT_WAIT_MS, and still go to Core.
//
// EC2: with EDGE_DECIDE = "on" (and the EDGE_TOKEN secret and the BEACON Durable Object bound),
// the Worker answers /decide and /decide/batch itself from the snapshot, with Core's own
// decision logic (src/kernel.js), and sends the decision record to Core after the response
// (ctx.waitUntil, POST /edge/decisions; EC3 puts a queue there). It asks Core instead, and says
// why in `x-prb-edge-fallback`, whenever it cannot be sure to answer as Core would:
//
// - no snapshot, or none of the version the beacon names, or the beacon did not answer;
// - a cookieless visitor this PoP has not seen today: only Core can make the day's pseudonym
//   (the salt never leaves Core), so the first decide of a visitor's day goes to Core and the
//   Worker remembers the pseudonym Core reports, hashed, in the PoP's cache, until midnight UTC;
// - the last minute before midnight UTC (the pseudonym changes at midnight);
// - a site with a picker (Core reads each visitor's latest picks), a simulated clock, an
//   experiment that started or ended since the snapshot, or anything Core would refuse;
// - a visitor Core reported erased.
//
// **Freshness (the 5 s rule).** KV can take a minute to show a new snapshot at a PoP (EC1
// measured 15 s and 51 s), so KV alone cannot make a kill switch, a retire or an experiment stop
// reach the edge in seconds. Core also puts every snapshot to a Durable Object per site (the
// beacon, src/beacon.js), which is strongly consistent. This isolate decides only from a
// snapshot whose version equals the beacon's, and it trusts a beacon reading for BEACON_FRESH_MS
// at most: older, the request waits for a new reading (up to BEACON_WAIT_MS, else Core decides).
// A reading older than BEACON_REFRESH_MS is refreshed in the background. So a change reaches the
// edge within BEACON_FRESH_MS of Core's push, whatever KV is doing.
//
// EC3, the write buffer. With QUEUE_WRITES = "on" (and the EVENTS_QUEUE producer bound), a
// browser's POST /e and the Worker's own decision records go onto a Cloudflare Queue instead of
// to Core, and /e answers 202 {queued} at once. The same script consumes the queue: it sends up
// to 100 messages at a time to Core's POST /e/batch and acks the batch only on a 2xx; anything
// else retries the whole batch with backoff (RETRY_DELAYS_S), and after max_retries (wrangler
// config) a message goes to the dead-letter queue. Core is idempotent on every id, so a retry or
// a replay is harmless. /e requests the Worker cannot vouch for (a pick, a rescore, an unknown
// field, a malformed body) still go straight to Core, which answers them as before. Off switch:
// deploy/proxy-worker/queue.sh off (the direct path, unchanged); the consumer keeps draining.
//
// Rollback: EDGE_DECIDE=off (deploy/proxy-worker/edge-decide.sh off) puts every decide back on
// Core; deleting the route (switch.sh off) takes the Worker out entirely.

import { decide as kernelDecide, identified as isIdentified, FORMAT } from './kernel.js';

export const SNAPSHOT_WAIT_MS = 150;
export const BEACON_FRESH_MS = 3000;
export const BEACON_REFRESH_MS = 1000;
export const BEACON_WAIT_MS = 400;
export const KV_WAIT_MS = 150;
/** No edge decide this close to midnight UTC: the cookieless pseudonym changes there. */
export const MIDNIGHT_GUARD_MS = 60_000;
const DECIDE_PATHS = new Set(['/decide', '/decide/batch']);
const PUSH_PATH = '/_prb/edge/snapshot';
const CACHE_ORIGIN = 'https://prb-edge.internal';
const DAY_MS = 86_400_000;
/** Backoff per delivery attempt when Core does not take a batch; the last one repeats. The whole
 * schedule (8 retries, about 23 min) stays inside Core's ORPHAN_HOLD_SECS (30 min), the time an
 * event waits in Core's outbox for a decision record still in the queue. */
export const RETRY_DELAYS_S = [15, 30, 60, 120, 240, 300];
const EVENT_KEYS = new Set(['id', 'name', 'slot', 'decision_id']);
const REQUEST_KEYS = new Set(['site', 'subject', 'events', 'tz_offset', 'consent']);
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const NAME_RE = /^[a-z0-9_:]{1,64}$/;
const SUBJECT_RE = /^[\x21-\x7e]{1,256}$/;
/** Largest /e body the Worker queues (a queue message holds 128 KB). */
const MAX_QUEUED_BYTES = 64_000;

/** Whether an /e body is one the queue may carry: exactly what Core's /e would accept as plain
 * events (no pick or rescore). Anything else goes to Core, which answers it itself. */
export function queueable(body) {
  if (!body || typeof body !== 'object' || Array.isArray(body)) return false;
  if (!Object.keys(body).every((k) => REQUEST_KEYS.has(k))) return false;
  if (typeof body.site !== 'string' || !body.site || typeof body.subject !== 'string' || !SUBJECT_RE.test(body.subject)) return false;
  if (body.tz_offset != null && !(Number.isInteger(body.tz_offset) && body.tz_offset >= -720 && body.tz_offset <= 840)) return false;
  if (body.consent != null && typeof body.consent !== 'boolean') return false;
  const ev = body.events;
  if (!Array.isArray(ev) || ev.length < 1 || ev.length > 100) return false;
  return ev.every((e) => e && typeof e === 'object' && !Array.isArray(e)
    && Object.keys(e).every((k) => EVENT_KEYS.has(k))
    && typeof e.id === 'string' && UUID_RE.test(e.id)
    && typeof e.name === 'string' && NAME_RE.test(e.name) && e.name !== 'pick' && e.name !== 'rescore'
    && (e.slot == null || typeof e.slot === 'string')
    && (e.decision_id == null || (typeof e.decision_id === 'string' && UUID_RE.test(e.decision_id))));
}

/// The site's snapshot as this PoP's KV sees it: {version, age_ms} or {missing|error|timeout} (EC1).
export async function snapshotAge(env, site, now = Date.now()) {
  if (!env.SNAPSHOTS) return { state: 'unbound' };
  if (typeof site !== 'string' || !site) return { state: 'no-site' };
  const read = env.SNAPSHOTS.get(`site/${site}`, 'json').then(
    (s) => (s ? { state: 'ok', version: s.version, age_ms: Math.max(0, now - Date.parse(s.generated_at)) } : { state: 'missing' }),
    () => ({ state: 'error' }),
  );
  let timer;
  const timeout = new Promise((resolve) => { timer = setTimeout(() => resolve({ state: 'timeout' }), SNAPSHOT_WAIT_MS); });
  try {
    return await Promise.race([read, timeout]);
  } finally {
    clearTimeout(timer);
  }
}

async function siteOf(request) {
  try {
    const body = await request.json();
    return body && body.site;
  } catch {
    return undefined;
  }
}

/** Resolves to `work`'s value, or to `fallback` after `ms`. */
async function within(work, ms, fallback) {
  let timer;
  const late = new Promise((resolve) => { timer = setTimeout(() => resolve(fallback), ms); });
  try {
    return await Promise.race([Promise.resolve(work).catch(() => fallback), late]);
  } finally {
    clearTimeout(timer);
  }
}

function sameSecret(a, b) {
  if (typeof a !== 'string' || typeof b !== 'string') return false;
  const x = new TextEncoder().encode(a);
  const y = new TextEncoder().encode(b);
  if (x.length !== y.length) return false;
  let d = 0;
  for (let i = 0; i < x.length; i++) d |= x[i] ^ y[i];
  return d === 0;
}

async function sha256hex(s) {
  const h = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(s));
  return [...new Uint8Array(h)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

function randomSeed() {
  return crypto.getRandomValues(new BigUint64Array(1))[0];
}

const json = (body, status = 200, headers = {}) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', ...headers } });

/** What the Worker keeps across requests in one isolate. Module state, but never request state:
 * readings of the beacon, the snapshot of that version, and this isolate's cold-start turns. */
export function createState() {
  return { beacons: new Map(), snapshots: new Map(), pending: new Map(), cold: new Map() };
}

/** The edge decide runtime. `deps` is injectable for tests: cache, clock, seed and id sources. */
export function createWorker(deps = {}) {
  const state = deps.state || createState();
  const now = deps.now || (() => Date.now());
  const seed = deps.seed || randomSeed;
  const newId = deps.id || (() => crypto.randomUUID());
  const cacheOf = () => deps.cache || caches.default;

  const stubOf = (env, site) => env.BEACON.get(env.BEACON.idFromName(site), { locationHint: env.BEACON_LOCATION || 'apac' });

  /** A fresh beacon reading for `site`, shared by concurrent requests. */
  function readBeacon(env, site) {
    let p = state.pending.get(site);
    if (!p) {
      const started = now();
      p = within(stubOf(env, site).version(), BEACON_WAIT_MS, null).then((v) => {
        state.pending.delete(site);
        if (v === null || typeof v !== 'object') return null;
        const reading = { version: v.version ?? null, at: started };
        state.beacons.set(site, reading);
        return reading;
      });
      state.pending.set(site, p);
    }
    return p;
  }

  /** The snapshot to decide from, or `{reason}` why there is none this instant. */
  async function current(env, ctx, site) {
    let b = state.beacons.get(site);
    const t = now();
    if (!b || t - b.at > BEACON_FRESH_MS) {
      b = await readBeacon(env, site);
      if (!b) return { reason: 'beacon-unreachable' };
    } else if (t - b.at > BEACON_REFRESH_MS) {
      ctx.waitUntil(readBeacon(env, site));
    }
    if (b.version === null) return { reason: 'snapshot-missing' };
    const have = state.snapshots.get(site);
    if (have && have.version === b.version) return { snap: have, beacon: b };
    const kv = env.SNAPSHOTS ? await within(env.SNAPSHOTS.get(`site/${site}`, 'json'), KV_WAIT_MS, null) : null;
    if (kv && kv.version === b.version) {
      state.snapshots.set(site, kv);
      return { snap: kv, beacon: b };
    }
    const fromBeacon = await within(stubOf(env, site).snapshot(), BEACON_WAIT_MS, null);
    if (fromBeacon && fromBeacon.version >= b.version) {
      state.snapshots.set(site, fromBeacon);
      if (fromBeacon.version > b.version) state.beacons.set(site, { version: fromBeacon.version, at: t });
      return { snap: fromBeacon, beacon: b };
    }
    return { reason: 'snapshot-stale' };
  }

  const keyUrl = (kind, hash) => `${CACHE_ORIGIN}/${kind}/${hash}`;
  const dayOf = (ms) => new Date(ms).toISOString().slice(0, 10);
  const pseudonymKey = async (site, raw, ms) => keyUrl('p', await sha256hex(`${site}\0${raw}\0${dayOf(ms)}`));
  const stickyKey = async (site, slot, stored) => keyUrl('d', await sha256hex(`${site}\0${slot}\0${stored}`));
  const goneKey = async (site, raw) => keyUrl('g', await sha256hex(`${site}\0${raw}`));

  async function cached(url) {
    try {
      const r = await cacheOf().match(url);
      return r ? await r.text() : null;
    } catch {
      return null;
    }
  }
  async function keep(url, text, seconds) {
    if (seconds <= 0) return;
    try {
      await cacheOf().put(url, new Response(text, { headers: { 'cache-control': `max-age=${Math.floor(seconds)}` } }));
    } catch {}
  }

  /** What Core's `x-prb-edge` header taught us: the stored subject and its live decisions. */
  async function learn(site, rawSubject, edgeHeader, bodyText, snap) {
    let info;
    try {
      const bytes = new Uint8Array(edgeHeader.match(/../g).map((h) => parseInt(h, 16)));
      info = JSON.parse(new TextDecoder().decode(bytes));
    } catch {
      return;
    }
    const t = now();
    if (info.subject && info.subject !== rawSubject) {
      const toMidnight = (Math.floor(t / DAY_MS) + 1) * DAY_MS - t;
      await keep(await pseudonymKey(site, rawSubject, t), info.subject, toMidnight / 1000);
    }
    let body;
    try {
      body = JSON.parse(bodyText);
    } catch {
      return;
    }
    const decisions = Array.isArray(body.decisions) ? body.decisions : [body];
    for (const rec of info.records || []) {
      const d = decisions.find((x) => x && x.decision_id === rec.id);
      if (!d) continue;
      const ttl = snap && snap.slots[d.slot] ? snap.slots[d.slot].decision_ttl_secs : 60;
      const left = ttl - (t - Date.parse(rec.created_at)) / 1000;
      await keep(await stickyKey(site, d.slot, info.subject), JSON.stringify({ ...rec, created_at_ms: Date.parse(rec.created_at) }), left);
    }
  }

  /** Forward to Core (with the edge token, so Core says what the Worker should learn). */
  async function toCore(request, env, ctx, { reason, body, snap, rawBody }) {
    const res = await proxy(request, env, { edgeToken: env.EDGE_TOKEN, body: rawBody });
    const out = new Headers(res.headers);
    const edge = out.get('x-prb-edge');
    out.delete('x-prb-edge');
    out.set('x-prb-decided-by', 'core');
    out.set('x-prb-edge-fallback', reason);
    if (snap) out.set('x-prb-snapshot-version', String(snap.version));
    if (res.status === 410 && body && typeof body.subject === 'string') {
      ctx.waitUntil(goneKey(body.site, body.subject).then((k) => keep(k, '1', 86_400)));
    }
    if (!edge || res.status !== 200 || !body) return new Response(res.body, { status: res.status, statusText: res.statusText, headers: out });
    const text = await res.text();
    ctx.waitUntil(learn(body.site, body.subject, edge, text, snap));
    return new Response(text, { status: res.status, statusText: res.statusText, headers: out });
  }

  /** This isolate's own cold-start turns since the snapshot, so a burst rotates the arms. */
  function withColdTurns(snap, slotKey) {
    const slot = snap.slots[slotKey];
    if (!slot || !slot.cold_start) return snap;
    const mine = state.cold.get(`${snap.site}\0${slotKey}\0${snap.version}`);
    if (!mine) return snap;
    const served = { ...slot.cold_start.served };
    for (const [id, n] of mine) served[id] = (served[id] || 0) + n;
    return { ...snap, slots: { ...snap.slots, [slotKey]: { ...slot, cold_start: { ...slot.cold_start, served } } } };
  }
  function countColdTurn(snap, slotKey, variantId) {
    const k = `${snap.site}\0${slotKey}\0${snap.version}`;
    let m = state.cold.get(k);
    if (!m) {
      for (const old of state.cold.keys()) if (old.startsWith(`${snap.site}\0${slotKey}\0`)) state.cold.delete(old);
      m = new Map();
      state.cold.set(k, m);
    }
    m.set(String(variantId), (m.get(String(variantId)) || 0) + 1);
  }

  /** Send the new decisions to Core, remember them as sticky, and learn of erasures. */
  async function after(env, snap, body, made) {
    for (const { slot, record } of made) {
      const ttl = snap.slots[slot].decision_ttl_secs;
      await keep(await stickyKey(body.site, slot, record.subject), JSON.stringify(record), ttl);
    }
    const record = {
      site: body.site,
      subject: body.subject,
      consent: body.ctx && typeof body.ctx.consent === 'boolean' ? body.ctx.consent : null,
      snapshot_version: snap.version,
      decisions: made.map((m) => m.record),
    };
    if (queueOn(env)) {
      try {
        await env.EVENTS_QUEUE.send({ kind: 'decisions', request: record });
        return;
      } catch (e) {
        console.log(JSON.stringify({ msg: 'edge record enqueue failed, sending direct', site: body.site, error: String(e) }));
      }
    }
    let res;
    try {
      res = await fetch(new URL('/edge/decisions', env.ORIGIN), {
        method: 'POST',
        headers: { 'content-type': 'application/json', authorization: `Bearer ${env.EDGE_TOKEN}` },
        body: JSON.stringify(record),
      });
    } catch (e) {
      console.log(JSON.stringify({ msg: 'edge record send failed', site: body.site, error: String(e) }));
      return;
    }
    let out = null;
    try {
      out = await res.json();
    } catch {}
    const results = (out && out.results) || [];
    if (!res.ok) console.log(JSON.stringify({ msg: 'edge record refused', site: body.site, status: res.status }));
    for (const r of results) {
      if (r.status === 'gone') await keep(await goneKey(body.site, body.subject), '1', 86_400);
      if (r.status === 'rejected') console.log(JSON.stringify({ msg: 'edge record rejected', site: body.site, id: r.id, reason: r.reason }));
    }
  }

  async function edgeDecide(request, env, ctx, path) {
    const started = now();
    const rawBody = await request.clone().arrayBuffer();
    let body;
    try {
      body = JSON.parse(new TextDecoder().decode(rawBody));
    } catch {
      return toCore(request, env, ctx, { reason: 'bad-json', rawBody });
    }
    const fallback = (reason, snap) => toCore(request, env, ctx, { reason, body, snap, rawBody });
    if (!body || typeof body.site !== 'string' || typeof body.subject !== 'string') return fallback('bad-request');
    const batch = path === '/decide/batch';
    const slots = batch ? body.slots : [body.slot];
    if (!Array.isArray(slots) || slots.length === 0 || slots.length > 20 || !slots.every((s) => typeof s === 'string')) return fallback('bad-request');
    const got = await current(env, ctx, body.site);
    if (!got.snap) return fallback(got.reason);
    const snap = got.snap;
    if (snap.format !== FORMAT) return fallback('snapshot-format', snap);
    for (const s of slots) if (snap.slots[s] && snap.slots[s].picker_keys.length > 0) return fallback('picker', snap);
    if (await cached(await goneKey(body.site, body.subject))) return fallback('erased', snap);
    const t = now();
    const consent = body.ctx && typeof body.ctx === 'object' ? body.ctx.consent : undefined;
    let stored = null;
    if (!isIdentified(snap.settings.privacy_mode, consent === true ? true : consent === false ? false : null)) {
      if ((Math.floor(t / DAY_MS) + 1) * DAY_MS - t < MIDNIGHT_GUARD_MS) return fallback('midnight', snap);
      stored = await cached(await pseudonymKey(body.site, body.subject, t));
      if (!stored) return fallback('unseen-cookieless', snap);
    }
    const subject = stored || body.subject;
    const out = [];
    const made = [];
    for (const slot of slots) {
      const one = { site: body.site, slot, subject: body.subject, ...(body.ctx !== undefined ? { ctx: body.ctx } : {}) };
      if (batch) {
        for (const k of Object.keys(body)) if (!['site', 'subject', 'slots', 'ctx'].includes(k)) return fallback('bad-request', snap);
      } else {
        for (const k of Object.keys(body)) if (!['site', 'slot', 'subject', 'ctx'].includes(k)) return fallback('bad-request', snap);
      }
      const stickyText = await cached(await stickyKey(body.site, slot, subject));
      let sticky = null;
      try {
        sticky = stickyText ? JSON.parse(stickyText) : null;
      } catch {}
      const r = kernelDecide(withColdTurns(snap, slot), one, { stored: subject, seed: seed(), at: t, sticky, picks: [], id: newId() });
      if (!r.ok) return fallback(r.reason, snap);
      out.push(r.response);
      if (r.record) made.push({ slot, record: r.record });
    }
    for (const m of made) if (m.record.cold_start) countColdTurn(snap, m.slot, m.record.variant_id);
    if (made.length > 0) ctx.waitUntil(after(env, snap, body, made));
    console.log(JSON.stringify({ msg: 'edge decide', site: body.site, slots: slots.length, version: snap.version, sticky: out.length - made.length, ms: now() - started, colo: request.cf?.colo }));
    return json(batch ? { decisions: out } : out[0], 200, {
      'x-prb-decided-by': 'edge',
      'x-prb-snapshot-version': String(snap.version),
      'cache-control': 'no-store',
    });
  }

  /** Core's snapshot push: kept in the site's beacon if it is newer than what it holds. */
  async function push(request, env) {
    const auth = request.headers.get('authorization') || '';
    if (!env.EDGE_TOKEN || !env.BEACON || !sameSecret(auth.replace(/^Bearer /, ''), env.EDGE_TOKEN)) return json({ error: 'edge token required' }, 401);
    let snap;
    try {
      snap = await request.json();
    } catch {
      return json({ error: 'not JSON' }, 400);
    }
    if (!snap || typeof snap.site !== 'string' || !Number.isInteger(snap.version)) return json({ error: 'a snapshot has site and version' }, 400);
    const kept = await stubOf(env, snap.site).put(snap);
    return json(kept);
  }

  /** POST /e onto the queue, or to Core when the queue cannot or should not take it. */
  async function queueEvents(request, env) {
    const raw = await request.clone().arrayBuffer();
    let body = null;
    try {
      body = JSON.parse(new TextDecoder().decode(raw));
    } catch {}
    if (raw.byteLength > MAX_QUEUED_BYTES || !queueable(body)) return direct(request, env, raw, 'not-queueable');
    try {
      await env.EVENTS_QUEUE.send({ kind: 'events', received_at: new Date(now()).toISOString(), request: body });
    } catch (e) {
      console.log(JSON.stringify({ msg: 'event enqueue failed, sending direct', site: body.site, error: String(e) }));
      return direct(request, env, raw, 'enqueue-failed');
    }
    const headers = { 'x-prb-queued': 'yes', 'cache-control': 'no-store' };
    const origin = request.headers.get('origin');
    const allowed = (env.SITE_ORIGINS || '').split(',').map((o) => o.trim()).filter(Boolean);
    if (origin && allowed.includes(origin)) Object.assign(headers, { 'access-control-allow-origin': origin, vary: 'origin' });
    return json({ queued: body.events.length }, 202, headers);
  }

  async function direct(request, env, raw, why) {
    const res = await proxy(request, env, { body: raw });
    const out = new Headers(res.headers);
    out.set('x-prb-queued', `no; ${why}`);
    return new Response(res.body, { status: res.status, statusText: res.statusText, headers: out });
  }

  /** The consumer: one /e/batch per delivery; ack only on Core's 2xx, else retry with backoff. */
  async function consume(batch, env) {
    const messages = batch.messages;
    if (messages.length === 0) return;
    const attempts = Math.max(...messages.map((m) => m.attempts || 1));
    const retry = (why, status) => {
      const delaySeconds = RETRY_DELAYS_S[Math.min(attempts, RETRY_DELAYS_S.length) - 1];
      console.log(JSON.stringify({ msg: 'event batch not taken, retrying', why, status, messages: messages.length, attempts, delaySeconds }));
      batch.retryAll({ delaySeconds });
    };
    const started = now();
    let res;
    try {
      res = await fetch(new URL('/e/batch', env.ORIGIN), {
        method: 'POST',
        headers: { 'content-type': 'application/json', authorization: `Bearer ${env.EDGE_TOKEN}` },
        body: JSON.stringify({ items: messages.map((m) => m.body) }),
      });
    } catch (e) {
      return retry(String(e));
    }
    if (res.status < 200 || res.status > 299) return retry('status', res.status);
    let out = null;
    try {
      out = await res.json();
    } catch {}
    batch.ackAll();
    const lagMs = Math.max(...messages.map((m) => started - +new Date(m.timestamp)));
    const refused = (out && out.refused) || [];
    console.log(JSON.stringify({
      msg: 'event batch', messages: messages.length, attempts, accepted: out?.accepted?.length ?? null, duplicate: out?.duplicate?.length ?? null,
      refused: refused.length, lag_ms: lagMs, core_ms: now() - started,
    }));
    for (const r of refused.slice(0, 20)) console.log(JSON.stringify({ msg: 'event refused', id: r.id, reason: r.reason }));
  }

  return {
    queue: (batch, env) => consume(batch, env),
    async fetch(request, env, ctx) {
      const path = new URL(request.url).pathname;
      if (request.method === 'POST' && path === PUSH_PATH) return push(request, env);
      if (request.method === 'POST' && path === '/e' && queueOn(env)) return queueEvents(request, env);
      const decide = request.method === 'POST' && DECIDE_PATHS.has(path);
      if (decide && env.EDGE_DECIDE === 'on' && env.EDGE_TOKEN && env.BEACON) return edgeDecide(request, env, ctx, path);
      return proxyWithAge(request, env, decide);
    },
  };
}

/** EC3's switch: the write buffer is on, bound, and has a token for /e/batch. */
export function queueOn(env) {
  return env.QUEUE_WRITES === 'on' && !!env.EVENTS_QUEUE && !!env.EDGE_TOKEN;
}

/** Forward to Core, rewriting what a proxy must. `opts.body` replaces a body already read. */
async function proxy(request, env, opts = {}) {
  const inUrl = new URL(request.url);
  const origin = new URL(env.ORIGIN);
  const url = new URL(inUrl.pathname + inUrl.search, origin);
  const headers = new Headers(request.headers);
  headers.delete('host');
  headers.delete('x-prb-edge-token');
  const ip = request.headers.get('cf-connecting-ip');
  if (ip) {
    const prior = request.headers.get('x-forwarded-for');
    headers.set('x-forwarded-for', prior ? `${prior}, ${ip}` : ip);
    headers.set('x-real-ip', ip);
  }
  // No X-Forwarded-Host: Google's front end routes on it and answers 404 for a run.app
  // service when it names another host.
  headers.set('x-forwarded-proto', inUrl.protocol.replace(':', ''));
  if (request.cf?.timezone) headers.set('x-prb-geo-tz', request.cf.timezone);
  if (request.cf?.country) headers.set('x-prb-geo-country', request.cf.country);
  if (opts.edgeToken) headers.set('x-prb-edge-token', opts.edgeToken);
  const hasBody = !['GET', 'HEAD'].includes(request.method);
  const init = { method: request.method, headers, body: hasBody ? (opts.body ?? request.body) : null, redirect: 'manual' };
  if (inUrl.pathname !== '/sdk.js') init.cache = 'no-store';
  const upstream = await fetch(url, init);
  const out = new Headers(upstream.headers);
  // Only the edge token's holder may see what Core tells the edge.
  if (!opts.edgeToken) out.delete('x-prb-edge');
  const loc = out.get('location');
  if (loc) {
    try {
      const l = new URL(loc, origin);
      if (l.host === origin.host) { l.host = inUrl.host; l.protocol = inUrl.protocol; out.set('location', l.toString()); }
    } catch {}
  }
  return new Response(upstream.body, { status: upstream.status, statusText: upstream.statusText, headers: out });
}

/** EC1's path: forward, and on a decide report the KV snapshot's age alongside. */
async function proxyWithAge(request, env, decide) {
  // A clone reads the site; the original body streams to Core untouched.
  const snapshot = decide ? siteOf(request.clone()).then((site) => snapshotAge(env, site).then((s) => ({ site, ...s }))) : null;
  const [res, snap] = await Promise.all([proxy(request, env), snapshot]);
  if (!snap) return res;
  const out = new Headers(res.headers);
  out.set('x-prb-snapshot', snap.state);
  if (snap.state === 'ok') {
    out.set('x-prb-snapshot-version', String(snap.version));
    out.set('x-prb-snapshot-age-ms', String(snap.age_ms));
  }
  console.log(JSON.stringify({ msg: 'edge snapshot', path: new URL(request.url).pathname, site: snap.site, state: snap.state, version: snap.version, age_ms: snap.age_ms, colo: request.cf?.colo }));
  return new Response(res.body, { status: res.status, statusText: res.statusText, headers: out });
}

export default createWorker();
