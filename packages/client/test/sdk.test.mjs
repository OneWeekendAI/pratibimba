import { test } from 'node:test';
import assert from 'node:assert/strict';
import { load, ok, sleep, DECISION } from './harness.mjs';

test('hides_only_slot_selectors_before_dom_ready', () => {
  const p = load({ decide: () => new Promise(() => {}) });
  assert.equal(p.w.__hiddenAtParse, true, 'the hide rule must exist before the body is parsed');
  assert.equal(p.visibility('hero'), 'hidden');
  assert.equal(p.visibility('nav'), 'visible');
});

test('swaps_payload_into_slot', async () => {
  const p = load();
  await sleep(10);
  assert.equal(p.hero().innerHTML, '<h1>Gift sets</h1>');
});

test('reveals_each_slot_as_it_swaps', async () => {
  // Two slots: hero answers at once, promo never does. With one slot, settling it removes the
  // whole hide rule and per-slot reveal is never exercised; it takes two to tell them apart.
  const manifest = { site: 'demo', slots: [
    { key: 'hero', selector: '#hero', metric: 'click', ttl_secs: 60, variants: [] },
    { key: 'promo', selector: '#promo', metric: 'click', ttl_secs: 60, variants: [] },
  ] };
  const p = load({ manifest, attrs: { 'data-timeout': '5000' },
    decide: (body) => (body.slot === 'hero' ? ok(DECISION) : new Promise(() => {})) });
  await sleep(10);
  assert.equal(p.visibility('hero'), 'visible');
  assert.equal(p.visibility('promo'), 'hidden');
});

test('timeout_reveals_baseline_when_decide_hangs', async () => {
  const p = load({ decide: () => new Promise(() => {}) });
  await sleep(100);
  assert.equal(p.w.document.getElementById('prb-hide'), null);
  assert.equal(p.visibility('hero'), 'visible');
  assert.equal(p.hero().textContent, 'Original');
});

test('late_decision_is_not_swapped_and_reports_missed', async () => {
  const p = load({ decide: () => sleep(120).then(() => ok(DECISION)) });
  await sleep(180);
  assert.equal(p.hero().textContent, 'Original');
  const names = p.events().map((e) => e.name);
  assert.deepEqual(names, ['missed']);
  assert.equal(p.events()[0].decision_id, 'd-1');
});

test('exposure_carries_decision_id', async () => {
  const p = load();
  await sleep(10);
  const ex = p.events().filter((e) => e.name === 'exposure');
  assert.equal(ex.length, 1);
  assert.equal(ex[0].decision_id, 'd-1');
  assert.equal(ex[0].slot, 'hero');
});

test('null_payload_keeps_baseline_and_still_exposes', async () => {
  const p = load({ decide: () => ok({ ...DECISION, variant: 'baseline', payload: null }) });
  await sleep(10);
  assert.equal(p.hero().textContent, 'Original');
  assert.equal(p.events().filter((e) => e.name === 'exposure').length, 1);
});

test('decide_error_reveals_immediately', async () => {
  const p = load({ attrs: { 'data-timeout': '5000' }, decide: () => Promise.resolve({ ok: false, status: 500, json: () => Promise.resolve({}) }) });
  await sleep(20);
  assert.equal(p.visibility('hero'), 'visible', 'an error must not hold the slot hidden until the timeout');
  assert.equal(p.hero().textContent, 'Original');
});

test('cached_decision_skips_network_within_ttl', async () => {
  const p = load({
    before(w) {
      w.sessionStorage.setItem('prb_subject', 's-1');
      w.sessionStorage.setItem('prb_d_demo_hero', JSON.stringify({ subject: 's-1', at: Date.now(), d: { ...DECISION, payload: '<h1>Cached</h1>' } }));
    },
  });
  await sleep(10);
  assert.equal(p.decides().length, 0);
  assert.equal(p.hero().innerHTML, '<h1>Cached</h1>');
});

test('expired_cache_decides_again', async () => {
  const p = load({
    before(w) {
      w.sessionStorage.setItem('prb_subject', 's-1');
      w.sessionStorage.setItem('prb_d_demo_hero', JSON.stringify({ subject: 's-1', at: Date.now() - 61000, d: { ...DECISION, payload: '<h1>Cached</h1>' } }));
    },
  });
  await sleep(10);
  assert.equal(p.decides().length, 1);
  assert.equal(p.hero().innerHTML, '<h1>Gift sets</h1>');
});

test('click_in_slot_reports_click', async () => {
  const p = load();
  await sleep(10);
  p.w.document.querySelector('#hero h1').dispatchEvent(new p.w.MouseEvent('click', { bubbles: true }));
  await sleep(5);
  const clicks = p.events().filter((e) => e.name === 'click');
  assert.equal(clicks.length, 1);
  assert.equal(clicks[0].decision_id, 'd-1');
});

test('no_cookie_without_opt_in', async () => {
  const p = load();
  await sleep(5);
  assert.equal(p.w.document.cookie, '');
  assert.ok(p.w.sessionStorage.getItem('prb_subject'));
});

test('cookie_with_opt_in', async () => {
  const p = load({ attrs: { 'data-cookie': 'on' } });
  await sleep(5);
  assert.match(p.w.document.cookie, /_prb_subject=/);
});

test('returning_only_when_cookie_predates_session', async () => {
  const fresh = load({ attrs: { 'data-cookie': 'on' } });
  const back = load({ attrs: { 'data-cookie': 'on' }, before: (w) => { w.document.cookie = '_prb_subject=old-visitor'; } });
  await sleep(5);
  assert.equal(fresh.decides()[0].body.ctx.returning, false);
  assert.equal(back.decides()[0].body.ctx.returning, true);
  assert.equal(back.decides()[0].body.subject, 'old-visitor');
});

test('from_ad_set_by_campaign_params', async () => {
  const plain = load();
  const ad = load({ url: 'https://shop.test/?utm_source=meta' });
  await sleep(5);
  assert.equal(plain.decides()[0].body.ctx.from_ad, false);
  assert.equal(ad.decides()[0].body.ctx.from_ad, true);
});

test('track_posts_goal_without_decision', async () => {
  const p = load();
  await sleep(10);
  p.w.prb.track('purchase');
  await sleep(5);
  const goals = p.events().filter((e) => e.name === 'purchase');
  assert.equal(goals.length, 1);
  assert.equal(goals[0].decision_id, undefined);
});
