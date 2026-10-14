// F1: prb.track / addToCart / purchase(request, ...) post first-class commerce events from edge
// code for the visitor the request's subject cookie names. Core keys a purchase on its order_id;
// the SDK sends the order's fields exactly and refuses a malformed order before any request.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createEdge } from '../src/index.ts';

function setup() {
  const calls: { url: string; body: any }[] = [];
  const fetch = (async (url: string, init: any) => {
    calls.push({ url, body: JSON.parse(init.body) });
    return new Response('{"accepted":1}', { status: 202 });
  }) as unknown as typeof globalThis.fetch;
  return { prb: createEdge({ core: 'http://core:8080/', site: 'shop', fetch }), calls };
}

const req = (cookie?: string) => new Request('https://shop.test/checkout', { method: 'POST', headers: cookie ? { cookie } : {} });

test('purchase_posts_the_order_for_the_cookie_subject', async () => {
  const { prb, calls } = setup();
  const sent = await prb.purchase(req('_prb_subject=v1; _prb_consent=granted'), { order_id: 'ord-1', value: 80, currency: 'usd', items: [{ id: 'sku-1' }] });
  assert.equal(sent, true);
  assert.equal(calls[0].url, 'http://core:8080/e');
  assert.equal(calls[0].body.subject, 'v1');
  assert.equal(calls[0].body.consent, true);
  const [ev] = calls[0].body.events;
  assert.deepEqual([ev.name, ev.order_id, ev.value, ev.currency], ['purchase', 'ord-1', 80, 'USD']);
  assert.deepEqual(ev.items, [{ id: 'sku-1' }]);
});

test('add_to_cart_and_track_post_for_the_cookie_subject', async () => {
  const { prb, calls } = setup();
  await prb.addToCart(req('_prb_subject=v1'), { id: 'sku-2', quantity: 2 });
  await prb.track(req('_prb_subject=v1'), 'signup');
  assert.deepEqual(calls.map((c) => c.body.events[0].name), ['add_to_cart', 'signup']);
  assert.deepEqual(calls[0].body.events[0].items, [{ id: 'sku-2', quantity: 2 }]);
  assert.equal(calls[1].body.consent, false);
});

test('no_subject_cookie_sends_nothing', async () => {
  const { prb, calls } = setup();
  assert.equal(await prb.purchase(req(), { order_id: 'o', value: 1, currency: 'USD' }), false);
  assert.equal(calls.length, 0);
});

test('malformed_orders_reject_and_send_nothing', async () => {
  const { prb, calls } = setup();
  for (const o of [{}, { order_id: 'o', value: -1, currency: 'USD' }, { order_id: 'o', value: 1, currency: 'US' }, { order_id: 'o', value: 1 }] as any[]) {
    await assert.rejects(prb.purchase(req('_prb_subject=v1'), o), TypeError, JSON.stringify(o));
  }
  await assert.rejects(prb.addToCart(req('_prb_subject=v1'), {} as any), TypeError);
  assert.equal(calls.length, 0);
});
