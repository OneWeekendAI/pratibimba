// Run the edge handler on Node, in front of any origin (for self-hosters without an edge).
//
//   import { createEdge } from '@pratibimba/edge';
//   import { serve } from '@pratibimba/edge/node';
//   const prb = createEdge({ core: 'http://127.0.0.1:8080', site: 'shop',
//     origin: (req) => fetch(new URL(new URL(req.url).pathname + new URL(req.url).search, 'http://127.0.0.1:3000'), req) });
//   serve(prb.handleRequest, 8787);

import { createServer, type IncomingMessage, type Server } from 'node:http';
import { Readable } from 'node:stream';

function toRequest(req: IncomingMessage): Request {
  const url = `http://${req.headers.host ?? 'localhost'}${req.url ?? '/'}`;
  const headers = new Headers();
  for (const [k, v] of Object.entries(req.headers)) if (v !== undefined) headers.set(k, Array.isArray(v) ? v.join(', ') : v);
  const body = req.method === 'GET' || req.method === 'HEAD' ? undefined : (Readable.toWeb(req) as ReadableStream);
  return new Request(url, { method: req.method, headers, body, duplex: 'half' } as RequestInit);
}

export function serve(handler: (r: Request) => Promise<Response>, port: number, host = '127.0.0.1'): Server {
  const server = createServer(async (req, res) => {
    try {
      const r = await handler(toRequest(req));
      const headers: Record<string, string | string[]> = {};
      r.headers.forEach((v, k) => {
        if (k !== 'set-cookie') headers[k] = v;
      });
      const cookies = r.headers.getSetCookie();
      if (cookies.length) headers['set-cookie'] = cookies;
      res.writeHead(r.status, headers);
      if (r.body) for await (const chunk of r.body as unknown as AsyncIterable<Uint8Array>) res.write(chunk);
      res.end();
    } catch (e) {
      res.writeHead(502, { 'content-type': 'text/plain' });
      res.end(`pratibimba edge: ${(e as Error).message}`);
    }
  });
  server.listen(port, host);
  return server;
}
