// F1: window.prb.addToCart(item) and window.prb.purchase(order) post first-class commerce events.
// Core derives a purchase's event id from its order_id, so sending one order twice stores it once;
// the SDK's job is to send the order's fields exactly and refuse a malformed order loudly.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { load, sleep } from './harness.mjs';

test('purchase_posts_order_value_currency_and_items', async () => {
  const p = load();
  await sleep(10);
  p.w.prb.purchase({ order_id: 'ord-1', value: 129.5, currency: 'inr', items: [{ id: 'sku-1', price: 64.75, quantity: 2 }] });
  await sleep(5);
  const ev = p.events().filter((e) => e.name === 'purchase');
  assert.equal(ev.length, 1);
  assert.equal(ev[0].order_id, 'ord-1');
  assert.equal(ev[0].value, 129.5);
  assert.equal(ev[0].currency, 'INR');
  assert.deepEqual(JSON.parse(JSON.stringify(ev[0].items)), [{ id: 'sku-1', price: 64.75, quantity: 2 }]);
  assert.equal(ev[0].decision_id, undefined, 'a purchase names no decision: Core attributes it');
});

test('purchase_sends_the_same_order_id_every_time', async () => {
  // A reloaded thank-you page calls purchase again; Core keys the row on order_id.
  const p = load();
  await sleep(10);
  p.w.prb.purchase({ order_id: 'ord-9', value: 10, currency: 'USD' });
  p.w.prb.purchase({ order_id: 'ord-9', value: 10, currency: 'USD' });
  await sleep(5);
  const ev = p.events().filter((e) => e.name === 'purchase');
  assert.deepEqual(ev.map((e) => e.order_id), ['ord-9', 'ord-9']);
});

test('add_to_cart_posts_the_item', async () => {
  const p = load();
  await sleep(10);
  p.w.prb.addToCart({ id: 'sku-2', price: 499, quantity: 1 });
  await sleep(5);
  const ev = p.events().filter((e) => e.name === 'add_to_cart');
  assert.equal(ev.length, 1);
  assert.deepEqual(JSON.parse(JSON.stringify(ev[0].items)), [{ id: 'sku-2', price: 499, quantity: 1 }]);
  assert.equal(ev[0].value, undefined, 'an item price is not revenue');
});

test('malformed_orders_throw_and_send_nothing', async () => {
  const p = load();
  await sleep(10);
  const bad = [
    {},
    { value: 10, currency: 'USD' },
    { order_id: 'o', value: -1, currency: 'USD' },
    { order_id: 'o', value: 'ten', currency: 'USD' },
    { order_id: 'o', value: 10 },
    { order_id: 'o', value: 10, currency: 'DOLLARS' },
    { order_id: 'o', value: 10, currency: 'USD', items: 'sku' },
  ];
  for (const o of bad) assert.throws(() => p.w.prb.purchase(o), p.w.TypeError, JSON.stringify(o));
  assert.throws(() => p.w.prb.addToCart({ price: 1 }), p.w.TypeError);
  await sleep(5);
  assert.equal(p.events().filter((e) => e.name === 'purchase' || e.name === 'add_to_cart').length, 0);
});
