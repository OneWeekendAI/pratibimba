// Declared features: the page sets window.__prb_features before the SDK; the SDK sends only
// the names the manifest declares, only boolean values, and a PII one only when the visitor
// is identified (the same rule Core applies, so PII never leaves the device needlessly).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { load, sleep, MANIFEST } from './harness.mjs';

const DECLARED = [{ key: 'corp_email', pii: true }, { key: 'loyalty_member', pii: false }];
const manifest = (mode) => Object.assign({}, MANIFEST, { privacy_mode: mode, features: DECLARED });
const page = (features) => (w) => { w.__prb_features = features; };
async function sent(opts) {
  const p = load(opts);
  await sleep(5);
  return p.decides()[0].body.ctx;
}

test('non_pii_feature_is_sent_without_consent', async () => {
  const ctx = await sent({ manifest: manifest('consent_aware'), before: page({ loyalty_member: true }) });
  assert.deepEqual(ctx.features, { loyalty_member: true });
});

test('pii_feature_is_not_sent_in_consent_aware_mode_without_consent', async () => {
  const ctx = await sent({ manifest: manifest('consent_aware'), before: page({ corp_email: true, loyalty_member: false }) });
  assert.deepEqual(ctx.features, { loyalty_member: false });
});

test('pii_feature_is_sent_in_consent_aware_mode_with_consent', async () => {
  const ctx = await sent({ manifest: manifest('consent_aware'), attrs: { 'data-cookie': 'on' }, before: page({ corp_email: true }) });
  assert.deepEqual(ctx.features, { corp_email: true });
});

test('pii_feature_is_not_sent_in_cookieless_mode_even_with_consent', async () => {
  const ctx = await sent({ manifest: manifest('cookieless'), attrs: { 'data-cookie': 'on' }, before: page({ corp_email: true }) });
  assert.equal(ctx.features, undefined);
});

test('pii_feature_is_sent_in_full_mode', async () => {
  const ctx = await sent({ manifest: manifest('full'), before: page({ corp_email: true }) });
  assert.deepEqual(ctx.features, { corp_email: true });
});

test('undeclared_feature_is_not_sent', async () => {
  const ctx = await sent({ manifest: manifest('full'), before: page({ age_band: true, loyalty_member: true }) });
  assert.deepEqual(ctx.features, { loyalty_member: true });
});

test('feature_with_a_non_boolean_value_is_not_sent', async () => {
  const ctx = await sent({ manifest: manifest('full'), before: page({ loyalty_member: 'yes', corp_email: 1 }) });
  assert.equal(ctx.features, undefined);
});

test('no_features_field_when_the_page_sets_none', async () => {
  const ctx = await sent({ manifest: manifest('full') });
  assert.equal(ctx.features, undefined);
});

test('feature_is_not_sent_when_the_manifest_declares_none', async () => {
  const ctx = await sent({ manifest: Object.assign({}, MANIFEST, { privacy_mode: 'full' }), before: page({ loyalty_member: true }) });
  assert.equal(ctx.features, undefined);
});
