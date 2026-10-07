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

// --- IAB TCF v2: purpose 1 ("store and/or access information on a device") -------------------

/** A CMP stub. `later`: answer after the page has read consent, as a real CMP often does. */
const cmp = (data, later = false) => (w) => {
  w.__tcfapi = (cmd, version, cb) => {
    if (cmd !== 'addEventListener' || version !== 2) return;
    const answer = () => cb(Object.assign({ gdprApplies: true, eventStatus: 'tcloaded', listenerId: 1 }, data), true);
    if (later) setTimeout(answer, 1); else answer();
  };
};
const purposeOne = (granted, extra = {}) => Object.assign({ purpose: { consents: { 1: granted, 2: true } } }, extra);

test('consent_aware_tcf_purpose_one_granted_sets_cookie', async () => {
  const p = load({ manifest: withMode('consent_aware'), before: cmp(purposeOne(true)) });
  await sleep(5);
  assert.match(p.w.document.cookie, /_prb_subject=/);
});

test('consent_aware_tcf_purpose_one_denied_keeps_session_whatever_google_says', async () => {
  // The IAB signal is the visitor's own answer to the question this cookie asks.
  const p = load({
    manifest: withMode('consent_aware'),
    before: (w) => { dataLayer(['consent', 'update', { analytics_storage: 'granted' }])(w); cmp(purposeOne(false))(w); },
  });
  await sleep(5);
  assert.equal(p.w.document.cookie, '');
});

test('consent_aware_tcf_where_gdpr_does_not_apply_falls_through', async () => {
  const p = load({
    manifest: withMode('consent_aware'),
    before: (w) => { dataLayer(['consent', 'update', { analytics_storage: 'granted' }])(w); cmp(purposeOne(false, { gdprApplies: false }))(w); },
  });
  await sleep(5);
  assert.match(p.w.document.cookie, /_prb_subject=/);
});

test('consent_aware_tcf_banner_still_showing_is_no_answer', async () => {
  const p = load({ manifest: withMode('consent_aware'), before: cmp(purposeOne(true, { eventStatus: 'cmpuishown' })) });
  await sleep(5);
  assert.equal(p.w.document.cookie, '');
});

test('consent_aware_tcf_answer_that_arrives_late_counts_from_the_next_page', async () => {
  const first = load({ manifest: withMode('consent_aware'), before: cmp(purposeOne(true), true) });
  await sleep(10);
  assert.equal(first.w.document.cookie, '', 'this page already read consent');
  const kept = first.w.sessionStorage.getItem('prb_tcf');
  const next = load({ manifest: withMode('consent_aware'), before: (w) => { w.sessionStorage.setItem('prb_tcf', kept); } });
  await sleep(5);
  assert.match(next.w.document.cookie, /_prb_subject=/);
});

test('consent_aware_tcf_late_refusal_is_kept_for_the_next_page', async () => {
  const first = load({ manifest: withMode('consent_aware'), before: cmp(purposeOne(false), true) });
  await sleep(10);
  const kept = first.w.sessionStorage.getItem('prb_tcf');
  const next = load({
    manifest: withMode('consent_aware'),
    before: (w) => { w.sessionStorage.setItem('prb_tcf', kept); w.OptanonActiveGroups = ',C0001,C0002,'; },
  });
  await sleep(5);
  assert.equal(next.w.document.cookie, '');
});

test('consent_aware_tcf_api_that_throws_falls_through', async () => {
  const p = load({
    manifest: withMode('consent_aware'),
    before: (w) => { w.__tcfapi = () => { throw new Error('cmp broke'); }; w.OptanonActiveGroups = ',C0002,'; },
  });
  await sleep(5);
  assert.match(p.w.document.cookie, /_prb_subject=/);
});
