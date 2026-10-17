// F2b: a sorted slot. The page marks each card with data-prb-item="<key>"; the SDK moves the
// cards into the decided order, reports the exposure with the order it rendered, and reports a
// click on a card with that card's id, which is what Core's cascade learns from.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { load, ok, sleep } from './harness.mjs';

const MANIFEST = {
  site: 'demo',
  slots: [{ key: 'grid', selector: '#grid', metric: 'click', ttl_secs: 60, kind: 'sorted', sorted: { visible: 2 },
            variants: ['a', 'b', 'c', 'd'].map((k) => ({ key: k, baseline: k === 'a' })) }],
};
const BODY = '<ul id="grid"><li id="h" class="note">Our picks</li>' +
  ['a', 'b', 'c', 'd'].map((k) => `<li data-prb-item="${k}"><a id="i-${k}">${k}</a></li>`).join('') + '</ul>';
// `b` is gated out for this visitor: not in the order.
const DECISION = { decision_id: 'd-9', slot: 'grid', variant: 'c', payload: null, order: ['c', 'a', 'd'], explanation: {} };

function page(d = DECISION) {
  return load({ manifest: MANIFEST, body: BODY, decide: () => ok(d) });
}
const rendered = (p) => [...p.w.document.querySelectorAll('#grid > [data-prb-item]')].map((e) => e.getAttribute('data-prb-item'));

test('sorted_slot_moves_the_cards_into_the_decided_order', async () => {
  const p = page();
  await sleep(20);
  // An item missing from the order keeps its place after the ordered ones: the page owns it.
  assert.deepEqual(rendered(p), ['c', 'a', 'd', 'b']);
  // A child that is not an item stays where it was.
  assert.equal(p.w.document.querySelector('#grid').firstElementChild.id, 'h');
  assert.equal(p.w.document.querySelector('#grid').getAttribute('data-prb-done'), '');
});

test('sorted_exposure_carries_the_rendered_order_as_items', async () => {
  const p = page();
  await sleep(20);
  const ex = p.events().filter((e) => e.name === 'exposure');
  assert.equal(ex.length, 1);
  assert.equal(ex[0].decision_id, 'd-9');
  assert.deepEqual(JSON.parse(JSON.stringify(ex[0].items)), [{ id: 'c' }, { id: 'a' }, { id: 'd' }]);
});

test('sorted_click_on_a_card_names_that_card', async () => {
  const p = page();
  await sleep(20);
  p.w.document.getElementById('i-d').click();
  await sleep(5);
  const cl = p.events().filter((e) => e.name === 'click');
  assert.equal(cl.length, 1);
  assert.equal(cl[0].decision_id, 'd-9');
  assert.deepEqual(JSON.parse(JSON.stringify(cl[0].items)), [{ id: 'd' }]);
});

test('sorted_click_outside_any_card_names_no_item', async () => {
  const p = page();
  await sleep(20);
  p.w.document.getElementById('h').click();
  await sleep(5);
  const cl = p.events().filter((e) => e.name === 'click');
  assert.equal(cl.length, 1);
  assert.equal(cl[0].items, undefined);
});

test('sorted_slot_without_an_order_keeps_the_page_order', async () => {
  // Core down: the slot is revealed as the page wrote it.
  const p = load({ manifest: MANIFEST, body: BODY, decide: () => Promise.reject(new Error('down')) });
  await sleep(20);
  assert.deepEqual(rendered(p), ['a', 'b', 'c', 'd']);
  assert.equal(p.visibility('grid'), 'visible');
});

test('a_swap_slot_never_reorders_its_children', async () => {
  // The order field on a non-sorted slot is ignored.
  const m = { site: 'demo', slots: [{ ...MANIFEST.slots[0], kind: undefined, sorted: undefined }] };
  const p = load({ manifest: m, body: BODY, decide: () => ok(DECISION) });
  await sleep(20);
  assert.deepEqual(rendered(p), ['a', 'b', 'c', 'd']);
});

test('sorted_slot_keeps_a_child_between_the_cards_in_its_place', async () => {
  // Same rule as the edge rewriter and the next SDK: the cards fill the cards' places in the
  // decided order; a child that is not a card (an ad tile, a divider) keeps its place.
  const body = '<ul id="grid"><li data-prb-item="a">a</li><li id="ad">ad</li><li data-prb-item="b">b</li><li data-prb-item="c">c</li></ul>';
  const p = load({ manifest: MANIFEST, body, decide: () => ok({ ...DECISION, order: ['c', 'a', 'b'] }) });
  await sleep(20);
  const seq = [...p.w.document.querySelector('#grid').children].map((e) => e.getAttribute('data-prb-item') ?? e.id);
  assert.deepEqual(seq, ['c', 'ad', 'a', 'b']);
});
