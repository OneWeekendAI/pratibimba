import { test } from 'node:test';
import { AsyncLocalStorage } from 'node:async_hooks';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createSlot } from '../src/slot.ts';

type Call = { url: string; body: any };

function core(answer: (slots: string[]) => any, calls: Call[] = []) {
  return (async (url: string, init: any) => {
    const body = JSON.parse(init.body);
    calls.push({ url, body });
    const out = answer(body.slots);
    if (out instanceof Error) throw out;
    return new Response(JSON.stringify(out), { status: out.status ?? 200, headers: { 'content-type': 'application/json' } });
  }) as unknown as typeof fetch;
}

function decision(slot: string, variant: string, payload: string | null = null) {
  return { decision_id: `d-${slot}`, slot, variant, payload, explanation: {} };
}

const visitorHeaders = async () => new Headers({ 'x-prb-subject': 'v1', 'x-prb-ctx': 'r1m0a0' });

function slotWith(fetch: typeof globalThis.fetch, scope = {}) {
  return createSlot({ core: 'http://core', site: 'shop', fetch, headers: visitorHeaders, requestScope: () => scope }).Slot;
}

async function html(el: Promise<any>) {
  return renderToStaticMarkup(await el);
}

const baseline = createElement('h1', null, 'Our espresso');

test('slot_renders_the_component_for_its_variant', async () => {
  const Slot = slotWith(core((s) => ({ decisions: s.map((k) => decision(k, 'gift')) })));
  const out = await html(Slot({ id: 'hero', variants: { gift: createElement('h1', null, 'Gift sets') }, children: baseline }));
  assert.match(out, /Gift sets/);
  assert.doesNotMatch(out, /Our espresso/);
  assert.match(out, /data-prb-variant="gift"/);
});

test('slot_renders_the_payload_when_no_component', async () => {
  const Slot = slotWith(core((s) => ({ decisions: s.map((k) => decision(k, 'ten_off', '<p>10% off today</p>')) })));
  const out = await html(Slot({ id: 'hero', children: baseline }));
  assert.match(out, /<p>10% off today<\/p>/);
});

test('slot_renders_children_for_the_baseline', async () => {
  const Slot = slotWith(core((s) => ({ decisions: s.map((k) => decision(k, 'baseline', null)) })));
  const out = await html(Slot({ id: 'hero', variants: { gift: createElement('h1', null, 'Gift sets') }, children: baseline }));
  assert.match(out, /Our espresso/);
  assert.match(out, /data-prb-decision="d-hero"/);
});

test('slot_keeps_the_baseline_when_core_is_down', async () => {
  const Slot = slotWith(core(() => new Error('ECONNREFUSED')));
  const out = await html(Slot({ id: 'hero', children: baseline }));
  assert.match(out, /Our espresso/);
  assert.doesNotMatch(out, /data-prb-decision/);
});

test('slot_keeps_the_baseline_on_a_core_error_status', async () => {
  const warnings: string[] = [];
  const warn = console.warn;
  console.warn = (m: string) => void warnings.push(String(m));
  try {
    const Slot = slotWith(core(() => ({ status: 500, error: 'boom' })));
    assert.match(await html(Slot({ id: 'hero', children: baseline })), /Our espresso/);
  } finally {
    console.warn = warn;
  }
  // The operator needs to see that Core answered 500, not a TypeError about a missing field.
  assert.ok(warnings.some((w) => /decide failed: 500/.test(w)), JSON.stringify(warnings));
});

test('slot_keeps_the_baseline_for_an_unknown_slot', async () => {
  const Slot = slotWith(core((s) => ({ decisions: s.map((k) => ({ slot: k, error: 'unknown slot' })) })));
  assert.match(await html(Slot({ id: 'nope', children: baseline })), /Our espresso/);
});

test('slots_in_one_request_share_one_call', async () => {
  const calls: Call[] = [];
  const Slot = slotWith(core((s) => ({ decisions: s.map((k) => decision(k, 'baseline')) }), calls));
  await Promise.all([html(Slot({ id: 'hero', children: baseline })), html(Slot({ id: 'promo', children: baseline }))]);
  assert.equal(calls.length, 1, `${calls.length} calls`);
  assert.deepEqual(calls[0].body.slots, ['hero', 'promo']);
  assert.deepEqual(calls[0].body.ctx, { returning: true, mobile: false, from_ad: false });
  assert.equal(calls[0].body.subject, 'v1');
});

test('slots_in_different_requests_do_not_share', async () => {
  // One Slot, two concurrent requests, each with its own scope (as React's cache gives).
  const calls: Call[] = [];
  const als = new AsyncLocalStorage<object>();
  const Slot = createSlot({
    core: 'http://core', site: 'shop', headers: visitorHeaders, requestScope: () => als.getStore()!,
    fetch: core((s) => ({ decisions: s.map((k) => decision(k, 'baseline')) }), calls),
  }).Slot;
  await Promise.all([als.run({}, () => html(Slot({ id: 'hero' }))), als.run({}, () => html(Slot({ id: 'hero' })))]);
  assert.equal(calls.length, 2, 'two visitors were decided in one call');
});

test('slot_without_a_proxy_renders_the_baseline_and_calls_nothing', async () => {
  const calls: Call[] = [];
  const Slot = createSlot({ core: 'http://core', site: 'shop', fetch: core(() => ({ decisions: [] }), calls), headers: async () => new Headers(), requestScope: () => ({}) }).Slot;
  assert.match(await html(Slot({ id: 'hero', children: baseline })), /Our espresso/);
  assert.equal(calls.length, 0);
});

test('slot_carries_no_hide_rule', async () => {
  // Flicker-free means the server never asks the browser to hide anything.
  const Slot = slotWith(core((s) => ({ decisions: s.map((k) => decision(k, 'gift', '<p>g</p>')) })));
  const out = await html(Slot({ id: 'hero', children: baseline }));
  assert.doesNotMatch(out, /visibility|prb-hide/);
});
