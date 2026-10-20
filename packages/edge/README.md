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
