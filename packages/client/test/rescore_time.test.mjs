// Time features on the on-device scorer: once a slot's model has learned them, the manifest's
// weights name `sin_hour` .. `is_business_hours_local`. The SDK computes them from the browser
// clock as Core does from tz_offset, with the night window the manifest ships as `night`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { load, ok, sleep } from './harness.mjs';

const TIME = { sin_hour: 0, cos_hour: 0, is_night_local: 0, is_weekend_local: 0, is_business_hours_local: 0 };
const W = (bias, t = {}) => Object.assign({ bias, returning: 0, mobile: 0, from_ad: 0 }, TIME, t);
const MANIFEST = (night) => ({ site: 'demo', privacy_mode: 'full', night,
  slots: [
    { key: 'taste', selector: '#taste', metric: 'buy', ttl_secs: 60, kind: 'picker',
      picker: { max_picks: 1, options: [{ key: 'kitchen', label: 'Kitchen' }] },
      variants: [{ key: 'none', baseline: true }, { key: 'picker', baseline: false }] },
    { key: 'shelf', selector: '#shelf', metric: 'click', ttl_secs: 60, in_session: true, variants: [
      { key: 'baseline', baseline: true, payload: null, weights: W(-1) },
      { key: 'plain', baseline: false, payload: '<p>P</p>', weights: W(-0.5) },
      { key: 'night', baseline: false, payload: '<p>N</p>', weights: W(-3, { is_night_local: 6 }) },
    ] },
  ] });
const BODY = '<a id="cat" data-prb-intent="kitchen">Kitchen</a><div id="taste"></div><div id="shelf">Shelf</div>';

// `at` is a local wall-clock time [year, month0, day, hour]; 2026-10-07 is a Wednesday.
async function rescored(at, { night, edit = (m) => m } = {}) {
  const decide = (body) => ok({ decision_id: 'd-' + body.slot, slot: body.slot, variant: body.slot === 'taste' ? 'none' : 'baseline', payload: null, explanation: { in_holdout: false } });
  const before = (w) => {
    const Real = w.Date;
    w.Date = class extends Real { constructor(...a) { super(...(a.length ? a : at)); } };
  };
  const p = load({ manifest: edit(MANIFEST(night)), body: BODY, decide, attrs: { 'data-timeout': '5000' }, before });
  await sleep(20);
  const shelf = p.w.document.getElementById('shelf');
  shelf.getBoundingClientRect = () => ({ top: 2000, bottom: 2100, left: 0, right: 100, width: 100, height: 100 });
  p.w.document.getElementById('cat').dispatchEvent(new p.w.MouseEvent('click', { bubbles: true }));
  await sleep(50);
  return p.events().find((e) => e.name === 'rescore' && e.slot === 'shelf');
}

const WED = (h) => [2026, 9, 7, h];
const SAT = (h) => [2026, 9, 10, h];

test('on_device_score_uses_the_night_flag', async () => {
  const r = await rescored(WED(23));
  assert.equal(r.variant, 'night');
  assert.equal(r.features.is_night_local, 1);
});

test('on_device_hour_sine_and_cosine_come_from_the_browser_clock', async () => {
  const r = await rescored(WED(18));
  assert.ok(Math.abs(r.features.sin_hour - -1) < 1e-9, JSON.stringify(r.features));
  assert.ok(Math.abs(r.features.cos_hour - 0) < 1e-9, JSON.stringify(r.features));
});

test('on_device_default_night_window_wraps_midnight', async () => {
  const r = await rescored(WED(3));
  assert.equal(r.features.is_night_local, 1);
});

test('on_device_daytime_is_not_night', async () => {
  const r = await rescored(WED(12));
  assert.equal(r.variant, 'plain');
  assert.equal(r.features.is_night_local, 0);
});

test('on_device_night_uses_the_manifest_window', async () => {
  const r = await rescored(WED(20), { night: { start: 19, end: 6 } });
  assert.equal(r.features.is_night_local, 1);
});

test('on_device_night_window_that_does_not_wrap_excludes_late_evening', async () => {
  const r = await rescored(WED(23), { night: { start: 1, end: 5 } });
  assert.equal(r.features.is_night_local, 0);
});

test('on_device_weekend_flag_on_a_saturday', async () => {
  const r = await rescored(SAT(12));
  assert.equal(r.features.is_weekend_local, 1);
  assert.equal(r.features.is_business_hours_local, 0);
});

test('on_device_weekday_is_not_weekend', async () => {
  const r = await rescored(WED(12));
  assert.equal(r.features.is_weekend_local, 0);
});

test('on_device_business_hours_on_a_weekday_afternoon', async () => {
  const r = await rescored(WED(14));
  assert.equal(r.features.is_business_hours_local, 1);
});

test('on_device_business_hours_end_at_17', async () => {
  const r = await rescored(WED(17));
  assert.equal(r.features.is_business_hours_local, 0);
});

test('on_device_business_hours_start_at_9', async () => {
  const r = await rescored(WED(9));
  assert.equal(r.features.is_business_hours_local, 1);
});

test('on_device_time_features_are_left_out_when_not_learned', async () => {
  const untimed = (m) => { m.slots[1].variants.forEach((v) => { for (const k of Object.keys(TIME)) delete v.weights[k]; }); return m; };
  const r = await rescored(WED(23), { edit: untimed });
  assert.ok(!('sin_hour' in r.features) && !('is_night_local' in r.features), JSON.stringify(r.features));
});
