// The taste picker and the on-device re-scorer, in jsdom. jsdom lays nothing out: every
// element reports a zero rect, which the SDK reads as "in the viewport". A test that wants a
// slot below the fold says so with `below()`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { load, ok, sleep } from './harness.mjs';

const W = (bias, kitchen) => ({ bias, returning: 0, mobile: 0, from_ad: 0, 'stated:kitchen': kitchen, 'stated:garden': 0, 'stated:books': 0 });
const IN_SESSION = [
  { key: 'baseline', baseline: true, payload: null, weights: W(-1, 0) },
  { key: 'kitchen', baseline: false, payload: '<p>Kitchen picks</p>', weights: W(-3, 6) },
];
const MANIFEST = { site: 'demo', slots: [
  { key: 'taste', selector: '#taste', metric: 'buy', ttl_secs: 60, kind: 'picker',
    picker: { max_picks: 2, options: [{ key: 'kitchen', label: 'Kitchen' }, { key: 'garden', label: 'Garden' }, { key: 'books', label: 'Books' }] },
    variants: [{ key: 'none', baseline: true }, { key: 'picker', baseline: false }] },
  { key: 'shelf', selector: '#shelf', metric: 'click', ttl_secs: 60, in_session: true, variants: IN_SESSION },
  { key: 'side', selector: '#side', metric: 'click', ttl_secs: 60, in_session: true, variants: IN_SESSION },
  { key: 'next', selector: '#only-on-the-next-page', metric: 'click', ttl_secs: 60, in_session: true, variants: IN_SESSION },
] };
const BODY = '<nav><a id="cat" data-prb-intent="kitchen">Kitchen</a></nav><div id="taste"></div>' +
  '<div id="shelf"><span id="shelf-text">Shelf original</span><input id="shelf-input"></div><div id="side">Side original</div>';

function page(opts = {}) {
  const held = !!opts.holdout;
  const decide = (body) => ok(body.slot === 'taste'
    ? { decision_id: 'd-taste', slot: 'taste', variant: opts.tasteVariant || 'picker', payload: '<p>What are you shopping for?</p>', explanation: { in_holdout: held } }
    : { decision_id: 'd-' + body.slot, slot: body.slot, variant: 'baseline', payload: null, explanation: { in_holdout: held } });
  const p = load({ manifest: MANIFEST, body: BODY, decide, attrs: Object.assign({ 'data-timeout': '5000' }, opts.attrs || {}), before: opts.before });
  p.el = (id) => p.w.document.getElementById(id);
  p.below = (id) => { p.el(id).getBoundingClientRect = () => ({ top: 2000, bottom: 2100, left: 0, right: 100, width: 100, height: 100 }); };
  p.click = (el) => el.dispatchEvent(new p.w.MouseEvent('click', { bubbles: true }));
  p.option = (key) => p.el('taste').querySelector(`[data-prb-option="${key}"]`);
  p.pickAndDone = (...keys) => { keys.forEach((k) => p.click(p.option(k))); p.click(p.el('taste').querySelector('[data-prb-submit]')); };
  p.named = (name) => p.events().filter((e) => e.name === name);
  return p;
}

// --- the picker -------------------------------------------------------------------------------

test('picker_renders_options_done_and_skip', async () => {
  const p = page();
  await sleep(20);
  const t = p.el('taste');
  assert.equal(t.querySelectorAll('[data-prb-option]').length, 3);
  assert.equal(p.option('garden').textContent, 'Garden');
  assert.ok(t.querySelector('[data-prb-skip]'), 'always skippable');
  assert.ok(t.querySelector('[data-prb-submit]').disabled, 'nothing picked yet');
});

test('picker_none_variant_renders_nothing', async () => {
  const p = page({ tasteVariant: 'none' });
  await sleep(20);
  assert.equal(p.el('taste').querySelectorAll('[data-prb-option]').length, 0);
});

