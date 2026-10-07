// Declared features on the on-device scorer: the manifest's weights name `feature:<key>`; the
// SDK fills them from what it sends as ctx.features (1 or 0), and leaves a PII one out of the
// vector, and out of the rescore event, unless the visitor is identified.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { load, ok, sleep } from './harness.mjs';

const W = (bias, corp, member) => ({ bias, returning: 0, mobile: 0, from_ad: 0, 'feature:corp_email': corp, 'feature:loyalty_member': member, 'stated:kitchen': 0 });
const MANIFEST = (mode) => ({ site: 'demo', privacy_mode: mode,
  features: [{ key: 'corp_email', pii: true }, { key: 'loyalty_member', pii: false }],
  slots: [
    { key: 'taste', selector: '#taste', metric: 'buy', ttl_secs: 60, kind: 'picker',
      picker: { max_picks: 1, options: [{ key: 'kitchen', label: 'Kitchen' }] },
      variants: [{ key: 'none', baseline: true }, { key: 'picker', baseline: false }] },
    { key: 'shelf', selector: '#shelf', metric: 'click', ttl_secs: 60, in_session: true, variants: [
      { key: 'baseline', baseline: true, payload: null, weights: W(-1, 0, 0) },
      { key: 'plain', baseline: false, payload: '<p>P</p>', weights: W(-0.5, 0, 0) },
      { key: 'member', baseline: false, payload: '<p>M</p>', weights: W(-3, 0, 6) },
      { key: 'corp', baseline: false, payload: '<p>C</p>', weights: W(-2, 7, 0) },
    ] },
  ] });
const BODY = '<a id="cat" data-prb-intent="kitchen">Kitchen</a><div id="taste"></div><div id="shelf">Shelf</div>';

async function rescored(mode, features, attrs = {}, edit = (m) => m) {
  const decide = (body) => ok({ decision_id: 'd-' + body.slot, slot: body.slot, variant: body.slot === 'taste' ? 'none' : 'baseline', payload: null, explanation: { in_holdout: false } });
  const p = load({ manifest: edit(MANIFEST(mode)), body: BODY, decide, attrs: Object.assign({ 'data-timeout': '5000' }, attrs), before: (w) => { w.__prb_features = features; } });
  await sleep(20);
  const shelf = p.w.document.getElementById('shelf');
  shelf.getBoundingClientRect = () => ({ top: 2000, bottom: 2100, left: 0, right: 100, width: 100, height: 100 });
  p.w.document.getElementById('cat').dispatchEvent(new p.w.MouseEvent('click', { bubbles: true }));
  await sleep(50);
  return p.events().find((e) => e.name === 'rescore' && e.slot === 'shelf');
}

test('on_device_score_uses_a_sent_feature', async () => {
  const r = await rescored('full', { loyalty_member: true });
  assert.equal(r.variant, 'member');
  assert.equal(r.features['feature:loyalty_member'], 1);
});

test('on_device_unsent_declared_feature_is_zero', async () => {
  const r = await rescored('full', {});
  assert.equal(r.variant, 'plain');
  assert.equal(r.features['feature:loyalty_member'], 0);
});

test('on_device_pii_feature_is_used_when_identified', async () => {
  const r = await rescored('consent_aware', { corp_email: true }, { 'data-cookie': 'on' });
  assert.equal(r.variant, 'corp');
  assert.equal(r.features['feature:corp_email'], 1);
});

test('on_device_pii_feature_is_left_out_when_not_identified', async () => {
  const r = await rescored('consent_aware', { corp_email: true });
  assert.equal(r.variant, 'plain');
  assert.ok(!('feature:corp_email' in r.features), JSON.stringify(r.features));
});

// A model can still hold a feature the site file no longer declares; Core would refuse it.
test('on_device_undeclared_feature_weight_is_left_out', async () => {
  const stale = (m) => { m.slots[1].variants.forEach((v) => { v.weights['feature:age_band'] = 0; }); return m; };
  const r = await rescored('full', { loyalty_member: true, age_band: true }, {}, stale);
  assert.ok(!('feature:age_band' in r.features), JSON.stringify(r.features));
});
