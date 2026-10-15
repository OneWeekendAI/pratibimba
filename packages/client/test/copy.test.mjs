// F4: a copy slot. The page marks each field with data-prb-field; the SDK writes the decided
// text as textContent, keeps the page's text for a field the variant leaves out, and never
// parses a field (or a stray payload) as HTML.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { load, ok, sleep } from './harness.mjs';

const MANIFEST = {
  site: 'demo',
  slots: [{ key: 'hero', selector: '#hero', metric: 'click', ttl_secs: 60, kind: 'copy',
            variants: [{ key: 'own', baseline: true }, { key: 'bold', baseline: false }] }],
};
const BODY = '<section id="hero"><h1 data-prb-field="headline">Our own</h1><a id="cta" data-prb-field="cta_label">Shop now</a></section>';
const decision = (copy, extra = {}) => ({ decision_id: 'd-1', slot: 'hero', variant: 'bold', payload: null, copy, explanation: {}, ...extra });
const page = (d) => load({ manifest: MANIFEST, body: BODY, decide: () => ok(d) });
const $ = (p, sel) => p.w.document.querySelector(sel);

test('copy_slot_writes_each_field_as_text', async () => {
  const p = page(decision({ headline: 'Ships tonight', cta_label: 'Buy' }));
  await sleep(20);
  assert.equal($(p, 'h1').textContent, 'Ships tonight');
  assert.equal($(p, '#cta').textContent, 'Buy');
  assert.equal($(p, '#hero').getAttribute('data-prb-variant'), 'bold');
});

test('copy_slot_keeps_the_page_text_for_a_missing_field', async () => {
  const p = page(decision({ headline: 'Ships tonight' }));
  await sleep(20);
  assert.equal($(p, '#cta').textContent, 'Shop now');
});

test('copy_field_markup_stays_text', async () => {
  const p = page(decision({ headline: '<img id="x" src="y">' }));
  await sleep(20);
  assert.equal($(p, '#x'), null);
  assert.equal($(p, 'h1').textContent, '<img id="x" src="y">');
});

test('copy_decision_ignores_a_payload', async () => {
  // Core always sends payload null for copy; copy wins even if one arrived.
  const p = page(decision({ headline: 'Ships tonight' }, { payload: '<p id="p">html</p>' }));
  await sleep(20);
  assert.equal($(p, '#p'), null);
  assert.equal($(p, 'h1').textContent, 'Ships tonight');
});

test('copy_baseline_leaves_the_page_alone', async () => {
  const p = page({ decision_id: 'd-2', slot: 'hero', variant: 'own', payload: null, copy: null, explanation: {} });
  await sleep(20);
  assert.equal($(p, 'h1').textContent, 'Our own');
});
