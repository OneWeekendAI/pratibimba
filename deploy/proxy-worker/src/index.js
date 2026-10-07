// Proxies every path on the public API hostname to Core on Cloud Run (asia-northeast3, which
// has no Cloud Run domain mappings). Method, query, headers and body pass through; request and
// response bodies stream, so SSE (/api/sites/*/stream) and /mcp work. Nothing is cached except
// /sdk.js, which keeps whatever Cache-Control Core sends.
//
// Rollback: delete this Worker's route on the hostname. DNS (proxied CNAME to
// ghs.googlehosted.com) and the Taiwan domain mapping are never touched, so traffic falls back
// to the asia-east1 service at once: deploy/proxy-worker/switch.sh off.
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

    const hasBody = !['GET', 'HEAD'].includes(request.method);
    const init = { method: request.method, headers, body: hasBody ? request.body : null, redirect: 'manual' };
    if (inUrl.pathname !== '/sdk.js') init.cache = 'no-store';
    const upstream = await fetch(url, init);

    const out = new Headers(upstream.headers);
    const loc = out.get('location');
    if (loc) {
      try {
        const l = new URL(loc, origin);
        if (l.host === origin.host) { l.host = inUrl.host; l.protocol = inUrl.protocol; out.set('location', l.toString()); }
      } catch {}
    }
    return new Response(upstream.body, { status: upstream.status, statusText: upstream.statusText, headers: out });
  },
};
