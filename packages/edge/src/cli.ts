// The sidecar entrypoint: `node dist/cli.js`. Configured by environment only, so the container
// needs no files. Sits in front of your origin and rewrites marked slots as the HTML streams.
//
//   PRB_CORE_URL     Core's base URL (required)
//   PRB_SITE         site name (required)
//   PRB_ORIGIN_URL   where the pages come from (required)
//   PRB_EDGE_PORT    listen port (default 8787)
//   PRB_EDGE_HOST    listen address (default 0.0.0.0)
//   PRB_EDGE_PERSIST 1 = one-year subject cookie (collect consent first)
//
// GET /__prb_edge/health answers 200 without touching Core or the origin.

import { createEdge } from './index.ts';
import { serve } from './node.ts';
import { proxyToOrigin } from './origin.ts';

const env = process.env;
const need = (k: string): string => {
  const v = env[k];
  if (!v) {
    console.error(`pratibimba-edge: ${k} is required`);
    process.exit(2);
  }
  return v;
};

const core = need('PRB_CORE_URL');
const site = need('PRB_SITE');
const originBase = new URL(need('PRB_ORIGIN_URL'));
const port = Number(env.PRB_EDGE_PORT ?? 8787);
const host = env.PRB_EDGE_HOST ?? '0.0.0.0';

const edge = createEdge({
  core,
  site,
  persist: env.PRB_EDGE_PERSIST === '1',
  origin: (req) => proxyToOrigin(req, originBase),
});

serve(async (req) => {
  if (new URL(req.url).pathname === '/__prb_edge/health') return new Response('ok\n');
  return edge.handleRequest(req);
}, port, host);
console.log(`pratibimba-edge: :${port} -> ${originBase.origin}, core ${core}, site ${site}`);
