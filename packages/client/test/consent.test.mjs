// Consent modes in the script: which storage the subject lives in, and what Core is told.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { load, sleep, MANIFEST } from './harness.mjs';

const withMode = (mode) => Object.assign({}, MANIFEST, { privacy_mode: mode });
const dataLayer = (...entries) => (w) => { w.dataLayer = entries; };

test('full_mode_sets_cookie_without_attribute', async () => {
  const p = load({ manifest: withMode('full') });
  await sleep(5);
  assert.match(p.w.document.cookie, /_prb_subject=/);
});

test('cookieless_mode_never_sets_cookie_even_with_attribute', async () => {
  const p = load({ manifest: withMode('cookieless'), attrs: { 'data-cookie': 'on' } });
  await sleep(5);
  assert.equal(p.w.document.cookie, '');
  assert.ok(p.w.sessionStorage.getItem('prb_subject'));
});

test('consent_aware_without_signal_keeps_session_storage', async () => {
  const p = load({ manifest: withMode('consent_aware') });
  await sleep(5);
  assert.equal(p.w.document.cookie, '');
});

test('consent_aware_google_consent_granted_sets_cookie', async () => {
  const p = load({ manifest: withMode('consent_aware'), before: dataLayer(['consent', 'update', { analytics_storage: 'granted' }]) });
  await sleep(5);
  assert.match(p.w.document.cookie, /_prb_subject=/);
});

test('consent_aware_latest_google_consent_wins', async () => {
  const p = load({
    manifest: withMode('consent_aware'),
    before: dataLayer(['consent', 'default', { analytics_storage: 'granted' }], ['consent', 'update', { analytics_storage: 'denied' }]),
  });
  await sleep(5);
  assert.equal(p.w.document.cookie, '');
});

test('consent_aware_onetrust_performance_group_sets_cookie', async () => {
  const p = load({ manifest: withMode('consent_aware'), before: (w) => { w.OptanonActiveGroups = ',C0001,C0002,'; } });
  await sleep(5);
  assert.match(p.w.document.cookie, /_prb_subject=/);
});

test('consent_aware_onetrust_without_performance_keeps_session', async () => {
  const p = load({ manifest: withMode('consent_aware'), before: (w) => { w.OptanonActiveGroups = ',C0001,'; } });
  await sleep(5);
  assert.equal(p.w.document.cookie, '');
});

test('decide_says_whether_the_visitor_consented', async () => {
  const yes = load({ manifest: withMode('full') });
  const no = load({ manifest: withMode('consent_aware') });
  await sleep(5);
  assert.equal(yes.decides()[0].body.ctx.consent, true);
  assert.equal(no.decides()[0].body.ctx.consent, false);
});

test('events_say_whether_the_visitor_consented', async () => {
  const p = load({ manifest: withMode('full') });
  await sleep(20);
  const batches = p.calls.filter((c) => c.url.endsWith('/e'));
  assert.ok(batches.length > 0, 'an exposure was sent');
  assert.ok(batches.every((c) => c.body.consent === true));
});
