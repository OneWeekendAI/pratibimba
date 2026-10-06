// The generic edge worker on Node, in front of a static origin. On Cloudflare the same
// handler is `export default { fetch: (req) => prb.handleRequest(req) }`.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { createEdge } from '@pratibimba/edge';
import { serve } from '@pratibimba/edge/node';

const CORE = process.env.PRB_CORE_URL ?? 'http://127.0.0.1:8080';
const PORT = Number(process.env.PORT ?? 3200);
const ORIGIN_PORT = PORT + 1;

// The origin: any server that renders HTML with slot markers. Here, one static file.
createServer(async (req, res) => {
  if (req.url === '/' || req.url?.startsWith('/?')) {
    res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'public, max-age=300' });
    res.end(await readFile(new URL('./origin/index.html', import.meta.url)));
  } else {
    res.writeHead(404).end();
  }
}).listen(ORIGIN_PORT, '127.0.0.1');

const prb = createEdge({
  core: CORE,
  site: 'edge-demo',
  origin: (req) => {
    const u = new URL(req.url);
    return fetch(`http://127.0.0.1:${ORIGIN_PORT}${u.pathname}${u.search}`, { headers: req.headers });
  },
});
serve(prb.handleRequest, PORT);
console.log(`edge on http://localhost:${PORT} -> origin :${ORIGIN_PORT}, core ${CORE}`);
