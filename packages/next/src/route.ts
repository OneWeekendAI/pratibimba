// `app/_prb/[...path]/route.ts`: the site's own first-party door to Core. Only the public
// surface goes through; the dashboard API (/api) never does, and cookies are not forwarded.

import type { Fetch } from './core.ts';

const ALLOWED: { method: string; pattern: RegExp }[] = [
  { method: 'GET', pattern: /^manifest\/[a-z0-9_-]{1,64}(\.js)?$/ },
  { method: 'GET', pattern: /^sdk\.js$/ },
  { method: 'POST', pattern: /^e$/ },
  { method: 'POST', pattern: /^decide$/ },
  { method: 'POST', pattern: /^decide\/batch$/ },
];

const FORWARDED_RESPONSE_HEADERS = ['content-type', 'cache-control'];

export function createRoute(opts: { core: string; fetch?: Fetch }) {
  async function handle(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const at = url.pathname.indexOf('/_prb/');
    const path = at >= 0 ? url.pathname.slice(at + '/_prb/'.length) : '';
    if (!ALLOWED.some((a) => a.method === request.method && a.pattern.test(path))) {
      return new Response(JSON.stringify({ error: 'not found' }), { status: 404, headers: { 'content-type': 'application/json' } });
    }
    const f = opts.fetch ?? fetch;
    const upstream = await f(`${opts.core.replace(/\/$/, '')}/${path}${url.search}`, {
      method: request.method,
      headers: request.method === 'POST' ? { 'content-type': 'application/json' } : {},
      body: request.method === 'POST' ? await request.text() : undefined,
      cache: 'no-store',
    });
    const headers = new Headers();
    for (const h of FORWARDED_RESPONSE_HEADERS) {
      const v = upstream.headers.get(h);
      if (v) headers.set(h, v);
    }
    return new Response(await upstream.arrayBuffer(), { status: upstream.status, headers });
  }
  return { GET: handle, POST: handle };
}
