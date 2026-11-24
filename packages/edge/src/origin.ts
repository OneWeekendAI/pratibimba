// Forward a request to the origin as a proxy would: the browser's Host header is kept, and a
// redirect is handed back untouched (status, Location, every Set-Cookie) rather than followed.
// `fetch` can do neither: it follows redirects by default and always rewrites Host.

import { request as httpRequest, type IncomingHttpHeaders } from 'node:http';
import { request as httpsRequest } from 'node:https';
import { Readable } from 'node:stream';

const HOP = new Set(['connection', 'keep-alive', 'transfer-encoding', 'upgrade', 'proxy-connection', 'te', 'trailer']);

export function proxyToOrigin(req: Request, base: URL): Promise<Response> {
  const u = new URL(req.url);
  const target = new URL(u.pathname + u.search, base);
  const headers: Record<string, string> = {};
  req.headers.forEach((v, k) => {
    if (!HOP.has(k)) headers[k] = v;
  });
  headers.host = req.headers.get('host') ?? u.host;
  // The rewriter reads the HTML as text, so the origin must not compress it.
  headers['accept-encoding'] = 'identity';
  const send = target.protocol === 'https:' ? httpsRequest : httpRequest;
  return new Promise((resolve, reject) => {
    const out = send(target, { method: req.method, headers }, (res) => {
      const h = new Headers();
      for (const [k, v] of Object.entries(res.headers as IncomingHttpHeaders)) {
        if (v === undefined || HOP.has(k)) continue;
        for (const one of Array.isArray(v) ? v : [v]) h.append(k, one);
      }
      const status = res.statusCode ?? 502;
      const noBody = status === 204 || status === 205 || status === 304 || req.method === 'HEAD';
      if (noBody) res.resume();
      resolve(new Response(noBody ? null : (Readable.toWeb(res) as unknown as ReadableStream), { status, statusText: res.statusMessage, headers: h }));
    });
    out.on('error', reject);
    if (req.body) Readable.fromWeb(req.body as never).pipe(out);
    else out.end();
  });
}
