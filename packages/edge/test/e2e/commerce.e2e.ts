// F1 end to end through @pratibimba/edge and a real Core with its live worker: the edge renders
// a decided hero, the beacon's exposure lands, and a purchase posted from edge code for the
// cookie's visitor is stored once however often it is sent and credits that decision's revenue.
//
//   cargo build && (cd packages/edge && npm run e2e)

import { test, after, before } from 'node:test';
import assert from 'node:assert/strict';
import { createEdge } from '../../src/index.ts';
import { heroSite, startCore, type Core } from './core.ts';

const PAGE = '<html><body><!-- prb:slot="hero" --><h1>Espresso</h1><!-- /prb:slot --></body></html>';
let core: Core;
before(async () => {
  const site = { ...heroSite('commerce'), currency: 'INR', privacy_mode: 'full' };
  site.slots[0].attribution_window_secs = 60;
  core = await startCore('prb_edge_commerce_e2e', 18097, [site]);
});
after(() => core?.stop());

test('edge_purchase_is_stored_once_and_credits_the_decision_revenue', async () => {
  const edge = createEdge({ core: core.url, site: 'commerce', timeoutMs: 2000, origin: async () => new Response(PAGE, { headers: { 'content-type': 'text/html' } }) });
  const page = await edge.handleRequest(new Request('https://shop.test/'));
  const html = await page.text();
  const m = /data-prb='([^']*)'/.exec(html);
  assert.ok(m, html);
  const cfg = JSON.parse(m[1].replace(/&#39;/g, "'").replace(/&lt;/g, '<').replace(/&amp;/g, '&'));
  const d = cfg.decisions[0];
  const exposure = { site: 'commerce', subject: cfg.subject, events: [{ id: crypto.randomUUID(), name: 'exposure', slot: d.slot, decision_id: d.id }] };
  const e = await edge.handleRequest(new Request('https://shop.test/_prb/e', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(exposure) }));
  assert.equal(e.status, 202, await e.text());
  const checkout = () => new Request('https://shop.test/checkout', { method: 'POST', headers: { cookie: `_prb_subject=${cfg.subject}` } });
  const order = { order_id: `ord-${Date.now()}`, value: 998.5, currency: 'inr' };
  assert.equal(await edge.purchase(checkout(), order), true);
  assert.equal(await edge.purchase(checkout(), order), true);
  assert.equal(core.sql(`SELECT count(*) FROM events WHERE order_id = '${order.order_id}'`), '1');
  for (let i = 0; i < 50 && core.sql(`SELECT coalesce(revenue::text, '') FROM decisions WHERE id = '${d.id}'`) === ''; i++) {
    await new Promise((r) => setTimeout(r, 100));
  }
  assert.equal(core.sql(`SELECT revenue::float8 FROM decisions WHERE id = '${d.id}'`), '998.5');
});