test('pick_sends_option_keys_with_the_decision_id', async () => {
  const p = page();
  await sleep(20);
  p.pickAndDone('kitchen', 'books');
  await sleep(10);
  const picks = p.named('pick');
  assert.equal(picks.length, 1);
  assert.deepEqual(picks[0].picks, ['kitchen', 'books']);
  assert.equal(picks[0].decision_id, 'd-taste');
  assert.equal(picks[0].slot, 'taste');
  assert.deepEqual(JSON.parse(p.w.sessionStorage.getItem('prb_stated')), { taste: ['kitchen', 'books'] });
  assert.equal(p.el('taste').querySelectorAll('[data-prb-option]').length, 0, 'the picker closes after a pick');
});

test('pick_is_capped_at_max_picks', async () => {
  const p = page();
  await sleep(20);
  p.pickAndDone('kitchen', 'garden', 'books');
  await sleep(10);
  assert.deepEqual(p.named('pick')[0].picks, ['kitchen', 'garden']);
});

test('skip_sends_skip_and_stores_nothing', async () => {
  const p = page();
  await sleep(20);
  p.click(p.el('taste').querySelector('[data-prb-skip]'));
  await sleep(30);
  assert.equal(p.named('skip').length, 1);
  assert.equal(p.named('pick').length, 0);
  assert.equal(p.w.sessionStorage.getItem('prb_stated'), null);
  assert.equal(p.named('rescore').length, 0, 'a skip is not a signal');
  assert.equal(p.el('taste').querySelectorAll('[data-prb-option]').length, 0);
});

test('stated_rides_on_later_decides_and_drops_unknown_keys', async () => {
  const p = page({ before(w) { w.sessionStorage.setItem('prb_stated', JSON.stringify({ taste: ['kitchen', 'gone'] })); } });
  await sleep(20);
  for (const d of p.decides()) assert.deepEqual(d.body.ctx.stated, ['kitchen']);
});

test('no_stated_means_no_ctx_stated', async () => {
  const p = page();
  await sleep(20);
  for (const d of p.decides()) assert.equal(d.body.ctx.stated, undefined);
});

test('stated_is_session_scoped_without_cookies', async () => {
  const p = page();
  await sleep(20);
  p.pickAndDone('kitchen');
  assert.equal(p.w.localStorage.getItem('prb_stated'), null);
  assert.ok(p.w.sessionStorage.getItem('prb_stated'));
});

test('stated_persists_only_with_cookies_on', async () => {
  const p = page({ attrs: { 'data-cookie': 'on' } });
  await sleep(20);
  p.pickAndDone('kitchen');
  assert.deepEqual(JSON.parse(p.w.localStorage.getItem('prb_stated')), { taste: ['kitchen'] });
});

// --- on-device re-scoring ---------------------------------------------------------------------

test('a_pick_reswaps_a_below_the_fold_slot', async () => {
  const p = page();
  await sleep(20);
  p.below('shelf');
  p.pickAndDone('kitchen');
  await sleep(50);
  assert.equal(p.el('shelf').innerHTML, '<p>Kitchen picks</p>');
  assert.equal(p.el('shelf').getAttribute('data-prb-variant'), 'kitchen');
  const r = p.named('rescore').find((e) => e.slot === 'shelf');
  assert.equal(r.variant, 'kitchen');
  assert.equal(r.features['stated:kitchen'], 1);
  assert.equal(r.features.bias, 1);
  assert.ok(r.score > 0.9 && r.score <= 1, `score ${r.score}`);
  const ex = p.named('exposure').filter((e) => e.slot === 'shelf');
  assert.equal(ex[ex.length - 1].decision_id, r.decision_id, 'the swap is exposed under the on-device decision');
});

test('a_click_after_a_reswap_credits_the_new_decision', async () => {
  const p = page();
  await sleep(20);
  p.below('shelf');
  p.pickAndDone('kitchen');
  await sleep(50);
  p.click(p.el('shelf').querySelector('p'));
  const r = p.named('rescore').find((e) => e.slot === 'shelf');
  assert.equal(p.named('click').filter((e) => e.slot === 'shelf').pop().decision_id, r.decision_id);
});

