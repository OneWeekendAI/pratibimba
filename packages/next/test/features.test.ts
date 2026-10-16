// Declared features on the server render: the owner's `features` callback names them; the slot
// sends as ctx.features only names the manifest declares, only booleans, and a pii one only
// when the visitor is identified (full, or consent_aware with consent).

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { createSlot, type SlotConfig } from '../src/slot.ts';

const DECLARED = [{ key: 'corp_email', pii: true }, { key: 'loyalty_member', pii: false }];

async function render(opts: { mode?: string; declared?: unknown; consent?: '1' | '0'; features?: SlotConfig['features']; manifestStatus?: number }) {
  const decides: any[] = [];
  let manifests = 0;
  const fetch = (async (url: string, init: any) => {
    if (url.includes('/manifest/')) {
      manifests += 1;
      if (opts.manifestStatus) return Response.json({ site: 'shop', privacy_mode: 'full', slots: [], features: DECLARED }, { status: opts.manifestStatus });
      return Response.json({ site: 'shop', privacy_mode: opts.mode ?? 'full', slots: [], ...(opts.declared === undefined ? { features: DECLARED } : opts.declared ? { features: opts.declared } : {}) });
    }
    const body = JSON.parse(init.body);
    decides.push(body);
    return Response.json({ decisions: body.slots.map((slot: string) => ({ decision_id: `d-${slot}`, slot, variant: 'gift', payload: null, explanation: {} })) });
  }) as unknown as typeof globalThis.fetch;
  const h: Record<string, string> = { 'x-prb-subject': 'v1', 'x-prb-ctx': 'r1m0a0', ...(opts.consent ? { 'x-prb-consent': opts.consent } : {}) };
  const { Slot } = createSlot({ core: 'http://core', site: 'shop', fetch, headers: async () => new Headers(h), requestScope: () => ({}), features: opts.features });
  const el: any = await Slot({ id: 'hero', children: createElement('h1', null, 'Espresso') });
  return { ctx: decides[0].ctx, manifests, variant: el.props['data-prb-variant'] };
}
const on = (v: Record<string, unknown>) => () => v;

test('next_non_pii_feature_is_sent_without_consent', async () => {
  const { ctx } = await render({ mode: 'consent_aware', features: on({ loyalty_member: true }) });
  assert.deepEqual(ctx.features, { loyalty_member: true });
});

test('next_pii_feature_is_not_sent_in_consent_aware_mode_without_consent', async () => {
  const { ctx } = await render({ mode: 'consent_aware', consent: '0', features: on({ corp_email: true, loyalty_member: false }) });
  assert.deepEqual(ctx.features, { loyalty_member: false });
});

test('next_pii_feature_is_sent_in_consent_aware_mode_with_consent', async () => {
  const { ctx } = await render({ mode: 'consent_aware', consent: '1', features: on({ corp_email: true }) });
  assert.deepEqual(ctx.features, { corp_email: true });
});

test('next_pii_feature_is_not_sent_in_cookieless_mode_even_with_consent', async () => {
  const { ctx } = await render({ mode: 'cookieless', consent: '1', features: on({ corp_email: true }) });
  assert.equal(ctx.features, undefined);
});

test('next_pii_feature_is_sent_in_full_mode', async () => {
  const { ctx } = await render({ mode: 'full', features: on({ corp_email: true }) });
  assert.deepEqual(ctx.features, { corp_email: true });
});

test('next_undeclared_feature_is_not_sent', async () => {
  const { ctx } = await render({ features: on({ age_band: true, loyalty_member: true }) });
  assert.deepEqual(ctx.features, { loyalty_member: true });
});

test('next_feature_with_a_non_boolean_value_is_not_sent', async () => {
  const { ctx } = await render({ features: on({ loyalty_member: 'yes', corp_email: 1 }) });
  assert.equal(ctx.features, undefined);
});

test('next_feature_is_not_sent_when_the_manifest_declares_none', async () => {
  const { ctx } = await render({ declared: null, features: on({ loyalty_member: true }) });
  assert.equal(ctx.features, undefined);
});

test('next_async_features_callback_is_awaited', async () => {
  const { ctx } = await render({ features: async () => ({ loyalty_member: true }) });
  assert.deepEqual(ctx.features, { loyalty_member: true });
});

test('next_features_callback_reads_the_request_headers', async () => {
  const { ctx } = await render({ features: (h) => ({ loyalty_member: h.get('x-prb-subject') === 'v1' }) });
  assert.deepEqual(ctx.features, { loyalty_member: true });
});

test('next_no_features_without_a_features_callback', async () => {
  const { ctx, manifests } = await render({});
  assert.equal(ctx.features, undefined);
  // Read once all the same, for the beacon's cookie rule (G1), never for features.
  assert.equal(manifests, 1);
});

test('next_empty_features_send_none', async () => {
  const { ctx, manifests } = await render({ features: on({}) });
  assert.equal(ctx.features, undefined);
  // Read once all the same, for the beacon's cookie rule (G1), never for features.
  assert.equal(manifests, 1);
});

test('next_unreadable_manifest_sends_no_features_and_still_decides', async () => {
  const { ctx, variant } = await render({ manifestStatus: 500, features: on({ loyalty_member: true }) });
  assert.equal(ctx.features, undefined);
  assert.equal(variant, 'gift');
});

test('next_throwing_features_callback_sends_none_and_still_decides', async () => {
  const { ctx, variant } = await render({ features: () => { throw new Error('boom'); } });
  assert.equal(ctx.features, undefined);
  assert.equal(variant, 'gift');
});

test('next_manifest_is_fetched_once_across_requests', async () => {
  let manifests = 0;
  const fetch = (async (url: string, init: any) => {
    if (url.includes('/manifest/')) {
      manifests += 1;
      return Response.json({ site: 'shop', privacy_mode: 'full', slots: [], features: DECLARED });
    }
    const body = JSON.parse(init.body);
    return Response.json({ decisions: body.slots.map((slot: string) => ({ decision_id: 'd', slot, variant: 'gift', payload: null, explanation: {} })) });
  }) as unknown as typeof globalThis.fetch;
  const { Slot } = createSlot({ core: 'http://core', site: 'shop', fetch, headers: async () => new Headers({ 'x-prb-subject': 'v1', 'x-prb-ctx': 'r1m0a0' }), requestScope: () => ({}), features: on({ loyalty_member: true }) });
  await Slot({ id: 'hero' });
  await Slot({ id: 'hero' });
  assert.equal(manifests, 1);
});
