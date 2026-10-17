// Single-page apps: React, Vue and friends mount the slot elements after DOMContentLoaded, often
// after a fetch. The SDK watches the DOM and applies a slot's decision to each element as it
// mounts, before it is painted. A slot element that mounts before its decision is held hidden for
// at most data-timeout ms counted from its own mount, then shown as the page drew it (fail open).
// An element the visitor has seen as the baseline is never swapped, and neither is one under the
// pointer.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { load, ok, sleep, DECISION } from './harness.mjs';

const EMPTY = '<nav id="nav">Nav</nav><div id="root"></div>';

function mount(p, html = '<a id="cta">Original</a>', id = 'hero') {
  const el = p.w.document.createElement('div');
  el.id = id;
  el.innerHTML = html;
  p.w.document.getElementById('root').appendChild(el);
  return el;
}
const tick = () => sleep(0);
const names = (p) => p.events().map((e) => e.name);

test('spa_slot_mounted_after_the_decision_gets_the_variant', async () => {
  const p = load({ body: EMPTY, attrs: { 'data-timeout': '5000' } });
  await sleep(20);
  assert.deepEqual(names(p), [], 'nothing is exposed while no slot element exists');
  mount(p);
  await tick();
  assert.equal(p.hero().innerHTML, '<h1>Gift sets</h1>');
  assert.equal(p.hero().getAttribute('data-prb-variant'), 'gift');
  assert.equal(p.visibility('hero'), 'visible');
  assert.deepEqual(names(p), ['exposure']);
});

test('spa_swap_happens_before_the_page_can_paint', async () => {
  // A MutationObserver callback is a microtask: it runs before the browser renders again.
  const p = load({ body: EMPTY, attrs: { 'data-timeout': '5000' } });
  await sleep(20);
  mount(p);
  await Promise.resolve();
  assert.equal(p.hero().innerHTML, '<h1>Gift sets</h1>');
});

test('spa_slot_mounted_before_its_decision_stays_hidden_until_it_lands', async () => {
  const p = load({ body: EMPTY, attrs: { 'data-timeout': '300' }, decide: () => sleep(60).then(() => ok(DECISION)) });
  await sleep(10);
  mount(p);
  await tick();
  assert.equal(p.visibility('hero'), 'hidden');
  await sleep(90);
  assert.equal(p.hero().innerHTML, '<h1>Gift sets</h1>');
  assert.equal(p.visibility('hero'), 'visible');
  assert.deepEqual(names(p), ['exposure']);
});

test('spa_late_mount_is_held_for_its_own_timeout_not_the_page_load', async () => {
  // The page-load deadline (60 ms) has passed when the element mounts at 100 ms; the decision
  // lands at 140 ms, inside the element's own 60 ms.
  const p = load({ body: EMPTY, attrs: { 'data-timeout': '60' }, decide: () => sleep(140).then(() => ok(DECISION)) });
  await sleep(100);
  mount(p);
  await tick();
  assert.equal(p.visibility('hero'), 'hidden', 'a late mount is held, not shown as the baseline');
  await sleep(70);
  assert.equal(p.hero().innerHTML, '<h1>Gift sets</h1>');
  assert.equal(p.visibility('hero'), 'visible');
  assert.equal(p.hero().hasAttribute('data-prb-wait'), false);
  assert.deepEqual(names(p), ['exposure']);
});

test('spa_held_slot_fails_open_and_a_later_decision_is_missed', async () => {
  let land;
  const p = load({ body: EMPTY, attrs: { 'data-timeout': '50' }, decide: () => new Promise((r) => { land = () => r({ ok: true, status: 200, json: () => Promise.resolve(DECISION) }); }) });
  await sleep(80);
  mount(p);
  await tick();
  assert.equal(p.visibility('hero'), 'hidden');
  await sleep(80);
  assert.equal(p.visibility('hero'), 'visible', 'never held longer than data-timeout');
  assert.equal(p.hero().textContent, 'Original');
  land();
  await sleep(20);
  assert.equal(p.hero().textContent, 'Original', 'the visitor saw the baseline: no swap');
  p.hero().remove();
  mount(p);
  await tick();
  assert.equal(p.hero().textContent, 'Original', 'nor on a remount later in the same page');
  assert.deepEqual(names(p), ['missed']);
});

test('spa_remount_gets_the_same_decision_and_one_exposure', async () => {
  const p = load({ body: EMPTY, attrs: { 'data-timeout': '5000' } });
  await sleep(20);
  const first = mount(p);
  await tick();
  assert.equal(first.innerHTML, '<h1>Gift sets</h1>');
  first.remove();
  await tick();
  mount(p);
  await tick();
  assert.equal(p.hero().innerHTML, '<h1>Gift sets</h1>');
  assert.deepEqual(names(p), ['exposure']);
});

