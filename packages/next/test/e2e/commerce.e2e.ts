// F1 end to end through @pratibimba/next and a real Core with its live worker: a server-side
// purchase for the visitor of a decided, exposed hero is stored once however often it is sent,
// and its value lands on that decision as revenue in the site's currency.
//
//   cargo build && (cd packages/next && npm run e2e)

import { test, after, before } from 'node:test';
import assert from 'node:assert/strict';
import { NextRequest } from 'next/server.js';
import { createPratibimba } from '../../src/index.ts';
import { heroSite, startCore, type Core } from './core.ts';

let core: Core;
before(async () => {
  const site = { ...heroSite('commerce'), currency: 'INR', privacy_mode: 'full' };
  site.slots[0].attribution_window_secs = 60;
  core = await startCore('prb_next_commerce_e2e', 18096, [site]);
});
after(() => core?.stop());

test('next_purchase_is_stored_once_and_credits_the_decision_revenue', async () => {
  const forwarded = new Headers();
  const prb = createPratibimba({ core: core.url, site: 'commerce', timeoutMs: 2000, headers: async () => forwarded, requestScope: () => ({}) });
  const res = prb.proxy(new NextRequest('https://shop.test/', { headers: { 'user-agent': 'Mozilla/5.0' } }));
  res.headers.forEach((v, k) => {
    if (k.startsWith('x-middleware-request-')) forwarded.set(k.slice('x-middleware-request-'.length), v);
  });
  const d = await prb.decide('hero');
  assert.ok(d, 'no decision');
  await prb.track('exposure', { slot: 'hero', decisionId: d.decision_id });
  await prb.addToCart({ id: 'sku-1', price: 499, quantity: 1 });
  const order = { order_id: `ord-${Date.now()}`, value: 998.5, currency: 'inr', items: [{ id: 'sku-1', price: 499.25, quantity: 2 }] };
  await prb.purchase(order);
  await prb.purchase(order); // the thank-you page, reloaded
  assert.equal(core.sql(`SELECT count(*) FROM events WHERE order_id = '${order.order_id}'`), '1');
  for (let i = 0; i < 50 && core.sql(`SELECT coalesce(revenue::text, '') FROM decisions WHERE id = '${d.decision_id}'`) === ''; i++) {
    await new Promise((r) => setTimeout(r, 100));
  }
  assert.equal(core.sql(`SELECT revenue::float8 FROM decisions WHERE id = '${d.decision_id}'`), '998.5');
  assert.equal(core.sql(`SELECT currency FROM events WHERE order_id = '${order.order_id}'`), 'INR');
});
