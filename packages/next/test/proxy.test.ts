import { test } from 'node:test';
import assert from 'node:assert/strict';
import { NextRequest } from 'next/server.js';
import { createProxy } from '../src/proxy.ts';

function req(url: string, cookie = '', ua = 'Mozilla/5.0 (Macintosh)') {
  return new NextRequest(url, { headers: { cookie, 'user-agent': ua } });
}

/** What the render will see: Next carries overridden request headers this way. */
function forwarded(res: Response, name: string) {
  return res.headers.get(`x-middleware-request-${name}`);
}

test('new_visitor_gets_a_session_cookie', () => {
  const res = createProxy()(req('https://shop.test/'));
  const c = res.cookies.get('_prb_subject');
  assert.ok(c?.value, 'no subject cookie');
  assert.equal(c.maxAge, undefined, 'a persistent cookie was set without opt-in');
  assert.equal(c.httpOnly, true);
});

test('persist_opt_in_sets_a_year_long_cookie', () => {
  const res = createProxy({ persist: true })(req('https://shop.test/'));
  assert.equal(res.cookies.get('_prb_subject')?.maxAge, 60 * 60 * 24 * 365);
});

test('subject_is_forwarded_to_the_render', () => {
  const res = createProxy()(req('https://shop.test/', '_prb_subject=abc-123'));
  assert.equal(forwarded(res, 'x-prb-subject'), 'abc-123');
});

test('returning_when_a_persisted_cookie_predates_the_visit', () => {
  const res = createProxy({ persist: true })(req('https://shop.test/', '_prb_subject=abc-123'));
  assert.equal(forwarded(res, 'x-prb-ctx'), 'r1m0a0');
});

test('not_returning_without_persistence', () => {
  // A session cookie cannot outlive the visit, so it says nothing about a previous one.
  const res = createProxy()(req('https://shop.test/', '_prb_subject=abc-123'));
  assert.equal(forwarded(res, 'x-prb-ctx'), 'r0m0a0');
});

test('visit_context_is_fixed_by_the_session_cookie', () => {
  // Second page of a visit that started from an ad on mobile: the cookie wins over this URL.
  const res = createProxy({ persist: true })(req('https://shop.test/cart', '_prb_subject=abc; _prb_s=r0m1a1'));
  assert.equal(forwarded(res, 'x-prb-ctx'), 'r0m1a1');
});

test('mobile_from_user_agent', () => {
  const res = createProxy()(req('https://shop.test/', '', 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0) Mobile/15E148'));
  assert.equal(forwarded(res, 'x-prb-ctx'), 'r0m1a0');
});

test('from_ad_from_campaign_params', () => {
  const res = createProxy()(req('https://shop.test/?utm_source=newsletter'));
  assert.equal(forwarded(res, 'x-prb-ctx'), 'r0m0a1');
});

test('invalid_subject_cookie_is_replaced', () => {
  const res = createProxy()(req('https://shop.test/', '_prb_subject=' + 'x'.repeat(300)));
  const s = forwarded(res, 'x-prb-subject')!;
  assert.ok(s.length <= 256 && s !== 'x'.repeat(300));
});

test('probe_header_names_the_sdk', () => {
  const res = createProxy()(req('https://shop.test/'));
  assert.equal(res.headers.get('x-prb-sdk'), 'next/0.1.0');
});
