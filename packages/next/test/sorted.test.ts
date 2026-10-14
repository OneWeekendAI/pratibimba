// F2b: <prb.Sorted> renders its keyed children in the decided order, each wrapped with
// data-prb-item, and its beacon reports the rendered order on exposure and the card on a click.
import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'https://shop.test/' });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
Object.defineProperty(globalThis, 'navigator', { value: dom.window.navigator, configurable: true });

const { createElement, act } = await import('react');
const { renderToStaticMarkup } = await import('react-dom/server');
const { createRoot } = await import('react-dom/client');
const { createSlot } = await import('../src/slot.ts');
const { PrbBeacon, __resetBeacon } = await import('../src/beacon.ts');

const visitorHeaders = async () => new Headers({ 'x-prb-subject': 'v1', 'x-prb-ctx': 'r1m0a0' });

function sortedWith(answer: () => any) {
  const fetch = (async () => {
    const out = answer();
    if (out instanceof Error) throw out;
    return new Response(JSON.stringify(out), { status: 200, headers: { 'content-type': 'application/json' } });
  }) as unknown as typeof globalThis.fetch;
  return createSlot({ core: 'http://core', site: 'shop', fetch, headers: visitorHeaders, requestScope: () => ({}) }).Sorted;
}

const cards = () => ['a', 'b', 'c', 'd'].map((k) => createElement('article', { key: k }, `card-${k}`));
const order = (html: string) => [...html.matchAll(/card-([a-d])/g)].map((m) => m[1]);
const grid = (o: string[] | undefined) => ({
  decisions: [{ decision_id: 'd-grid', slot: 'grid', variant: o ? o[0] : 'a', payload: null, ...(o ? { order: o } : {}), explanation: {} }],
});

test('sorted_renders_children_in_the_decided_order', async () => {
  const Sorted = sortedWith(() => grid(['c', 'a', 'd']));
  const html = renderToStaticMarkup(await Sorted({ id: 'grid', children: cards() }));
  // b is not in the order (gated): it follows the ordered items.
  assert.deepEqual(order(html), ['c', 'a', 'd', 'b']);
  assert.match(html, /data-prb-item="c"/);
  assert.match(html, /data-prb-decision="d-grid"/);
});

test('sorted_keeps_the_page_order_when_core_is_down', async () => {
  const Sorted = sortedWith(() => new Error('ECONNREFUSED'));
  const html = renderToStaticMarkup(await Sorted({ id: 'grid', children: cards() }));
  assert.deepEqual(order(html), ['a', 'b', 'c', 'd']);
  assert.doesNotMatch(html, /data-prb-decision/);
});

test('sorted_keeps_the_page_order_for_a_decision_with_no_order', async () => {
  const Sorted = sortedWith(() => grid(undefined));
  const html = renderToStaticMarkup(await Sorted({ id: 'grid', children: cards() }));
  assert.deepEqual(order(html), ['a', 'b', 'c', 'd']);
});

let posts: any[] = [];
beforeEach(() => {
  posts = [];
  __resetBeacon();
  globalThis.fetch = (async (url: string, init: any) => {
    posts.push({ url, body: JSON.parse(init.body) });
    return new Response('{}', { status: 202 });
  }) as any;
  document.body.innerHTML = '';
});

async function mountGrid() {
  const host = document.createElement('div');
  host.setAttribute('data-prb-decision', 'd-grid');
  host.innerHTML = '<p id="title">Picks</p><div data-prb-item="c"><a id="c">c</a></div><div data-prb-item="a"><a id="a">a</a></div>';
  const mountPoint = document.createElement('span');
  host.appendChild(mountPoint);
  document.body.appendChild(host);
  await act(async () =>
    createRoot(mountPoint).render(createElement(PrbBeacon, { site: 'shop', slot: 'grid', decision: 'd-grid', subject: 'v1', eventsPath: '/_prb/e', items: ['c', 'a'] })),
  );
}
const ev = (name: string) => posts.flatMap((p) => p.body.events).filter((e: any) => e.name === name);

test('sorted_beacon_exposure_carries_the_rendered_order', async () => {
  await mountGrid();
  assert.deepEqual(ev('exposure')[0].items, [{ id: 'c' }, { id: 'a' }]);
});

test('sorted_beacon_click_names_the_card', async () => {
  await mountGrid();
  (document.getElementById('a') as any).click();
  assert.deepEqual(ev('click')[0].items, [{ id: 'a' }]);
  (document.getElementById('title') as any).click();
  assert.equal(ev('click')[1].items, undefined);
});
