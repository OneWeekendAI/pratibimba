// The visitor's time zone on the server path. A server render has no browser clock, so the
// proxy names the zone from the cookie the browser half sets, or else from the edge's IP
// geolocation header, and the slot sends it to Core as `ctx.tz`.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { NextRequest } from 'next/server.js';
import { createProxy } from '../src/proxy.ts';
import { createSlot } from '../src/slot.ts';

function tzReq(headers: Record<string, string>) {
  return new NextRequest('https://shop.test/', { headers: { 'user-agent': 'Mozilla/5.0', ...headers } });
}

function forwarded(res: Response, name: string) {
  return res.headers.get(`x-middleware-request-${name}`);
}

test('tz_from_the_vercel_geo_header_is_forwarded_to_the_render', () => {
  const res = createProxy()(tzReq({ 'x-vercel-ip-timezone': 'Asia/Kolkata' }));
  assert.equal(forwarded(res, 'x-prb-tz'), 'Asia/Kolkata');
});

test('tz_from_the_cloudflare_geo_header_is_forwarded_to_the_render', () => {
  const res = createProxy()(tzReq({ 'cf-timezone': 'Europe/London' }));
  assert.equal(forwarded(res, 'x-prb-tz'), 'Europe/London');
});

test('tz_from_the_cloudfront_geo_header_is_forwarded_to_the_render', () => {
  const res = createProxy()(tzReq({ 'cloudfront-viewer-time-zone': 'Australia/Sydney' }));
  assert.equal(forwarded(res, 'x-prb-tz'), 'Australia/Sydney');
});

test('tz_falls_back_to_the_cookie_the_browser_set', () => {
  const res = createProxy()(tzReq({ cookie: '_prb_tz=America%2FNew_York' }));
  assert.equal(forwarded(res, 'x-prb-tz'), 'America/New_York');
});

test('tz_cookie_wins_over_the_geo_header', () => {
  // The browser names its own zone; IP geolocation is wrong on some carriers and every VPN.
  const res = createProxy()(tzReq({ 'x-vercel-ip-timezone': 'Asia/Tokyo', cookie: '_prb_tz=America%2FNew_York' }));
  assert.equal(forwarded(res, 'x-prb-tz'), 'America/New_York');
});

test('tz_that_is_not_a_real_zone_is_not_forwarded', () => {
  const res = createProxy()(tzReq({ 'x-vercel-ip-timezone': 'Mars/Olympus_Mons', cookie: '_prb_tz=%3Cscript%3E' }));
  assert.equal(forwarded(res, 'x-prb-tz'), null);
});

test('tz_bad_cookie_falls_through_to_the_geo_header', () => {
  const res = createProxy()(tzReq({ 'x-vercel-ip-timezone': 'Europe/Paris', cookie: '_prb_tz=Mars%2FOlympus_Mons' }));
  assert.equal(forwarded(res, 'x-prb-tz'), 'Europe/Paris');
});

test('tz_bad_geo_header_falls_through_to_the_next_header', () => {
  const res = createProxy()(tzReq({ 'x-vercel-ip-timezone': 'Mars/Olympus_Mons', 'cf-timezone': 'Europe/Paris' }));
  assert.equal(forwarded(res, 'x-prb-tz'), 'Europe/Paris');
});

test('tz_request_header_sent_by_the_client_is_dropped', () => {
  // Only the proxy names the zone; a visitor cannot hand the render one directly.
  const res = createProxy()(tzReq({ 'x-prb-tz': 'Asia/Tokyo' }));
  assert.equal(forwarded(res, 'x-prb-tz'), null);
});

test('tz_geo_headers_can_be_switched_off', () => {
  // Off a platform that sets them, a geo header is just something the client sent.
  const res = createProxy({ timeZoneHeaders: [] })(tzReq({ 'x-vercel-ip-timezone': 'Asia/Tokyo' }));
  assert.equal(forwarded(res, 'x-prb-tz'), null);
});

// --- the slot sends it ------------------------------------------------------------------------

type Call = { url: string; body: any };

function core(calls: Call[]) {
  return (async (url: string, init: any) => {
    const body = JSON.parse(init.body);
    calls.push({ url, body });
    const decisions = body.slots.map((slot: string) => ({ decision_id: `d-${slot}`, slot, variant: 'gift', payload: null, explanation: {} }));
    return new Response(JSON.stringify({ decisions }), { headers: { 'content-type': 'application/json' } });
  }) as unknown as typeof fetch;
}

async function sentCtx(headers: Record<string, string>) {
  const calls: Call[] = [];
  const { Slot } = createSlot({ core: 'http://core', site: 'shop', fetch: core(calls), headers: async () => new Headers(headers), requestScope: () => ({}) });
  renderToStaticMarkup(await Slot({ id: 'hero', children: createElement('h1', null, 'Espresso') }));
  return calls[0].body.ctx;
}

test('slot_sends_the_visitors_time_zone_to_core', async () => {
  const ctx = await sentCtx({ 'x-prb-subject': 'v1', 'x-prb-ctx': 'r1m0a0', 'x-prb-tz': 'Asia/Kolkata' });
  assert.deepEqual(ctx, { returning: true, mobile: false, from_ad: false, tz: 'Asia/Kolkata' });
});

test('slot_sends_no_time_zone_when_none_is_known', async () => {
  const ctx = await sentCtx({ 'x-prb-subject': 'v1', 'x-prb-ctx': 'r1m0a0' });
  assert.deepEqual(ctx, { returning: true, mobile: false, from_ad: false });
});
