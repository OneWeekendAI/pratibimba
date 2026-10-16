// Declared features on the edge path: the owner's `features` callback names them per request;
// the edge sends as ctx.features only names the manifest declares, only booleans, and a pii
// one only when the visitor is identified (full, or consent_aware with consent).

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createEdge, type EdgeOptions } from '../src/index.ts';

const PAGE = '<html><body><!-- prb:slot="hero" --><h1>Espresso</h1><!-- /prb:slot --></body></html>';
const DECLARED = [{ key: 'corp_email', pii: true }, { key: 'loyalty_member', pii: false }];

function edge(opts: { mode?: string; declared?: unknown; manifestStatus?: number; features?: EdgeOptions['features'] }, seen: { decides: any[]; manifests: number }) {
  const fetch = (async (url: string, init: any) => {
    if (url.includes('/manifest/')) {
      seen.manifests += 1;
      if (opts.manifestStatus) return Response.json({ site: 'shop', privacy_mode: 'full', slots: [], features: DECLARED }, { status: opts.manifestStatus });
      return Response.json({ site: 'shop', privacy_mode: opts.mode ?? 'full', slots: [], ...(opts.declared === undefined ? { features: DECLARED } : opts.declared ? { features: opts.declared } : {}) });
    }
    seen.decides.push(JSON.parse(init.body));
    return Response.json({ decision_id: 'd-1', slot: 'hero', variant: 'gift', payload: '<h1>Gifts</h1>', explanation: {} });
  }) as unknown as typeof globalThis.fetch;
  const origin = async () => new Response(PAGE, { headers: { 'content-type': 'text/html' } });
  return createEdge({ core: 'http://core', site: 'shop', origin, fetch, features: opts.features });
}

async function run(opts: Parameters<typeof edge>[0] & { cookie?: string }) {
  const seen = { decides: [] as any[], manifests: 0 };
  const req = new Request('https://shop.test/', { headers: opts.cookie ? { cookie: opts.cookie } : {} });
  const html = await (await edge(opts, seen).handleRequest(req)).text();
  return { ctx: seen.decides[0].ctx, manifests: seen.manifests, html };
}
const on = (v: Record<string, unknown>) => () => v;

test('edge_non_pii_feature_is_sent_without_consent', async () => {
  const { ctx } = await run({ mode: 'consent_aware', features: on({ loyalty_member: true }) });
  assert.deepEqual(ctx.features, { loyalty_member: true });
});

test('edge_pii_feature_is_not_sent_in_consent_aware_mode_without_consent', async () => {
  const { ctx } = await run({ mode: 'consent_aware', cookie: '_prb_consent=denied', features: on({ corp_email: true, loyalty_member: false }) });
  assert.deepEqual(ctx.features, { loyalty_member: false });
});

test('edge_pii_feature_is_sent_in_consent_aware_mode_with_consent', async () => {
  const { ctx } = await run({ mode: 'consent_aware', cookie: '_prb_consent=granted', features: on({ corp_email: true }) });
  assert.deepEqual(ctx.features, { corp_email: true });
});

test('edge_pii_feature_is_not_sent_in_cookieless_mode_even_with_consent', async () => {
  const { ctx } = await run({ mode: 'cookieless', cookie: '_prb_consent=granted', features: on({ corp_email: true }) });
  assert.equal(ctx.features, undefined);
});

test('edge_pii_feature_is_sent_in_full_mode', async () => {
  const { ctx } = await run({ mode: 'full', features: on({ corp_email: true }) });
  assert.deepEqual(ctx.features, { corp_email: true });
});

test('edge_undeclared_feature_is_not_sent', async () => {
  const { ctx } = await run({ features: on({ age_band: true, loyalty_member: true }) });
  assert.deepEqual(ctx.features, { loyalty_member: true });
});

test('edge_feature_with_a_non_boolean_value_is_not_sent', async () => {
  const { ctx } = await run({ features: on({ loyalty_member: 'yes', corp_email: 1 }) });
  assert.equal(ctx.features, undefined);
});

test('edge_feature_is_not_sent_when_the_manifest_declares_none', async () => {
  const { ctx } = await run({ declared: null, features: on({ loyalty_member: true }) });
  assert.equal(ctx.features, undefined);
});

test('edge_async_features_callback_is_awaited', async () => {
  const { ctx } = await run({ features: async () => ({ loyalty_member: true }) });
  assert.deepEqual(ctx.features, { loyalty_member: true });
});

test('edge_features_callback_reads_the_request', async () => {
  const { ctx } = await run({ cookie: 'member=1', features: (r) => ({ loyalty_member: (r.headers.get('cookie') ?? '').includes('member=1') }) });
  assert.deepEqual(ctx.features, { loyalty_member: true });
});

test('edge_no_features_without_a_features_callback', async () => {
  const { ctx, manifests } = await run({});
  assert.equal(ctx.features, undefined);
  // Read once all the same, after the decision, for the beacon's cookie rule (G1).
  assert.equal(manifests, 1);
});

test('edge_empty_features_send_none', async () => {
  const { ctx, manifests } = await run({ features: on({}) });
  assert.equal(ctx.features, undefined);
  // Read once all the same, after the decision, for the beacon's cookie rule (G1).
  assert.equal(manifests, 1);
});

test('edge_unreadable_manifest_sends_no_features_and_still_decides', async () => {
  const { ctx, html } = await run({ manifestStatus: 500, features: on({ loyalty_member: true }) });
  assert.equal(ctx.features, undefined);
  assert.match(html, /Gifts/);
});

test('edge_throwing_features_callback_sends_none_and_still_decides', async () => {
  const { ctx, html } = await run({ features: () => { throw new Error('boom'); } });
  assert.equal(ctx.features, undefined);
  assert.match(html, /Gifts/);
});

test('edge_manifest_is_fetched_once_across_requests', async () => {
  const seen = { decides: [] as any[], manifests: 0 };
  const prb = edge({ features: on({ loyalty_member: true }) }, seen);
  await (await prb.handleRequest(new Request('https://shop.test/'))).text();
  await (await prb.handleRequest(new Request('https://shop.test/'))).text();
  assert.equal(seen.manifests, 1);
});