test('never_swaps_the_hovered_slot', async () => {
  const p = page();
  await sleep(20);
  p.below('shelf');
  p.below('side');
  p.el('shelf-text').dispatchEvent(new p.w.MouseEvent('mouseover', { bubbles: true }));
  p.pickAndDone('kitchen');
  await sleep(50);
  assert.equal(p.el('shelf-text').textContent, 'Shelf original', 'the element under the cursor must not change');
  assert.equal(p.el('side').innerHTML, '<p>Kitchen picks</p>', 'the slot the visitor is not touching does swap');
});

test('never_swaps_the_focused_slot', async () => {
  const p = page();
  await sleep(20);
  p.below('shelf');
  p.below('side');
  p.el('shelf-input').focus();
  p.pickAndDone('kitchen');
  await sleep(50);
  assert.equal(p.el('shelf-text').textContent, 'Shelf original');
  assert.equal(p.w.document.activeElement.id, 'shelf-input');
  assert.equal(p.el('side').innerHTML, '<p>Kitchen picks</p>');
});

test('never_swaps_a_slot_in_the_viewport_but_keeps_it_for_the_next_page', async () => {
  const p = page();
  await sleep(20);
  p.pickAndDone('kitchen');
  await sleep(50);
  assert.equal(p.el('shelf-text').textContent, 'Shelf original');
  const cached = JSON.parse(p.w.sessionStorage.getItem('prb_d_demo_shelf'));
  assert.equal(cached.d.variant, 'kitchen');
  assert.equal(cached.d.decision_id, p.named('rescore').find((e) => e.slot === 'shelf').decision_id);
});

test('a_next_page_slot_is_decided_on_the_device_and_cached', async () => {
  const p = page();
  await sleep(20);
  p.pickAndDone('kitchen');
  await sleep(50);
  const cached = JSON.parse(p.w.sessionStorage.getItem('prb_d_demo_next'));
  assert.equal(cached.d.variant, 'kitchen');
  assert.equal(cached.d.payload, '<p>Kitchen picks</p>');
  assert.equal(cached.d.explanation.policy, 'on_device');
});

test('a_holdout_visitor_is_never_rescored', async () => {
  const p = page({ holdout: true });
  await sleep(20);
  p.below('shelf');
  p.pickAndDone('kitchen');
  await sleep(50);
  assert.equal(p.el('shelf-text').textContent, 'Shelf original');
  assert.equal(p.named('rescore').length, 0);
  assert.equal(p.named('pick').length, 1, 'the pick itself is still recorded');
});

test('an_intent_click_rescores', async () => {
  const p = page();
  await sleep(20);
  p.below('shelf');
  p.click(p.el('cat'));
  await sleep(50);
  assert.equal(p.el('shelf').innerHTML, '<p>Kitchen picks</p>');
  const r = p.named('rescore').find((e) => e.slot === 'shelf');
  assert.equal(r.features['stated:kitchen'], 0.5);
});

test('adapt_min_holds_rescoring_until_enough_signals', async () => {
  const p = page({ attrs: { 'data-adapt-min': '2' } });
  await sleep(20);
  p.below('shelf');
  p.click(p.el('cat'));
  await sleep(50);
  assert.equal(p.el('shelf-text').textContent, 'Shelf original');
  p.click(p.el('cat'));
  await sleep(50);
  assert.equal(p.el('shelf').innerHTML, '<p>Kitchen picks</p>');
});

test('no_swap_when_the_device_agrees_with_the_server', async () => {
  const p = page();
  await sleep(20);
  p.below('shelf');
  p.pickAndDone('garden');
  await sleep(50);
  assert.equal(p.el('shelf-text').textContent, 'Shelf original');
  assert.equal(p.named('rescore').filter((e) => e.slot === 'shelf').length, 0);
});
