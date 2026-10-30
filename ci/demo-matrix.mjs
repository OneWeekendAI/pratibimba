// L3: the local demo matrix. Brought up by ci/demo-matrix.sh; see the README ("Local demo matrix").
//
// Starts, all on this machine: Core (direct, no edge), Core (edge-configured), `pratibimba worker`,
// a fake Cloudflare KV REST API that mirrors into wrangler's local KV, the proxy Worker under
// `wrangler dev` (miniflare: local KV, Durable Object, queue), and examples/demo-site on a static
// server. Then, per stack mode (direct, and the Worker with edge decide on/off x queue on/off), it
// exercises slot kinds x audiences x consent x erase and prints a PASS/FAIL/SKIP table. Every row
// asserts something observable: the DOM in real Chromium, a response header, or a database row.
// A mode that cannot run here is SKIP with its reason, never PASS.
//
//   --record          Playwright video of every visit, under target/demo-matrix/videos
//   --prove           after the matrix, break one thing per category and show its row go red
//   --mutate=NAME     run only that mutation's modes with it applied (rows are expected to fail)
//   --modes=a,b       only those modes (direct, w-off-off, w-on-off, w-off-on, w-on-on)
import { execFileSync, spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import http from 'node:http';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import zlib from 'node:zlib';
import { randomUUID } from 'node:crypto';

const ROOT = path.resolve(import.meta.dirname, '..');
const args = process.argv.slice(2);
const flag = (n) => args.includes(`--${n}`);
const opt = (n) => args.find((a) => a.startsWith(`--${n}=`))?.slice(n.length + 3);
const RECORD = flag('record');
const PROVE = flag('prove');
const ONLY_MUTATION = opt('mutate');

const P = Number(process.env.PRB_MATRIX_PORT ?? 18180);
const PORT = { direct: P, edge: P + 1, worker: P + 7, demo: P + 8, kv: P + 9 };
const URLS = { direct: `http://127.0.0.1:${PORT.direct}`, edge: `http://127.0.0.1:${PORT.edge}`, worker: `http://127.0.0.1:${PORT.worker}`, demo: `http://127.0.0.1:${PORT.demo}` };
const SITE = 'matrix';
const TOKEN = 'matrix-admin-token';
const EDGE_TOKEN = 'matrix-edge-token';
const ADMIN_DB = process.env.PRB_MATRIX_ADMIN_DB;
if (!ADMIN_DB) throw new Error('run through ci/demo-matrix.sh (it sets PRB_MATRIX_ADMIN_DB)');
const DB = ADMIN_DB.replace(/\/[^/]*$/, '/prb_matrix');
const BIN = path.join(ROOT, 'target/debug/pratibimba');
const WORK = fs.mkdtempSync(path.join(os.tmpdir(), 'prb-matrix-'));
const OUT = path.join(ROOT, 'target/demo-matrix');
const VIDEOS = path.join(OUT, 'videos');
const DEMO_DIR = path.join(ROOT, 'examples/demo-site');
const SDK_FILE = path.join(ROOT, 'packages/client/sdk.js');

// ---------------------------------------------------------------- small helpers
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const psql = (sql) => execFileSync('psql', [DB, '-qAt', '-c', sql], { stdio: 'pipe' }).toString().trim();
const psqlAdmin = (sql) => execFileSync('psql', [ADMIN_DB, '-qAt', '-c', sql], { stdio: 'pipe' }).toString().trim();
const kids = [];
function start(name, cmd, argv, env = {}, opts = {}) {
  const k = spawn(cmd, argv, { env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'], detached: true, ...opts });
  k.log = '';
  k.name = name;
  k.stdout.on('data', (d) => { k.log += d; });
  k.stderr.on('data', (d) => { k.log += d; });
  kids.push(k);
  return k;
}
function stop(k) {
  if (!k || k.exitCode !== null || k.signalCode) return;
  try { process.kill(-k.pid, 'SIGTERM'); } catch { /* gone */ }
}
async function until(what, fn, seconds = 30) {
  const end = Date.now() + seconds * 1000;
  let last;
  while (Date.now() < end) {
    try { const v = await fn(); if (v) return v; } catch (e) { last = e; }
    await sleep(150);
  }
  throw new Error(`timed out waiting for ${what}${last ? `: ${last.message}` : ''}`);
}
const api = (p, body, method = body === undefined ? 'GET' : 'POST') =>
  fetch(URLS.direct + '/api' + p, { method, headers: { 'content-type': 'application/json', authorization: `Bearer ${TOKEN}` }, body: body === undefined ? undefined : JSON.stringify(body) });
const post = (base, p, body) => fetch(base + p, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });

// ---------------------------------------------------------------- Playwright (from any sibling install)
function loadPlaywright() {
  for (const dir of ['examples/demo-site', 'examples/next', 'examples/edge', 'dashboard', 'packages/agent']) {
    try { return createRequire(path.join(ROOT, dir, 'package.json'))('playwright'); } catch { /* try the next */ }
  }
  throw new Error('playwright not found: run `npm install` in examples/demo-site');
}

// ---------------------------------------------------------------- a PNG with no dependencies
function png(w, h) {
  const crcTable = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
  const crc = (b) => { let c = 0xffffffff; for (const x of b) c = crcTable[(c ^ x) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
  const chunk = (t, d) => { const len = Buffer.alloc(4); len.writeUInt32BE(d.length); const td = Buffer.concat([Buffer.from(t), d]); const c = Buffer.alloc(4); c.writeUInt32BE(crc(td)); return Buffer.concat([len, td, c]); };
  const raw = Buffer.alloc((w * 3 + 1) * h);
  for (let y = 0; y < h; y++) {
    raw[y * (w * 3 + 1)] = 0;
    for (let x = 0; x < w; x++) {
      const n = (Math.imul(x, 2654435761) ^ Math.imul(y, 40503)) >>> 7;
      const o = y * (w * 3 + 1) + 1 + x * 3;
      raw[o] = ((x * 255) / w) ^ (n & 15); raw[o + 1] = (y * 255) / h; raw[o + 2] = n % 97;
    }
  }
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}

// ---------------------------------------------------------------- the demo site's static server
let mutation = null; // an SDK mutation applied to what /__sdk.js serves (--prove)
const SDK_MUTATIONS = {
  swap: ["else if (d.payload !== null && d.payload !== undefined) el.innerHTML = d.payload;", "else if (false) el.innerHTML = d.payload;"],
  copy: ["else if (d.copy && typeof d.copy === 'object') writeCopy(el, d.copy);", "else if (false) writeCopy(el, d.copy);"],
  image: ["if (rs) { img.setAttribute('srcset', rs.srcset);", "if (false) { img.setAttribute('srcset', rs.srcset);"],
  sorted: ['if (sorted) shown = reorder(el, d.order);', 'if (sorted) shown = [];'],
  consent: ["var useCookie = mode === 'full' || (mode === 'consent_aware' && consented());", 'var useCookie = true;'],
};
function sdkSource() {
  let src = fs.readFileSync(SDK_FILE, 'utf8');
  const m = mutation && SDK_MUTATIONS[mutation];
  if (m) {
    if (!src.includes(m[0])) throw new Error(`mutation ${mutation}: the SDK line it breaks has changed; update SDK_MUTATIONS`);
    src = src.replace(m[0], m[1]);
  }
  return src;
}
function demoServer() {
  const html = fs.readFileSync(path.join(DEMO_DIR, 'index.html'), 'utf8');
  // The site's own origin forwards /_p/<direct|worker>/* to Core or to the Worker in front of it:
  // the production pattern (README: proxy /_prb/* on your own origin), so no CORS is involved.
  const upstream = { direct: URLS.direct, worker: URLS.worker };
  const srv = http.createServer((req, res) => {
    const u = new URL(req.url, URLS.demo);
    const px = /^\/_p\/(direct|worker)(\/.*)$/.exec(u.pathname);
    if (px) {
      const chunks = [];
      req.on('data', (c) => chunks.push(c));
      req.on('end', async () => {
        try {
          const headers = { ...req.headers };
          delete headers.host; delete headers.connection; delete headers['content-length'];
          const body = chunks.length ? Buffer.concat(chunks) : undefined;
          const r = await fetch(upstream[px[1]] + px[2] + u.search, { method: req.method, headers, body: ['GET', 'HEAD'].includes(req.method) ? undefined : body, redirect: 'manual' });
          const out = Object.fromEntries([...r.headers].filter(([k]) => !['content-encoding', 'content-length', 'transfer-encoding', 'connection'].includes(k)));
          res.writeHead(r.status, out);
          res.end(Buffer.from(await r.arrayBuffer()));
        } catch (e) { res.writeHead(502); res.end(String(e)); }
      });
      return;
    }
    if (u.pathname === '/__sdk.js') { res.writeHead(200, { 'content-type': 'text/javascript', 'cache-control': 'no-store' }); return res.end(sdkSource()); }
    if (u.pathname.startsWith('/img/')) {
      const f = path.join(DEMO_DIR, 'img', path.basename(u.pathname));
      if (fs.existsSync(f)) { res.writeHead(200, { 'content-type': 'image/svg+xml' }); return res.end(fs.readFileSync(f)); }
    }
    if (u.pathname === '/') {
      const via = u.searchParams.get('via') === 'worker' ? 'worker' : 'direct';
      const body = html.replaceAll('__BASE__', `/_p/${via}`).replaceAll('__COOKIE__', u.searchParams.get('consent') === 'on' ? 'data-cookie="on"' : '');
      res.writeHead(200, { 'content-type': 'text/html', 'cache-control': 'no-store' });
      return res.end(body);
    }
    res.writeHead(404); res.end();
  });
  return new Promise((ok) => srv.listen(PORT.demo, '127.0.0.1', () => ok(srv)));
}

// ---------------------------------------------------------------- fake Cloudflare KV -> wrangler local KV
const kvStore = new Map();
let kvBusy = Promise.resolve();
let wranglerConfig = null;
const STATE = path.join(WORK, 'wrangler-state');
function mirror(key, value) {
  kvBusy = kvBusy.then(() => new Promise((done) => {
    if (!wranglerConfig) return done();
    const f = path.join(WORK, `kv-${randomUUID()}.json`);
    const argv = ['kv', 'key', value === null ? 'delete' : 'put', '--binding', 'SNAPSHOTS', '--local', '--persist-to', STATE, '-c', wranglerConfig, key];
    if (value !== null) { fs.writeFileSync(f, value); argv.push('--path', f); }
    const k = spawn('wrangler', argv, { stdio: 'ignore', env: { ...process.env, WRANGLER_SEND_METRICS: 'false' } });
    k.on('exit', () => done());
    k.on('error', () => done());
  }));
}
function kvServer() {
  const srv = http.createServer((req, res) => {
    const m = /\/values\/([^/?]+)/.exec(req.url);
    const key = m && decodeURIComponent(m[1]);
    if (!key) { res.writeHead(404); return res.end(); }
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => {
      if (req.method === 'PUT') { const v = Buffer.concat(chunks).toString(); kvStore.set(key, v); mirror(key, v); }
      else if (req.method === 'DELETE') { kvStore.delete(key); mirror(key, null); }
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end('{"success":true,"errors":[],"messages":[],"result":null}');
    });
  });
  return new Promise((ok) => srv.listen(PORT.kv, '127.0.0.1', () => ok(srv)));
}

// ---------------------------------------------------------------- the stack
const coreEnv = {
  PRB_DB: DB, PRB_ADMIN_TOKEN: TOKEN, RUST_LOG: 'warn', PRB_WORKER_TICK_MS: '500',
  PRB_ASSETS_DIR: path.join(WORK, 'assets'), PRB_DASHBOARD_DIR: path.join(ROOT, 'dashboard/out'),
};
const edgeEnv = {
  PRB_EDGE_KV_ACCOUNT: 'acct', PRB_EDGE_KV_NAMESPACE: 'ns', PRB_EDGE_KV_TOKEN: 'kv-token', PRB_EDGE_KV_API: `http://127.0.0.1:${PORT.kv}/client/v4`,
  PRB_EDGE_TOKEN: EDGE_TOKEN, PRB_EDGE_BEACON_URL: `${URLS.worker}/_prb/edge/snapshot`,
};
let worker = null;
let workerReason = null; // why Worker modes cannot run, once known

async function startWorker(decide, queue) {
  stop(worker);
  if (worker) await until('wrangler to exit', () => worker.exitCode !== null || worker.signalCode, 15).catch(() => {});
  wranglerConfig = path.join(WORK, 'wrangler.jsonc');
  fs.writeFileSync(wranglerConfig, JSON.stringify({
    name: 'prb-local-proxy',
    main: path.join(ROOT, 'deploy/proxy-worker/src/worker.js'),
    // miniflare in the installed wrangler supports up to this date; the checked-in config's is newer.
    compatibility_date: '2026-07-01',
    vars: { ORIGIN: URLS.edge, EDGE_DECIDE: decide, QUEUE_WRITES: queue, EDGE_TOKEN, SITE_ORIGINS: URLS.demo },
    kv_namespaces: [{ binding: 'SNAPSHOTS', id: 'local-snapshots' }],
    durable_objects: { bindings: [{ name: 'BEACON', class_name: 'SiteBeacon' }] },
    migrations: [{ tag: 'ec2-beacon', new_sqlite_classes: ['SiteBeacon'] }],
    queues: { producers: [{ binding: 'EVENTS_QUEUE', queue: 'pratibimba-events' }], consumers: [{ queue: 'pratibimba-events', max_batch_size: 100, max_batch_timeout: 1, max_retries: 3 }] },
  }, null, 2));
  worker = start('wrangler', 'wrangler', ['dev', '-c', wranglerConfig, '--port', String(PORT.worker), '--persist-to', STATE, '--log-level', 'warn'],
    { WRANGLER_SEND_METRICS: 'false', CI: '1' }, { cwd: WORK });
  await until('the Worker to answer', async () => (await fetch(`${URLS.worker}/health`)).status > 0, 60).catch((e) => {
    throw new Error(`wrangler dev did not start: ${e.message}\n${worker.log.slice(-600)}`);
  });
}

async function settleEdge() {
  // Snapshot published by the worker process, mirrored into the Worker's local KV.
  let lastV = null, stable = 0;
  const end = Date.now() + 30000;
  while (Date.now() < end && stable < 6) {
    const v = psql(`SELECT coalesce(max(version),0) FROM edge_snapshots WHERE site_id='${SITE}'`);
    stable = v === lastV && v !== '0' ? stable + 1 : 0;
    lastV = v;
    await sleep(500);
  }
  await kvBusy;
}

// ---------------------------------------------------------------- site setup
let assetId = null;
async function applySite() {
  const text = fs.readFileSync(path.join(DEMO_DIR, 'site.json'), 'utf8').replace('__ASSET__', assetId);
  const f = path.join(WORK, 'site.json');
  fs.writeFileSync(f, text);
  execFileSync(BIN, ['apply', f], { env: { ...process.env, ...coreEnv }, stdio: 'pipe' });
}
const variantId = (key) => Number(psql(`SELECT id FROM variants WHERE key='${key}' AND slot_id IN (SELECT id FROM slots WHERE site_id='${SITE}') ORDER BY id LIMIT 1`));
const audienceOf = {};
async function makeAudience(name, predicate, viaAgent) {
  let id;
  if (viaAgent) {
    const t = (await (await api(`/sites/${SITE}/mcp-tokens`, { name: 'demo matrix agent' })).json()).token;
    const r = await fetch(URLS.direct + '/mcp', {
      method: 'POST', headers: { 'content-type': 'application/json', accept: 'application/json, text/event-stream', authorization: `Bearer ${t}` },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'create_audience', arguments: { agent: 'Claude Code', justification: 'demo matrix: owners with two or more orders', predicate, name } } }),
    });
    const j = await r.json();
    if (j.result?.isError) throw new Error('create_audience: ' + j.result.content[0].text);
    id = JSON.parse(j.result.content[0].text).id;
  } else {
    const r = await api(`/sites/${SITE}/segments`, { predicate, name, actor: 'matrix' });
    id = (await r.json()).id;
  }
  const p = await api(`/segments/${id}/promote`, { actor: 'matrix (a person promotes)' });
  if (!p.ok) throw new Error('promote: ' + (await p.text()));
  audienceOf[name] = id;
  return id;
}
const restrict = (variant, audience) => api(`/variants/${variantId(variant)}/audience`, { audience, actor: 'matrix' });
async function setupAudiences() {
  const nw = await makeAudience('new visitors', { returning: false }, false);
  const rt = await makeAudience('returning visitors', { returning: true }, false);
  const hs = await makeAudience('two or more orders', { purchase_count: { gte: 2 } }, true);
  for (const [v, a] of [['welcome', nw], ['gift', rt], ['loyal', hs]]) {
    const r = await restrict(v, a);
    if (!r.ok) throw new Error(`restrict ${v}: ${await r.text()}`);
  }
}

