# @pratibimba/edge

Pratibimba at the edge: rewrites marked slots in your origin's HTML as it streams, before the browser sees a byte. One handler for Cloudflare Workers, Deno Deploy and Vercel Edge, plus a Node server in `@pratibimba/edge/node`.

```sh
npm i @pratibimba/edge
```

```js
import { createEdge } from '@pratibimba/edge';
const prb = createEdge({ core: 'https://core.internal', site: 'demo' });
export default { fetch: (req) => prb.handleRequest(req) };
// in the origin's HTML:  <!-- prb:slot="hero" --> <h1>Default headline</h1> <!-- /prb:slot -->
```

Needs a running Pratibimba Core. Slot markers, copy and sorted slots, and the rest: see the [main README](https://github.com/OneWeekendAI/pratibimba#readme).

MIT licensed.

## Sidecar image

For a site with no edge platform, run the edge as a container in front of your origin. It is a
small Node image, runs as a non-root user and has no runtime dependencies.

```sh
docker build -t pratibimba-edge packages/edge
docker run -p 8787:8787 \
  -e PRB_CORE_URL=http://core:8080 -e PRB_SITE=shop -e PRB_ORIGIN_URL=http://app:3000 \
  pratibimba-edge
```

Point your reverse proxy or DNS at port 8787. `PRB_EDGE_PORT` changes the port and
`PRB_EDGE_PERSIST=1` sets the one-year subject cookie (collect consent first). The container
exits at once if a required variable is missing. `GET /__prb_edge/health` answers 200 without
touching Core or the origin; the image's `HEALTHCHECK` uses it.

`node ci/edge-image.mjs` builds the image and checks it against a real Core and origin on your
machine. It pushes nothing.
