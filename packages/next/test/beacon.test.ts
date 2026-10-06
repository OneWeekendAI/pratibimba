import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'https://shop.test/' });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
Object.defineProperty(globalThis, 'navigator', { value: dom.window.navigator, configurable: true });

const { createElement, act } = await import('react');
const { createRoot } = await import('react-dom/client');
const { PrbBeacon, __resetBeacon } = await import('../src/beacon.ts');

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

async function mount(decision = 'd-1') {
  const host = document.createElement('div');
  host.setAttribute('data-prb-decision', decision);
  host.innerHTML = '<a id="cta">Shop</a>';
  const mountPoint = document.createElement('span');
  host.appendChild(mountPoint);
  document.body.appendChild(host);
  const root = createRoot(mountPoint);
  await act(async () => root.render(createElement(PrbBeacon, { site: 'shop', slot: 'hero', decision, subject: 'v1', eventsPath: '/_prb/e' })));
  return { host, root };
}

test('beacon_reports_exposure_once_hydrated', async () => {
  await mount();
  assert.equal(posts.length, 1);
  assert.equal(posts[0].url, '/_prb/e');
  assert.deepEqual(
    { name: posts[0].body.events[0].name, decision: posts[0].body.events[0].decision_id, subject: posts[0].body.subject },
    { name: 'exposure', decision: 'd-1', subject: 'v1' },
  );
});

test('beacon_reports_a_click_inside_the_slot', async () => {
  const { host } = await mount('d-2');
  (host.querySelector('#cta') as any).click();
  const clicks = posts.filter((p) => p.body.events[0].name === 'click');
  assert.equal(clicks.length, 1);
  assert.equal(clicks[0].body.events[0].decision_id, 'd-2');
});

test('beacon_reports_one_exposure_per_decision', async () => {
  await mount('d-3');
  await mount('d-3');
  assert.equal(posts.filter((p) => p.body.events[0].name === 'exposure').length, 1);
});