// History: two orders for each seeded subject, written through Core, waited for in live_events.
async function seedHistory(prefix, n) {
  const subjects = [];
  for (let i = 0; i < n; i++) {
    const s = `${prefix}-h${i}`;
    subjects.push(s);
    const r = await post(URLS.direct, '/e', { site: SITE, subject: s, consent: true, events: [
      { id: randomUUID(), name: 'purchase', order_id: `${s}-1`, value: 120, currency: 'USD', items: [{ id: 'amber' }] },
      { id: randomUUID(), name: 'purchase', order_id: `${s}-2`, value: 80, currency: 'USD', items: [{ id: 'birch' }] },
    ] });
    if (r.status !== 202 && r.status !== 200) throw new Error(`seed history: ${r.status}`);
  }
  await until('history rows', () => Number(psql(`SELECT count(*) FROM live_events WHERE site_id='${SITE}' AND subject LIKE '${prefix}-h%' AND name='purchase'`)) >= n * 2, 30);
  return subjects;
}

// ---------------------------------------------------------------- one browser visit
let pw, browser, rowLabel = '', activeMutation = null;
async function visit(base, { consent = true, subject = null, tz = 'Asia/Kolkata' } = {}) {
  const opts = { timezoneId: tz, viewport: { width: 900, height: 720 } };
  if (RECORD) opts.recordVideo = { dir: VIDEOS, size: { width: 900, height: 720 } };
  const ctx = await browser.newContext(opts);
  try {
    if (subject && consent) await ctx.addCookies([{ name: '_prb_subject', value: subject, url: URLS.demo }]);
    if (subject && !consent) await ctx.addInitScript((s) => { sessionStorage.setItem('prb_subject', s); }, subject);
    const page = await ctx.newPage();
    const decides = [], events = [];
    page.on('response', async (r) => {
      if (!/\/decide$/.test(new URL(r.url()).pathname)) return;
      let body = null;
      try { body = await r.json(); } catch { /* 410 etc. */ }
      let slot = null;
      try { slot = JSON.parse(r.request().postData() ?? '{}').slot; } catch { /* none */ }
      decides.push({ slot, status: r.status(), by: r.headers()['x-prb-decided-by'] ?? 'core', body });
    });
    page.on('request', (r) => {
      if (r.method() === 'POST' && /\/e$/.test(new URL(r.url()).pathname)) { try { events.push(JSON.parse(r.postData())); } catch { /* none */ } }
    });
    await page.goto(`${URLS.demo}/?via=${base === URLS.worker ? 'worker' : 'direct'}&consent=${consent ? 'on' : 'off'}`);
    if (RECORD) await page.evaluate((t) => { const d = document.createElement('div'); d.textContent = t; d.style.cssText = 'position:fixed;right:8px;bottom:8px;background:#111;color:#fff;font:12px monospace;padding:6px 10px;z-index:99999;opacity:.9'; document.body.appendChild(d); }, rowLabel);
    await page.waitForFunction(() => document.querySelectorAll('[data-prb-done]').length >= 4, null, { timeout: 8000 }).catch(() => {});
    await sleep(RECORD ? 1500 : 250);
    const dom = await page.evaluate(() => {
      const $ = (s) => document.querySelector(s);
      const img = $('#banner img');
      return {
        hero: { variant: $('#hero').getAttribute('data-prb-variant'), text: $('#hero').textContent.trim() },
        headline: { variant: $('#headline').getAttribute('data-prb-variant'), text: $('#headline [data-prb-field=headline]').textContent.trim(), cta: $('#headline [data-prb-field=cta]').textContent.trim() },
        banner: { variant: $('#banner').getAttribute('data-prb-variant'), src: img.getAttribute('src'), srcset: img.getAttribute('srcset'), sizes: img.getAttribute('sizes'), loaded: img.complete && img.naturalWidth > 0 },
        grid: { variant: $('#grid').getAttribute('data-prb-variant'), order: [...document.querySelectorAll('#grid > [data-prb-item]')].map((e) => e.getAttribute('data-prb-item')) },
      };
    });
    const cookies = (await ctx.cookies()).map((c) => c.name);
    const session = await page.evaluate(() => ({ subject: sessionStorage.getItem('prb_subject') }));
    const cookieSubject = (await ctx.cookies()).find((c) => c.name === '_prb_subject')?.value ?? null;
    await sleep(RECORD ? 800 : 0);
    return { decides, events, dom, cookies, subject: cookieSubject ?? session.subject };
  } finally {
    await ctx.close();
  }
}
const dec = (v, slot) => v.decides.find((d) => d.slot === slot);

