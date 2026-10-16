// The visitor's consent on the edge path: read from cookies (or the owner's override), sent to
// Core as `ctx.consent`, and handed to the beacon so its events carry the same answer.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';
import { createEdge } from '../src/index.ts';
import { BEACON_JS } from '../src/beacon.ts';
import { tcString } from './tcf.ts';

const PAGE = '<html><body><!-- prb:slot="hero" --><h1>Espresso</h1><!-- /prb:slot --></body></html>';

async function run(cookie?: string, extra = {}) {
  const calls: any[] = [];
  const fetch = (async (_url: string, init: any) => {
    calls.push(JSON.parse(init.body));
    return Response.json({ decision_id: 'd-1', slot: 'hero', variant: 'gift', payload: '<h1>Gifts</h1>', explanation: {} });
  }) as unknown as typeof globalThis.fetch;
  const origin = async () => new Response(PAGE, { headers: { 'content-type': 'text/html' } });
  const req = new Request('https://shop.test/', { headers: cookie ? { cookie } : {} });
  const html = await (await createEdge({ core: 'http://core', site: 'shop', origin, fetch, ...extra }).handleRequest(req)).text();
  return { consent: calls[0].ctx.consent, html };
}
const sent = async (cookie?: string, extra = {}) => (await run(cookie, extra)).consent;

test('edge_consent_granted_cookie_is_sent_as_yes', async () => {
  assert.equal(await sent('_prb_consent=granted'), true);
});

test('edge_consent_denied_cookie_is_sent_as_no', async () => {
  assert.equal(await sent('_prb_consent=denied; OptanonConsent=groups%3DC0002%3A1'), false);
});

test('edge_consent_from_tcf_purpose_one', async () => {
  assert.equal(await sent(`euconsent-v2=${tcString(true)}`), true);
});

test('edge_consent_tcf_without_purpose_one_is_no', async () => {
  assert.equal(await sent(`euconsent-v2=${tcString(false)}; OptanonConsent=groups%3DC0002%3A1`), false);
});

test('edge_consent_from_onetrust_performance_group', async () => {
  assert.equal(await sent('OptanonConsent=isGpcEnabled=0&groups=C0001%3A1%2CC0002%3A1'), true);
});

test('edge_consent_onetrust_without_performance_group_is_no', async () => {
  assert.equal(await sent('OptanonConsent=groups=C0001%3A1%2CC0002%3A0'), false);
});

test('edge_consent_defaults_to_no', async () => {
  assert.equal(await sent(), false);
});

test('edge_consent_option_overrides_the_cookies', async () => {
  assert.equal(await sent('_prb_consent=granted', { consent: false }), false);
});

test('edge_consent_option_true_grants_without_cookies', async () => {
  assert.equal(await sent(undefined, { consent: true }), true);
});

test('edge_consent_option_function_undefined_falls_through_to_the_cookies', async () => {
  assert.equal(await sent('_prb_consent=1', { consent: () => undefined }), true);
});

test('edge_hands_the_beacon_the_same_consent', async () => {
  const { html } = await run('_prb_consent=granted');
  assert.match(html, /data-prb='[^']*"consent":true/);
});

function load(consent: boolean | undefined) {
  const posted: any[] = [];
  const cfg = JSON.stringify({ site: 'shop', subject: 'v1', consent, decisions: [{ slot: 'hero', id: 'd-1' }] });
  new JSDOM(`<!doctype html><html><body><!-- prb:slot="hero" --><h1>Gifts</h1><!-- /prb:slot --><script data-prb='${cfg}'>${BEACON_JS}</script></body></html>`, {
    runScripts: 'dangerously',
    url: 'https://shop.test/',
    beforeParse(w: any) {
      w.fetch = (_url: string, init: any) => {
        posted.push(JSON.parse(init.body));
        return Promise.resolve();
      };
      w.crypto.randomUUID ??= () => 'id';
    },
  });
  return posted;
}

test('edge_beacon_events_carry_consent', async () => {
  const posted = load(true);
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(posted[0].consent, true);
});

test('edge_beacon_events_without_consent_say_so', async () => {
  const posted = load(undefined);
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(posted[0].consent, false);
});

// --- the zone cookie is written only when the site may remember the visitor (G1) -------------

