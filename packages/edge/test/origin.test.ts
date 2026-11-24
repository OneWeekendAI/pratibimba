// The sidecar is a proxy: an origin redirect reaches the browser as the origin sent it, and the
// origin sees the browser's Host.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer, request, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import { createEdge } from '../src/index.ts';
import { serve } from '../src/node.ts';
import { proxyToOrigin } from '../src/origin.ts';

const listen = (s: Server) => new Promise<number>((r) => s.listen(0, '127.0.0.1', () => r((s.address() as AddressInfo).port)));
const get = (port: number, path: string, host: string) =>
  new Promise<{ status: number; headers: Record<string, unknown>; body: string }>((resolve, reject) => {
    const q = request({ host: '127.0.0.1', port, path, headers: { host } }, (res) => {
      let body = '';
      res.on('data', (c) => (body += c));
      res.on('end', () => resolve({ status: res.statusCode!, headers: res.headers, body }));
    });
    q.on('error', reject);
    q.end();
  });

test('origin_redirect_and_host_pass_through_the_sidecar', async () => {
  const seen: string[] = [];
  const origin = createServer((q, r) => {
    seen.push(`${q.url} host=${q.headers.host}`);
    if (q.url === '/login') {
      r.writeHead(302, { location: '/home', 'set-cookie': ['sid=1; Path=/', 'b=2'] });
      return r.end();
    }
    r.writeHead(200, { 'content-type': 'text/plain' });
    r.end('home');
  });
  const op = await listen(origin);
  const base = new URL(`http://127.0.0.1:${op}`);
  const edge = createEdge({ core: 'http://127.0.0.1:1', site: 's', origin: (req) => proxyToOrigin(req, base) });
  const sidecar = serve(edge.handleRequest, 0);
  await new Promise((r) => sidecar.once('listening', r));
  const sp = (sidecar.address() as AddressInfo).port;
  try {
    const res = await get(sp, '/login', 'shop.example.com');
    assert.equal(res.status, 302);
    assert.equal(res.headers.location, '/home');
    assert.deepEqual(res.headers['set-cookie'], ['sid=1; Path=/', 'b=2']);
    assert.deepEqual(seen, ['/login host=shop.example.com']);
  } finally {
    sidecar.close();
    origin.close();
  }
});