test('spa_slot_nested_in_a_mounted_subtree_is_found', async () => {
  const p = load({ body: EMPTY, attrs: { 'data-timeout': '5000' } });
  await sleep(20);
  const wrap = p.w.document.createElement('main');
  wrap.innerHTML = '<section><div id="hero"><a>Original</a></div></section>';
  p.w.document.getElementById('root').appendChild(wrap);
  await tick();
  assert.equal(p.hero().innerHTML, '<h1>Gift sets</h1>');
});

test('spa_never_swaps_an_element_under_the_pointer', async () => {
  // The page's own CSS can make a child visible inside a held slot (visibility is inherited and
  // overridable). If the pointer is on it when the decision lands, the slot keeps the baseline.
  const p = load({ body: EMPTY, attrs: { 'data-timeout': '300' }, decide: () => sleep(60).then(() => ok(DECISION)) });
  await sleep(10);
  mount(p);
  await tick();
  p.w.document.getElementById('cta').dispatchEvent(new p.w.MouseEvent('mouseover', { bubbles: true }));
  await sleep(90);
  assert.equal(p.hero().textContent, 'Original');
  assert.equal(p.visibility('hero'), 'visible');
  assert.deepEqual(names(p), ['missed']);
});

test('spa_never_swaps_an_element_the_pointer_moved_onto', async () => {
  // pointermove alone (a pen or touch pointer; no mouseover, so no :hover in jsdom).
  const p = load({ body: EMPTY, attrs: { 'data-timeout': '300' }, decide: () => sleep(60).then(() => ok(DECISION)) });
  await sleep(10);
  mount(p);
  await tick();
  p.w.document.getElementById('cta').dispatchEvent(new p.w.Event('pointermove', { bubbles: true }));
  await sleep(90);
  assert.equal(p.hero().textContent, 'Original');
  assert.deepEqual(names(p), ['missed']);
});

test('spa_click_on_a_late_mounted_slot_is_reported_once', async () => {
  const p = load({ body: EMPTY, attrs: { 'data-timeout': '5000' }, decide: () => ok({ ...DECISION, payload: '<a id="go">Gift sets</a>' }) });
  await sleep(20);
  mount(p);
  await tick();
  p.w.document.getElementById('go').click();
  await tick();
  assert.deepEqual(names(p), ['exposure', 'click']);
});

test('spa_sorted_slot_mounted_late_is_reordered', async () => {
  const manifest = { site: 'demo', slots: [{ key: 'grid', selector: '#grid', metric: 'click', ttl_secs: 60, kind: 'sorted',
    variants: ['a', 'b', 'c'].map((k) => ({ key: k })) }] };
  const p = load({ manifest, body: EMPTY, attrs: { 'data-timeout': '5000' },
    decide: () => ok({ decision_id: 'd-2', slot: 'grid', variant: 'c', payload: null, order: ['c', 'b', 'a'], explanation: {} }) });
  await sleep(20);
  mount(p, ['a', 'b', 'c'].map((k) => `<article data-prb-item="${k}">${k}</article>`).join(''), 'grid');
  await tick();
  const order = [...p.w.document.querySelectorAll('#grid > [data-prb-item]')].map((e) => e.getAttribute('data-prb-item'));
  assert.deepEqual(order, ['c', 'b', 'a']);
  assert.deepEqual(JSON.parse(JSON.stringify(p.events()[0].items)), [{ id: 'c' }, { id: 'b' }, { id: 'a' }]);
});

test('spa_copy_slot_mounted_late_gets_its_text', async () => {
  const manifest = { site: 'demo', slots: [{ key: 'hero', selector: '#hero', metric: 'click', ttl_secs: 60, kind: 'copy', variants: [] }] };
  const p = load({ manifest, body: EMPTY, attrs: { 'data-timeout': '5000' },
    decide: () => ok({ decision_id: 'd-3', slot: 'hero', variant: 'back', payload: null, copy: { headline: 'Welcome back' }, explanation: {} }) });
  await sleep(20);
  mount(p, '<h1 data-prb-field="headline">Cooked fresh</h1>');
  await tick();
  assert.equal(p.hero().querySelector('h1').textContent, 'Welcome back');
});

test('spa_failed_decide_shows_late_mounts_at_once', async () => {
  const p = load({ body: EMPTY, attrs: { 'data-timeout': '5000' }, decide: () => Promise.resolve({ ok: false, status: 500, json: () => Promise.resolve({}) }) });
  await sleep(20);
  mount(p);
  await tick();
  assert.equal(p.visibility('hero'), 'visible');
  assert.equal(p.hero().textContent, 'Original');
});

test('spa_decision_after_the_baseline_was_seen_is_missed_even_if_the_element_left', async () => {
  let land;
  const p = load({ body: EMPTY, attrs: { 'data-timeout': '30' }, decide: () => new Promise((r) => { land = () => r({ ok: true, status: 200, json: () => Promise.resolve(DECISION) }); }) });
  await sleep(50);
  mount(p);
  await sleep(60);
  p.hero().remove();
  land();
  await sleep(20);
  assert.deepEqual(names(p), ['missed']);
});