/** The `_prb_tz` cookie strings beacon.js writes, attributes included, and the cookie after. */
async function tzWrites(cfgExtra: { consent?: boolean; remember?: boolean }, existing = ''): Promise<{ writes: string[]; after: string }> {
  const writes: string[] = [];
  const cfg = JSON.stringify({ site: 'shop', subject: 'v1', ...cfgExtra, decisions: [{ slot: 'hero', id: 'd-1' }] });
  const dom = new JSDOM(`<!doctype html><html><body><!-- prb:slot="hero" --><h1>Gifts</h1><!-- /prb:slot --><script data-prb='${cfg}'>${BEACON_JS}</script></body></html>`, {
    runScripts: 'dangerously',
    url: 'https://shop.test/',
    beforeParse(w: any) {
      w.fetch = () => Promise.resolve();
      w.crypto.randomUUID ??= () => 'id';
      const desc = Object.getOwnPropertyDescriptor(w.Document.prototype, 'cookie')!;
      if (existing) desc.set!.call(w.document, existing);
      Object.defineProperty(w.document, 'cookie', {
        configurable: true,
        get: () => desc.get!.call(w.document),
        set: (v: string) => {
          writes.push(v);
          desc.set!.call(w.document, v);
        },
      });
    },
  });
  await new Promise((r) => setTimeout(r, 20));
  return { writes: writes.filter((v) => v.startsWith('_prb_tz=')), after: dom.window.document.cookie };
}

const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
const yearCookie = `_prb_tz=${encodeURIComponent(zone)}; Max-Age=31536000; Path=/; Secure`;

test('edge_tz_cookie_is_persistent_when_the_site_may_remember', async () => {
  const { writes } = await tzWrites({ consent: true, remember: true });
  assert.equal(writes.length, 1, JSON.stringify(writes));
  assert.match(writes[0], /; Max-Age=31536000/);
});

test('edge_tz_cookie_is_not_written_when_the_site_may_not_remember', async () => {
  const { writes, after } = await tzWrites({ consent: false, remember: false });
  assert.deepEqual(writes, []);
  assert.doesNotMatch(after, /_prb_tz=/);
});

test('edge_tz_cookie_follows_remember_not_bare_consent', async () => {
  // A cookieless site with a consenting visitor: consent is true, the site still stores nothing.
  const { writes } = await tzWrites({ consent: true });
  assert.deepEqual(writes, []);
});

test('edge_tz_cookie_is_expired_when_remembering_stops', async () => {
  const { writes, after } = await tzWrites({ consent: false, remember: false }, yearCookie);
  assert.equal(writes.length, 1, JSON.stringify(writes));
  assert.match(writes[0], /; Max-Age=0/);
  assert.doesNotMatch(after, /_prb_tz=/);
});

// --- the edge hands the beacon `remember` from the manifest's privacy mode ---------------------

async function remembered(mode: string | null, cookie?: string): Promise<unknown> {
  const fetch = (async (url: string) => {
    if (url.includes('/manifest/')) {
      if (mode === null) return new Response('down', { status: 503 });
      return Response.json({ site: 'shop', privacy_mode: mode, slots: [], features: [] });
    }
    return Response.json({ decision_id: 'd-1', slot: 'hero', variant: 'gift', payload: '<h1>Gifts</h1>', explanation: {} });
  }) as unknown as typeof globalThis.fetch;
  const origin = async () => new Response(PAGE, { headers: { 'content-type': 'text/html' } });
  const req = new Request('https://shop.test/', { headers: cookie ? { cookie } : {} });
  const html = await (await createEdge({ core: 'http://core', site: 'shop', origin, fetch }).handleRequest(req)).text();
  const m = /data-prb='([^']+)'/.exec(html);
  assert.ok(m, html);
  return JSON.parse(m[1]).remember;
}

test('edge_remember_on_a_full_site_without_consent', async () => {
  assert.equal(await remembered('full'), true);
});

test('edge_remember_on_a_consent_aware_site_with_consent', async () => {
  assert.equal(await remembered('consent_aware', '_prb_consent=granted'), true);
});

test('edge_no_remember_on_a_consent_aware_site_without_consent', async () => {
  assert.equal(await remembered('consent_aware'), false);
});

test('edge_no_remember_on_a_cookieless_site_even_with_consent', async () => {
  assert.equal(await remembered('cookieless', '_prb_consent=granted'), false);
});

test('edge_no_remember_when_the_manifest_cannot_be_read', async () => {
  assert.equal(await remembered(null, '_prb_consent=granted'), false);
});

test('edge_decision_never_waits_on_the_manifest', async () => {
  // The cookie rule's manifest read comes after the first decide, so a cold manifest cache
  // cannot hold a slot past its deadline.
  const order: string[] = [];
  const fetch = (async (url: string) => {
    order.push(url.includes('/manifest/') ? 'manifest' : 'decide');
    if (url.includes('/manifest/')) return Response.json({ site: 'shop', privacy_mode: 'full', slots: [], features: [] });
    return Response.json({ decision_id: 'd-1', slot: 'hero', variant: 'gift', payload: '<h1>Gifts</h1>', explanation: {} });
  }) as unknown as typeof globalThis.fetch;
  const origin = async () => new Response(PAGE, { headers: { 'content-type': 'text/html' } });
  await (await createEdge({ core: 'http://core', site: 'shop', origin, fetch }).handleRequest(new Request('https://shop.test/'))).text();
  assert.deepEqual(order, ['decide', 'manifest']);
});
