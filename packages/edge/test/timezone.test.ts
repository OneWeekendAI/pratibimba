// The visitor's time zone on the edge path: the edge names it from the cookie the beacon sets,
// else from its own geolocation (the Cloudflare `request.cf` object or a platform header),
// and sends it to Core as `ctx.tz`.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';
import { createEdge } from '../src/index.ts';
import { BEACON_JS } from '../src/beacon.ts';

const PAGE = '<html><body><!-- prb:slot="hero" --><h1>Espresso</h1><!-- /prb:slot --></body></html>';

async function ctxSent(request: Request, extra = {}) {
  const calls: any[] = [];
  const fetch = (async (url: string, init: any) => {
    calls.push(JSON.parse(init.body));
    return Response.json({ decision_id: 'd-1', slot: 'hero', variant: 'gift', payload: '<h1>Gifts</h1>', explanation: {} });
  }) as unknown as typeof globalThis.fetch;
  const origin = async () => new Response(PAGE, { headers: { 'content-type': 'text/html' } });
  await (await createEdge({ core: 'http://core', site: 'shop', origin, fetch, ...extra }).handleRequest(request)).text();
  return calls[0].ctx;
}

const get = (headers: Record<string, string> = {}) => new Request('https://shop.test/', { headers });

test('edge_sends_the_vercel_geo_time_zone_to_core', async () => {
  assert.equal((await ctxSent(get({ 'x-vercel-ip-timezone': 'Asia/Kolkata' }))).tz, 'Asia/Kolkata');
});

test('edge_sends_the_cloudflare_request_cf_time_zone_to_core', async () => {
  // On Cloudflare Workers the zone is on the request itself, with no transform rule needed.
  const r = Object.assign(get(), { cf: { timezone: 'Europe/London' } });
  assert.equal((await ctxSent(r)).tz, 'Europe/London');
});

test('edge_falls_back_to_the_tz_cookie', async () => {
  assert.equal((await ctxSent(get({ cookie: '_prb_tz=America%2FNew_York' }))).tz, 'America/New_York');
});

test('edge_cookie_wins_over_the_geo_header', async () => {
  assert.equal((await ctxSent(get({ 'cf-timezone': 'Asia/Tokyo', cookie: '_prb_tz=America%2FNew_York' }))).tz, 'America/New_York');
});

test('edge_cookie_wins_over_the_cloudflare_request_cf_time_zone', async () => {
  const r = Object.assign(get({ cookie: '_prb_tz=America%2FNew_York' }), { cf: { timezone: 'Europe/London' } });
  assert.equal((await ctxSent(r)).tz, 'America/New_York');
});

test('edge_request_cf_time_zone_wins_over_a_geo_header', async () => {
  const r = Object.assign(get({ 'x-vercel-ip-timezone': 'Asia/Tokyo' }), { cf: { timezone: 'Europe/London' } });
  assert.equal((await ctxSent(r)).tz, 'Europe/London');
});

test('edge_bad_cookie_falls_through_to_the_geo_header', async () => {
  assert.equal((await ctxSent(get({ 'cf-timezone': 'Asia/Tokyo', cookie: '_prb_tz=Mars%2FOlympus_Mons' }))).tz, 'Asia/Tokyo');
});

test('edge_sends_no_time_zone_that_is_not_a_real_zone', async () => {
  const ctx = await ctxSent(get({ 'x-vercel-ip-timezone': 'Mars/Olympus_Mons', cookie: '_prb_tz=%E0%A4%A' }));
  assert.equal('tz' in ctx, false, JSON.stringify(ctx));
});

test('edge_geo_headers_can_be_switched_off', async () => {
  const ctx = await ctxSent(get({ 'x-vercel-ip-timezone': 'Asia/Tokyo' }), { timeZoneHeaders: [] });
  assert.equal('tz' in ctx, false, JSON.stringify(ctx));
});

// --- the beacon sets the cookie and reports the offset -----------------------------------------

function load() {
  const posted: any[] = [];
  const cfg = JSON.stringify({ site: 'shop', subject: 'v1', decisions: [{ slot: 'hero', id: 'd-1' }] });
  const dom = new JSDOM(`<!doctype html><html><body><!-- prb:slot="hero" --><h1>Gifts</h1><!-- /prb:slot --><script data-prb='${cfg}'>${BEACON_JS}</script></body></html>`, {
    runScripts: 'dangerously',
    url: 'https://shop.test/',
    beforeParse(w: any) {
      w.fetch = (url: string, init: any) => {
        posted.push(JSON.parse(init.body));
        return Promise.resolve();
      };
      w.crypto.randomUUID ??= () => 'id';
    },
  });
  return { dom, posted };
}

test('beacon_remembers_the_browsers_time_zone_in_a_cookie', async () => {
  const { dom } = load();
  await new Promise((r) => setTimeout(r, 20));
  const zone = dom.window.Intl.DateTimeFormat().resolvedOptions().timeZone;
  assert.ok(dom.window.document.cookie.split('; ').includes(`_prb_tz=${encodeURIComponent(zone)}`), dom.window.document.cookie);
});

test('beacon_events_carry_the_browsers_offset', async () => {
  const { posted } = load();
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(posted[0].tz_offset, -new Date().getTimezoneOffset());
});