// ---------------------------------------------------------------- rows
const results = [];
function record(mode, category, name, status, detail = '') {
  const r = { mode, category, name, status, detail };
  results.push(r);
  return r;
}
async function row(mode, category, name, fn) {
  rowLabel = `${mode} | ${name}`;
  try {
    const out = await fn();
    if (out && out.skip) return record(mode, category, name, 'SKIP', out.skip);
    return record(mode, category, name, 'PASS', typeof out === 'string' ? out : '');
  } catch (e) {
    return record(mode, category, name, 'FAIL', String(e.message ?? e).split('\n')[0].slice(0, 200));
  }
}
const need = (cond, msg) => { if (!cond) throw new Error(msg); };

/** Fresh visits until `pick` returns a truthy description; every visit is also run through `always`. */
async function tryVisits(base, visitOpts, tries, pick, always) {
  for (let i = 0; i < tries; i++) {
    const v = await visit(base, typeof visitOpts === 'function' ? visitOpts(i) : visitOpts);
    always?.(v);
    const got = pick(v);
    if (got) return got;
  }
  throw new Error(`never saw the expected variant in ${tries} visits`);
}

async function modeRows(mode, base, decideOn, queueOn, ops) {
  const tag = `${mode}-${Date.now().toString(36)}`;
  const mut = activeMutation;
  const history = await seedHistory(tag, 14);

  // --- swap kind x audiences -------------------------------------------------------------
  await row(mode, 'swap', 'swap | new visitor, consent on', async () => {
    return tryVisits(base, { consent: true }, 14, (v) => {
      const h = v.dom.hero;
      return h.variant === 'welcome' && /Ten percent off/.test(h.text) ? 'DOM shows the welcome variant' : null;
    }, (v) => {
      need(v.dom.hero.variant !== 'gift', 'a new visitor was shown the returning-only gift');
      need(dec(v, 'hero')?.body?.variant === v.dom.hero.variant, `DOM variant ${v.dom.hero.variant} differs from the decision ${dec(v, 'hero')?.body?.variant}`);
    });
  });
  await row(mode, 'swap', 'swap | returning visitor, consent on', async () => {
    return tryVisits(base, (i) => ({ consent: true, subject: `${tag}-ret${i}` }), 14, (v) => {
      if (v.dom.hero.variant !== 'gift' || !/gift sets/.test(v.dom.hero.text)) return null;
      const by = dec(v, 'hero').by, want = decideOn ? 'edge' : 'core';
      need(by === want, `the page's decide was answered by ${by}, expected ${want}`);
      return `DOM shows the gift variant (decided by ${by})`;
    }, (v) => {
      need(v.dom.hero.variant !== 'welcome', 'a returning visitor was shown the new-only welcome');
    });
  });
  await row(mode, 'audience', 'audience | gates hold over 40 decisions each (new/returning)', async () => {
    const seen = { new: new Set(), ret: new Set() };
    for (let i = 0; i < 40; i++) {
      for (const [k, ret] of [['new', false], ['ret', true]]) {
        const r = await post(base, '/decide', { site: SITE, slot: 'hero', subject: `${tag}-g${k}${i}`, ctx: { returning: ret, consent: true } });
        need(r.status === 200, `decide ${r.status}`);
        seen[k].add((await r.json()).variant);
      }
    }
    need(!seen.new.has('gift'), `new visitors saw gift (${[...seen.new]})`);
    need(!seen.ret.has('welcome'), `returning visitors saw welcome (${[...seen.ret]})`);
    need(seen.new.has('welcome') && seen.ret.has('gift'), `audience variants never served (${[...seen.new]} / ${[...seen.ret]})`);
    return `new -> ${[...seen.new].sort()}; returning -> ${[...seen.ret].sort()}`;
  });
  await row(mode, 'audience', 'audience | agent-drafted history audience (template copy)', async () => {
    // Visitors with two orders: the loyal variant's placeholder fills from their history.
    const got = await tryVisits(base, (i) => ({ consent: true, subject: history[i % history.length] }), history.length, (v) => {
      const h = v.dom.headline;
      return h.variant === 'loyal' && h.text === 'You have placed 2 orders' && h.cta === 'Reorder' ? h.text : null;
    });
    // And nobody without that history is ever offered it.
    for (let i = 0; i < 25; i++) {
      const r = await post(base, '/decide', { site: SITE, slot: 'headline', subject: `${tag}-nohist${i}`, ctx: { returning: true, consent: true } });
      need((await r.json()).variant !== 'loyal', 'loyal copy served to a visitor with no orders');
    }
    return got;
  });

  // --- copy / image / sorted -------------------------------------------------------------
  await row(mode, 'copy', 'copy | plain text fields', async () => {
    return tryVisits(base, { consent: true }, 14, (v) => {
      const h = v.dom.headline;
      return h.variant === 'plain' && h.text === 'Fresh roasts every week' && h.cta === 'Shop' ? h.text : null;
    });
  });
  await row(mode, 'image', 'image | uploaded asset, srcset', async () => {
    return tryVisits(base, { consent: true }, 14, (v) => {
      const b = v.dom.banner;
      if (b.variant !== 'promo') return null;
      need(b.src.includes(`/assets/${SITE}/${assetId}`), `src is ${b.src}`);
      need(/\?w=320 320w/.test(b.srcset ?? '') && /1400w/.test(b.srcset ?? ''), `srcset is ${b.srcset}`);
      need(/640px/.test(b.sizes ?? ''), `sizes is ${b.sizes}`);
      need(b.loaded, 'the image did not load');
      return `srcset widths ${(b.srcset.match(/\d+w/g) ?? []).join(' ')}`;
    });
  });
  await row(mode, 'sorted', 'sorted | order follows the decision', async () => {
    const page = ['amber', 'birch', 'cedar', 'dune'];
    return tryVisits(base, { consent: true }, 14, (v) => {
      const want = dec(v, 'grid')?.body?.order;
      need(Array.isArray(want), 'the grid decision carried no order');
      need(JSON.stringify(v.dom.grid.order) === JSON.stringify(want), `DOM ${v.dom.grid.order} != decision ${want}`);
      return JSON.stringify(want) !== JSON.stringify(page) ? `DOM order ${v.dom.grid.order.join(',')}` : null;
    });
  });

  // --- consent ---------------------------------------------------------------------------
  await row(mode, 'consent', 'consent on | cookie subject stored raw, returning honoured', async () => {
    const s = `${tag}-consent`;
    const v = await visit(base, { consent: true, subject: s });
    need(v.cookies.includes('_prb_subject'), 'no first-party cookie with consent');
    need(v.events.length && v.events.every((e) => e.consent === true), 'events did not say consent:true');
    need(dec(v, 'hero')?.body, 'no decision');
    await until('consented events stored', () => Number(psql(`SELECT count(*) FROM live_events WHERE site_id='${SITE}' AND subject='${s}'`)) > 0, 20);
    const tz = psql(`SELECT max(tz_offset_min) FROM live_events WHERE site_id='${SITE}' AND subject='${s}'`);
    need(tz === '330', `consented events carry tz_offset_min ${tz}, expected 330`);
    return 'raw subject stored, tz 330 (Asia/Kolkata)';
  });
  await row(mode, 'consent', 'consent off | no cookie, pseudonymous, never returning', async () => {
    let stored = 0;
    let subject = null;
    for (let i = 0; i < 12; i++) {
      const v = await visit(base, { consent: false, subject: `${tag}-anon${i}` });
      need(!v.cookies.includes('_prb_subject'), 'a cookie was set without consent');
      need(v.events.length && v.events.every((e) => e.consent === false), 'events did not say consent:false');
      need(v.dom.hero.variant !== 'gift', 'a visitor without consent was treated as returning');
      subject = `${tag}-anon${i}`;
      if (i === 0) break;
    }
    await until('anonymous events stored', () => (stored = Number(psql(`SELECT count(*) FROM live_events WHERE site_id='${SITE}' AND subject LIKE 'p1_%' AND received_at > now() - interval '60 seconds'`))) > 0, 20);
    const raw = Number(psql(`SELECT count(*) FROM live_events WHERE site_id='${SITE}' AND subject='${subject}'`));
    need(raw === 0, 'the raw client id of a cookieless visitor was stored');
    return `${stored} pseudonymous rows, 0 under the raw id`;
  });

  // --- edge decide ---------------------------------------------------------------------
  await row(mode, 'edge', mode === 'direct' ? 'edge decide | not applicable' : decideOn ? 'edge decide on | consented swap decided at the edge' : 'edge decide off | every decide answered by Core', async () => {
    if (mode === 'direct') return { skip: 'no Worker in front of Core in the direct mode' };
    const by = new Set();
    for (let i = 0; i < 6; i++) {
      const r = await post(base, '/decide', { site: SITE, slot: 'hero', subject: `${tag}-edge${i}`, ctx: { returning: true, consent: true } });
      need(r.status === 200, `decide ${r.status}`);
      by.add(r.headers.get('x-prb-decided-by') ?? 'core');
    }
    if (decideOn) need(by.has('edge'), `no decide answered by the edge (x-prb-decided-by: ${[...by]})`);
    else need(!by.has('edge') && by.has('core'), `decide was not forwarded to Core (x-prb-decided-by: ${[...by]})`);
    // Copy and sorted always fall through to Core, even with edge decide on.
    const c = await post(base, '/decide', { site: SITE, slot: 'grid', subject: `${tag}-edgeg`, ctx: { consent: true } });
    need((c.headers.get('x-prb-decided-by') ?? 'core') === 'core', `sorted decide was answered by ${c.headers.get('x-prb-decided-by')}, expected core`);
    return `swap decided by ${[...by]}; sorted by core`;
  });

  // --- write queue ---------------------------------------------------------------------
  await row(mode, 'queue', mode === 'direct' ? 'write queue | direct to Core (no buffer in this mode)' : queueOn ? 'write queue on | /e buffered, then drained into Core' : 'write queue off | /e written straight to Core', async () => {
    const s = `${tag}-queue`;
    const id = randomUUID();
    const r = await post(base, '/e', { site: SITE, subject: s, consent: true, events: [{ id, name: 'page_view' }] });
    need(r.status === 202 || r.status === 200, `/e answered ${r.status}`);
    const q = r.headers.get('x-prb-queued');
    if (mode === 'direct') need(q === null, `a direct /e was queued (${q})`);
    else if (queueOn) need(q === 'yes', `/e was not buffered (x-prb-queued: ${q})`);
    else need(q !== 'yes', `/e was buffered although the queue is off (x-prb-queued: ${q})`);
    await until('the event in live_events', () => Number(psql(`SELECT count(*) FROM live_events WHERE site_id='${SITE}' AND id='${id}'`)) === 1, 25);
    return `x-prb-queued: ${q ?? 'absent'}; event stored`;
  });

  // --- erase ---------------------------------------------------------------------------
  await row(mode, 'erase', 'erase | 410 and no live_events rows afterwards', async () => {
    const s = `${tag}-erased`;
    const v = await visit(base, { consent: true, subject: s });
    need(v.events.length, 'the visit sent no events');
    await until('events stored', () => Number(psql(`SELECT count(*) FROM live_events WHERE site_id='${SITE}' AND subject='${s}'`)) > 0, 20);
    const victim = mut === 'erase' ? `${s}-someone-else` : s;
    const e = await fetch(ops.eraseBase + `/api/sites/${SITE}/erase`, { method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${TOKEN}` }, body: JSON.stringify({ subject: victim, actor: 'matrix' }) });
    need(e.status === 200 || e.status === 201, `erase answered ${e.status}`);
    const d = await until('decide to answer 410', async () => (await post(base, '/decide', { site: SITE, slot: 'hero', subject: s, ctx: { consent: true } })).status === 410, 45).catch(() => null);
    need(d, 'decide still answers for an erased subject after 45 s');
    const w = await post(base, '/e', { site: SITE, subject: s, consent: true, events: [{ id: randomUUID(), name: 'page_view' }] });
    await sleep(queueOn ? 4000 : 800);
    const rows = Number(psql(`SELECT count(*) FROM live_events WHERE site_id='${SITE}' AND subject='${s}'`));
    need(rows === 0, `${rows} live_events rows remain for the erased subject`);
    return `decide 410, /e ${w.status}, 0 rows`;
  });
}

// Rows that do not depend on the mode, run once.
async function globalRows() {
  await row('global', 'swap', 'sdk | the demo page runs the repo SDK, same as Core serves', async () => {
    const served = await (await fetch(URLS.direct + '/sdk.js')).text();
    need(served === fs.readFileSync(SDK_FILE, 'utf8'), "Core's /sdk.js differs from packages/client/sdk.js (rebuild Core)");
  });
  await row('global', 'image', 'image | upload stored narrower widths', async () => {
    const list = await (await api(`/sites/${SITE}/assets`)).json();
    const a = list.assets.find((x) => x.id === assetId);
    need(a && JSON.stringify(a.widths) === JSON.stringify([320, 640, 960, 1280]), `widths ${JSON.stringify(a?.widths)}`);
    const w = await fetch(`${URLS.direct}/assets/${SITE}/${assetId}?w=640`);
    need(w.status === 200, `?w=640 answered ${w.status}`);
    return 'widths 320/640/960/1280';
  });
  await row('global', 'consent', 'tz | withheld when fields_allowed excludes tz_offset', async () => {
    const before = await (await api(`/sites/${SITE}/privacy`)).json();
    const fields = before.fields_allowed ?? ['returning', 'mobile', 'from_ad', 'stated', 'tz_offset', 'slot', 'decision_id', 'picks', 'rescore', 'history'];
    try {
      const w = await api(`/sites/${SITE}/privacy`, { fields_allowed: fields.filter((f) => f !== 'tz_offset') });
      need(w.ok, `privacy write ${w.status}`);
      const s = `tz-${Date.now().toString(36)}`;
      await visit(URLS.direct, { consent: true, subject: s });
      await until('events stored', () => Number(psql(`SELECT count(*) FROM live_events WHERE site_id='${SITE}' AND subject='${s}'`)) > 0, 20);
      const n = Number(psql(`SELECT count(*) FROM live_events WHERE site_id='${SITE}' AND subject='${s}' AND tz_offset_min IS NOT NULL`));
      need(n === 0, `${n} events kept a tz_offset the site does not collect`);
    } finally {
      await api(`/sites/${SITE}/privacy`, { fields_allowed: before.fields_allowed ?? null });
    }
  });
  record('global', 'consent', 'tz | consent off sends no tz', 'SKIP', 'Core gates tz by the site\'s fields_allowed, not by consent: a cookieless visit still stores tz_offset when the field is allowed');
  await row('global', 'dashboard', 'dashboard | sign in and see the demo site', async () => {
    const ctx = await browser.newContext(RECORD ? { recordVideo: { dir: VIDEOS, size: { width: 1100, height: 720 } }, viewport: { width: 1100, height: 720 } } : {});
    try {
      const page = await ctx.newPage();
      await page.goto(`${URLS.direct}/app/login/`);
      await page.getByLabel('Admin token').fill(TOKEN);
      await page.getByRole('button', { name: 'Sign in' }).click();
      await page.getByRole('heading', { name: SITE }).waitFor({ timeout: 15000 });
      if (RECORD) await sleep(1500);
    } finally { await ctx.close(); }
  });
}

// ---------------------------------------------------------------- modes and mutations
const MODES = {
  direct: { label: 'direct', decide: false, queue: false, worker: false },
  'w-off-off': { label: 'worker decide=off queue=off', decide: false, queue: false, worker: true },
  'w-on-off': { label: 'worker decide=on queue=off', decide: true, queue: false, worker: true },
  'w-off-on': { label: 'worker decide=off queue=on', decide: false, queue: true, worker: true },
  'w-on-on': { label: 'worker decide=on queue=on', decide: true, queue: true, worker: true },
};
// What each mutation breaks, where it runs, and the rows that must go red.
const MUTATIONS = {
  swap: { mode: 'direct', breaks: 'SDK stops writing a swap payload', red: /^swap \| (new|returning)/ },
  copy: { mode: 'direct', breaks: 'SDK stops writing copy fields', red: /^copy \|/ },
  image: { mode: 'direct', breaks: 'SDK stops writing the srcset', red: /^image \| uploaded/ },
  sorted: { mode: 'direct', breaks: 'SDK stops reordering items', red: /^sorted \|/ },
  consent: { mode: 'direct', breaks: 'SDK sets the cookie without consent', red: /^consent off/ },
  audience: { mode: 'direct', breaks: 'gift variant unrestricted from its audience', red: /^audience \| gates/ },
  edge: { mode: 'w-on-off', breaks: 'Worker runs with edge decide off while the mode says on', red: /^edge decide on/ },
  queue: { mode: 'w-off-on', breaks: 'Worker runs with the queue off while the mode says on', red: /^write queue on/ },
  erase: { mode: 'direct', breaks: 'erase names a different subject', red: /^erase \|/ },
};

async function runMode(key, mut = null) {
  const m = MODES[key];
  activeMutation = mut;
  const start0 = results.length;
  if (m.worker) {
    if (workerReason) { record(key, 'all', 'every row', 'SKIP', workerReason); return; }
    try {
      // Mutations edge/queue run the Worker with the setting flipped from what the mode claims.
      await startWorker(mut === 'edge' ? 'off' : m.decide ? 'on' : 'off', mut === 'queue' ? 'off' : m.queue ? 'on' : 'off');
      await settleEdge();
      if (m.decide) await until('edge-decided answers', async () => {
        const r = await post(URLS.worker, '/decide', { site: SITE, slot: 'hero', subject: `warm-${Date.now()}`, ctx: { returning: true, consent: true } });
        return r.headers.get('x-prb-decided-by') === 'edge';
      }, 40).catch(() => {});
    } catch (e) {
      workerReason = `the proxy Worker cannot run locally: ${String(e.message).split('\n')[0].slice(0, 160)}`;
      record(key, 'all', 'every row', 'SKIP', workerReason);
      return;
    }
  }
  const base = m.worker ? URLS.worker : URLS.direct;
  const ops = { eraseBase: m.worker ? URLS.edge : URLS.direct };
  await modeRows(key, base, m.decide, m.queue, ops);
  return results.slice(start0);
}

// ---------------------------------------------------------------- main
function table(rows) {
  const w = (k, min) => Math.max(min, ...rows.map((r) => String(r[k]).length));
  const wm = w('mode', 4), wn = w('name', 3);
  const line = (r) => `${String(r.status).padEnd(5)}  ${r.mode.padEnd(wm)}  ${r.name.padEnd(wn)}  ${r.detail}`;
  return [`${'     '}  ${'mode'.padEnd(wm)}  ${'row'.padEnd(wn)}  observed`, ...rows.map(line)].join('\n');
}

let exiting = false;
function cleanup() {
  if (exiting) return;
  exiting = true;
  for (const k of kids) stop(k);
  try { fs.rmSync(WORK, { recursive: true, force: true }); } catch { /* best effort */ }
}
process.on('exit', cleanup);
for (const s of ['SIGINT', 'SIGTERM']) process.on(s, () => { cleanup(); process.exit(130); });

async function main() {
  fs.mkdirSync(OUT, { recursive: true });
  if (RECORD) { fs.rmSync(VIDEOS, { recursive: true, force: true }); fs.mkdirSync(VIDEOS, { recursive: true }); }
  pw = loadPlaywright();
  psqlAdmin('DROP DATABASE IF EXISTS prb_matrix WITH (FORCE)');
  psqlAdmin('CREATE DATABASE prb_matrix');
  execFileSync(BIN, ['migrate'], { env: { ...process.env, ...coreEnv }, stdio: 'pipe' });
  const strip = Object.fromEntries(Object.keys(process.env).filter((k) => k.startsWith('PRB_EDGE_')).map((k) => [k, undefined]));
  const spawnEnv = (extra) => { const e = { ...process.env, ...coreEnv, ...extra }; for (const k of Object.keys(strip)) if (!(k in extra)) delete e[k]; return e; };

  await demoServer();
  await kvServer();
  const direct = start('core-direct', BIN, ['serve'], {}, { env: { ...spawnEnv({}), PRB_BIND: `127.0.0.1:${PORT.direct}` } });
  const edge = start('core-edge', BIN, ['serve'], {}, { env: { ...spawnEnv(edgeEnv), PRB_BIND: `127.0.0.1:${PORT.edge}` } });
  const wk = start('pratibimba-worker', BIN, ['worker'], {}, { env: { ...spawnEnv(edgeEnv), PRB_BIND: `127.0.0.1:${PORT.edge + 100}` } });
  for (const [u, k] of [[URLS.direct, direct], [URLS.edge, edge]]) {
    await until(`${k.name} health`, async () => (await fetch(u + '/health')).ok, 30).catch((e) => { throw new Error(`${e.message}\n${k.log.slice(-800)}`); });
  }

  // Asset first (the site file names it), then the site, audiences, and the Worker's first config.
  psql(`INSERT INTO sites (id, origin) VALUES ('${SITE}', '${URLS.demo}') ON CONFLICT DO NOTHING`);
  const up = await fetch(`${URLS.direct}/api/sites/${SITE}/assets`, { method: 'POST', headers: { authorization: `Bearer ${TOKEN}` }, body: png(1400, 560) });
  if (up.status !== 201) throw new Error(`asset upload ${up.status}: ${await up.text()}`);
  assetId = (await up.json()).id;
  if (MODES_SELECTED().some((k) => MODES[k].worker)) {
    try { await startWorker('off', 'off'); } catch (e) { workerReason = `the proxy Worker cannot run locally: ${String(e.message).split('\n')[0].slice(0, 160)}`; }
  }
  await applySite();
  await setupAudiences();
  if (!workerReason && MODES_SELECTED().some((k) => MODES[k].worker)) await settleEdge();

  browser = await pw.chromium.launch();
  const restoreGift = async () => restrict('gift', audienceOf['returning visitors']);

  if (!ONLY_MUTATION) {
    await globalRows();
    for (const k of MODES_SELECTED()) await runMode(k);
  }

  const proofs = [];
  if (PROVE || ONLY_MUTATION) {
    const names = ONLY_MUTATION ? [ONLY_MUTATION] : Object.keys(MUTATIONS);
    for (const name of names) {
      const mu = MUTATIONS[name];
      if (!mu) throw new Error(`unknown mutation ${name}; known: ${Object.keys(MUTATIONS).join(', ')}`);
      const before = results.length;
      if (SDK_MUTATIONS[name]) mutation = name;
      if (name === 'audience') await restrict('gift', null);
      try {
        await runMode(mu.mode, name);
      } finally {
        mutation = null;
        if (name === 'audience') await restoreGift();
      }
      const mutated = results.splice(before);
      const red = mutated.filter((r) => r.status === 'FAIL' && mu.red.test(r.name));
      const skipped = mutated.find((r) => r.status === 'SKIP' && r.name === 'every row');
      proofs.push({ name, mode: mu.mode, breaks: mu.breaks, red: red.map((r) => r.name), detail: red[0]?.detail ?? skipped?.detail ?? '', caught: red.length > 0, skipped: !!skipped });
    }
  }

  await browser.close();
  for (const k of kids) stop(k);
  await sleep(500);
  try { psqlAdmin('DROP DATABASE IF EXISTS prb_matrix WITH (FORCE)'); } catch { /* best effort */ }
  console.log('\n' + table(results));
  const count = (s) => results.filter((r) => r.status === s).length;
  console.log(`\n${count('PASS')} pass, ${count('FAIL')} fail, ${count('SKIP')} skip`);
  if (proofs.length) {
    console.log('\nMutation proof (a row must go red when its category is broken):');
    for (const p of proofs) console.log(`  ${p.skipped ? 'SKIP' : p.caught ? 'RED ' : 'MISS'}  ${p.name.padEnd(9)} [${p.mode}] ${p.breaks}${p.caught ? `\n        -> ${p.red.join('; ')}: ${p.detail}` : p.skipped ? ` (${p.detail})` : ''}`);
  }
  if (RECORD) console.log(`\nvideos: ${fs.readdirSync(VIDEOS).length} files in ${VIDEOS}`);
  const bad = count('FAIL') > 0 || proofs.some((p) => !p.caught && !p.skipped);
  if (ONLY_MUTATION) process.exit(proofs.every((p) => p.caught) ? 1 : 3); // a mutated run is meant to fail
  process.exit(bad ? 1 : 0);
}
function MODES_SELECTED() {
  const want = opt('modes') ?? (ONLY_MUTATION && MUTATIONS[ONLY_MUTATION] ? MUTATIONS[ONLY_MUTATION].mode : null);
  const all = want ? want.split(',') : Object.keys(MODES);
  for (const k of all) if (!MODES[k]) throw new Error(`unknown mode ${k}`);
  return all;
}
main().catch((e) => { console.error(e.stack ?? e); process.exit(2); });
