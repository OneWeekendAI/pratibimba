// Proxies every path on the public API hostname to Core on Cloud Run (asia-northeast3, which
// has no Cloud Run domain mappings). Method, query, headers and body pass through; request and
// response bodies stream, so SSE (/api/sites/*/stream) and /mcp work. Nothing is cached except
// /sdk.js, which keeps whatever Cache-Control Core sends.
//
// EC1 (edge-cache.md): on /decide and /decide/batch the Worker also reads the site's edge
// snapshot from KV (binding SNAPSHOTS, key `site/<id>`, written by Core) and reports how old it
// is, as response headers and one log line. It decides nothing yet: every request still goes to
// Core, and the KV read runs alongside the forward, capped at SNAPSHOT_WAIT_MS, so it never
// holds a response longer than that. 150 ms, not 50: cold KV reads (first at a PoP) measured
// above 50 ms live, and the forward to Seoul takes longer than 150 ms from most PoPs anyway, so
// the cap rarely adds time.
//
// Rollback: delete this Worker's route on the hostname. DNS (proxied CNAME to
// ghs.googlehosted.com) and the Taiwan domain mapping are never touched, so traffic falls back
// to the asia-east1 service at once: deploy/proxy-worker/switch.sh off.

export const SNAPSHOT_WAIT_MS = 150;
const DECIDE_PATHS = new Set(['/decide', '/decide/batch']);

/// The site's snapshot as this PoP sees it: {version, age_ms} or {missing|error|timeout}.
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

export default {
  async fetch(request, env) {
    const inUrl = new URL(request.url);
    const origin = new URL(env.ORIGIN);
    const url = new URL(inUrl.pathname + inUrl.search, origin);

    const headers = new Headers(request.headers);
    headers.delete('host');
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

    const decide = request.method === 'POST' && DECIDE_PATHS.has(inUrl.pathname);
    // A clone reads the site; the original body streams to Core untouched.
    const snapshot = decide ? siteOf(request.clone()).then((site) => snapshotAge(env, site).then((s) => ({ site, ...s }))) : null;

    const hasBody = !['GET', 'HEAD'].includes(request.method);
    const init = { method: request.method, headers, body: hasBody ? request.body : null, redirect: 'manual' };
    if (inUrl.pathname !== '/sdk.js') init.cache = 'no-store';
    const [upstream, snap] = await Promise.all([fetch(url, init), snapshot]);

    const out = new Headers(upstream.headers);
    const loc = out.get('location');
    if (loc) {
      try {
        const l = new URL(loc, origin);
        if (l.host === origin.host) { l.host = inUrl.host; l.protocol = inUrl.protocol; out.set('location', l.toString()); }
      } catch {}
    }
    if (snap) {
      out.set('x-prb-snapshot', snap.state);
      if (snap.state === 'ok') {
        out.set('x-prb-snapshot-version', String(snap.version));
        out.set('x-prb-snapshot-age-ms', String(snap.age_ms));
      }
      console.log(JSON.stringify({ msg: 'edge snapshot', path: inUrl.pathname, site: snap.site, state: snap.state, version: snap.version, age_ms: snap.age_ms, colo: request.cf?.colo }));
    }
    return new Response(upstream.body, { status: upstream.status, statusText: upstream.statusText, headers: out });
  },
};
