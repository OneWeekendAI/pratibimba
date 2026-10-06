// Loads sdk.js into a jsdom page the way a real install does: the manifest script, then the
// SDK, both in <head>, before the body is parsed. `fetch` is a recording mock.
import { JSDOM } from 'jsdom';
import { readFileSync } from 'node:fs';

const SDK = readFileSync(new URL('../sdk.js', import.meta.url), 'utf8');

export const MANIFEST = {
  site: 'demo',
  slots: [{ key: 'hero', selector: '#hero', metric: 'click', ttl_secs: 60, variants: [] }],
};

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export function ok(body) {
  return Promise.resolve({ ok: true, status: 200, json: () => Promise.resolve(body) });
}

export const DECISION = { decision_id: 'd-1', slot: 'hero', variant: 'gift', payload: '<h1>Gift sets</h1>', explanation: {} };

/**
 * opts.decide: (body) => Promise<response>   (default: resolves DECISION)
 * opts.attrs: extra attributes on the SDK tag
 * opts.url: page URL
 * opts.before: (window) => void, runs before parsing (seed storage, cookies)
 */
export function load(opts = {}) {
  const calls = [];
  const attrs = Object.assign({ 'data-site': 'demo', 'data-base': 'https://core.test/', 'data-timeout': '60' }, opts.attrs || {});
  const attrText = Object.entries(attrs).map(([k, v]) => `${k}="${v}"`).join(' ');
  const html = `<!doctype html><html><head>
<script>window.__prb_manifest = ${JSON.stringify(opts.manifest || MANIFEST)};</script>
<script ${attrText}>${SDK}</script>
<script>window.__hiddenAtParse = !!document.getElementById('prb-hide');</script>
</head><body><nav id="nav">Nav</nav><div id="hero"><a id="cta">Original</a></div><div id="promo">Promo</div></body></html>`;
  const dom = new JSDOM(html, {
    url: opts.url || 'https://shop.test/',
    runScripts: 'dangerously',
    beforeParse(window) {
      window.fetch = (url, init) => {
        const body = JSON.parse(init.body);
        calls.push({ url, body });
        if (url.endsWith('/decide')) return (opts.decide || (() => ok(DECISION)))(body);
        return ok({ accepted: 1 });
      };
      if (opts.before) opts.before(window);
    },
  });
  const w = dom.window;
  const events = () => calls.filter((c) => c.url.endsWith('/e')).flatMap((c) => c.body.events);
  const decides = () => calls.filter((c) => c.url.endsWith('/decide'));
  const visibility = (id) => w.getComputedStyle(w.document.getElementById(id)).visibility;
  return { w, calls, events, decides, visibility, hero: () => w.document.getElementById('hero') };
}
