// The visitor's consent on the server path. The proxy reads it from cookies (or the owner's
// override) and hands it to the render; the slot sends it to Core as `ctx.consent`, and the
// beacon's events carry the same answer.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { NextRequest } from 'next/server.js';
import { createProxy } from '../src/proxy.ts';
import { createSlot } from '../src/slot.ts';
import { tcString } from './tcf.ts';

function req(cookie?: string, headers: Record<string, string> = {}) {
  return new NextRequest('https://shop.test/', { headers: { 'user-agent': 'Mozilla/5.0', ...(cookie ? { cookie } : {}), ...headers } });
}
const forwarded = (res: Response) => res.headers.get('x-middleware-request-x-prb-consent');

test('consent_granted_cookie_is_forwarded_as_yes', () => {
  assert.equal(forwarded(createProxy()(req('_prb_consent=granted'))), '1');
});

test('consent_denied_cookie_is_forwarded_as_no', () => {
  assert.equal(forwarded(createProxy()(req('_prb_consent=denied; OptanonConsent=groups%3DC0001%3A1%2CC0002%3A1'))), '0');
});

test('consent_from_tcf_purpose_one', () => {
  assert.equal(forwarded(createProxy()(req(`euconsent-v2=${tcString(true)}`))), '1');
});

test('consent_tcf_without_purpose_one_is_no', () => {
  assert.equal(forwarded(createProxy()(req(`euconsent-v2=${tcString(false)}; OptanonConsent=groups%3DC0002%3A1`))), '0');
});

test('consent_tcf_string_of_another_version_is_ignored', () => {
  assert.equal(forwarded(createProxy()(req(`euconsent-v2=${tcString(false, 1)}; OptanonConsent=groups%3DC0002%3A1`))), '1');
});

test('consent_from_onetrust_performance_group', () => {
  assert.equal(forwarded(createProxy()(req('OptanonConsent=isGpcEnabled=0&groups=C0001%3A1%2CC0002%3A1&AwaitingReconsent=false'))), '1');
});

test('consent_onetrust_without_performance_group_is_no', () => {
  assert.equal(forwarded(createProxy()(req('OptanonConsent=groups=C0001%3A1%2CC0002%3A0'))), '0');
});

test('consent_defaults_to_no', () => {
  assert.equal(forwarded(createProxy()(req())), '0');
});

test('consent_header_sent_by_the_client_is_replaced', () => {
  assert.equal(forwarded(createProxy()(req(undefined, { 'x-prb-consent': '1' }))), '0');
});

test('consent_option_overrides_the_cookies', () => {
  assert.equal(forwarded(createProxy({ consent: false })(req('_prb_consent=granted'))), '0');
});

test('consent_option_true_grants_without_cookies', () => {
  assert.equal(forwarded(createProxy({ consent: true })(req())), '1');
});

test('consent_option_function_reads_the_cookies', () => {
  const consent = (c: { get(n: string): string | undefined }) => (c.get('my_cmp') === 'yes' ? true : undefined);
  assert.equal(forwarded(createProxy({ consent })(req('my_cmp=yes'))), '1');
});

test('consent_option_function_undefined_falls_through_to_the_cookies', () => {
  assert.equal(forwarded(createProxy({ consent: () => undefined })(req('_prb_consent=1'))), '1');
});

// --- the slot sends it ------------------------------------------------------------------------

async function render(headers: Record<string, string>) {
  const calls: any[] = [];
  const fetch = (async (_url: string, init: any) => {
    const body = JSON.parse(init.body);
    calls.push(body);
    const decisions = body.slots.map((slot: string) => ({ decision_id: `d-${slot}`, slot, variant: 'gift', payload: null, explanation: {} }));
    return Response.json({ decisions });
  }) as unknown as typeof globalThis.fetch;
  const { Slot } = createSlot({ core: 'http://core', site: 'shop', fetch, headers: async () => new Headers(headers), requestScope: () => ({}) });
  const el: any = await Slot({ id: 'hero', children: createElement('h1', null, 'Espresso') });
  return { ctx: calls[0].ctx, beacon: el.props.children[1].props };
}

test('slot_sends_consent_to_core', async () => {
  const { ctx } = await render({ 'x-prb-subject': 'v1', 'x-prb-ctx': 'r1m0a0', 'x-prb-consent': '1' });
  assert.equal(ctx.consent, true);
});

test('slot_sends_no_consent_as_false', async () => {
  const { ctx } = await render({ 'x-prb-subject': 'v1', 'x-prb-ctx': 'r1m0a0', 'x-prb-consent': '0' });
  assert.equal(ctx.consent, false);
});

test('slot_hands_the_beacon_the_same_consent', async () => {
  const { beacon } = await render({ 'x-prb-subject': 'v1', 'x-prb-ctx': 'r1m0a0', 'x-prb-consent': '1' });
  assert.equal(beacon.consent, true);
});

test('track_posts_the_visitors_consent', async () => {
  const { createPratibimba } = await import('../src/index.ts');
  const posts: any[] = [];
  const fetch = (async (_url: string, init: any) => {
    posts.push(JSON.parse(init.body));
    return new Response('{}', { status: 202 });
  }) as unknown as typeof globalThis.fetch;
  const headers = async () => new Headers({ 'x-prb-subject': 'v1', 'x-prb-ctx': 'r1m0a0', 'x-prb-consent': '1' });
  await createPratibimba({ core: 'http://core', site: 'shop', fetch, headers }).track('purchase');
  assert.equal(posts[0].consent, true);
});
