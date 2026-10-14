// F1: prb.addToCart(item) and prb.purchase(order) from server code post first-class commerce
// events for this request's visitor. Core keys a purchase on its order_id; the SDK sends the
// order's fields exactly and refuses a malformed order before any request.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createPratibimba } from '../src/index.ts';

type Call = { url: string; body: any };

function setup(consent = '1') {
  const calls: Call[] = [];
  const fetch = (async (url: string, init: any) => {
    calls.push({ url, body: JSON.parse(init.body) });
    return new Response('{"accepted":1}', { status: 202, headers: { 'content-type': 'application/json' } });
  }) as unknown as typeof globalThis.fetch;
  const headers = async () => new Headers({ 'x-prb-subject': 'v1', 'x-prb-ctx': 'r1m0a0', 'x-prb-consent': consent });
  const prb = createPratibimba({ core: 'http://core', site: 'shop', fetch, headers, requestScope: () => ({}) });
  return { prb, calls };
}

test('purchase_posts_the_order_for_this_visitor', async () => {
  const { prb, calls } = setup();
  await prb.purchase({ order_id: 'ord-1', value: 129.5, currency: 'inr', items: [{ id: 'sku-1', price: 64.75, quantity: 2 }] });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, 'http://core/e');
  assert.equal(calls[0].body.subject, 'v1');
  assert.equal(calls[0].body.consent, true);
  const [ev] = calls[0].body.events;
  assert.equal(ev.name, 'purchase');
  assert.deepEqual([ev.order_id, ev.value, ev.currency], ['ord-1', 129.5, 'INR']);
  assert.deepEqual(ev.items, [{ id: 'sku-1', price: 64.75, quantity: 2 }]);
  assert.match(ev.id, /^[0-9a-f-]{36}$/);
});

test('add_to_cart_posts_the_item_and_no_value', async () => {
  const { prb, calls } = setup();
  await prb.addToCart({ id: 'sku-2', price: 499 });
  const [ev] = calls[0].body.events;
  assert.equal(ev.name, 'add_to_cart');
  assert.deepEqual(ev.items, [{ id: 'sku-2', price: 499 }]);
  assert.equal(ev.value, undefined);
});

test('malformed_orders_reject_and_send_nothing', async () => {
  const { prb, calls } = setup();
  const bad: any[] = [
    {},
    { value: 10, currency: 'USD' },
    { order_id: 'o', value: -1, currency: 'USD' },
    { order_id: 'o', value: Number.NaN, currency: 'USD' },
    { order_id: 'o', value: 10 },
    { order_id: 'o', value: 10, currency: 'DOLLARS' },
    { order_id: 'o', value: 10, currency: 'USD', items: [{ price: 1 }] },
  ];
  for (const o of bad) await assert.rejects(prb.purchase(o), TypeError, JSON.stringify(o));
  await assert.rejects(prb.addToCart({ price: 1 } as any), TypeError);
  assert.equal(calls.length, 0);
});
