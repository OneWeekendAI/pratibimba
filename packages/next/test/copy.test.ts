// F4: <prb.Copy> hands its children the decided text as plain strings over the page's own, so
// a field renders escaped and a variant that leaves a field out keeps the page's text for it.
import { test } from 'node:test';
import assert from 'node:assert/strict';

const { createElement } = await import('react');
const { renderToStaticMarkup } = await import('react-dom/server');
const { createSlot, copyText } = await import('../src/slot.ts');

const visitorHeaders = async () => new Headers({ 'x-prb-subject': 'v1', 'x-prb-ctx': 'r1m0a0' });

function copyWith(answer: () => any) {
  const fetch = (async () => {
    const out = answer();
    if (out instanceof Error) throw out;
    return new Response(JSON.stringify(out), { status: 200, headers: { 'content-type': 'application/json' } });
  }) as unknown as typeof globalThis.fetch;
  return createSlot({ core: 'http://core', site: 'shop', fetch, headers: visitorHeaders, requestScope: () => ({}) }).Copy;
}

const decision = (variant: string, copy: Record<string, string> | null) => ({
  decisions: [{ decision_id: 'd-hero', slot: 'hero', variant, payload: null, copy, explanation: {} }],
});
const fallback = { headline: 'Our own headline', cta_label: 'Shop now' };
const hero = (t: Record<string, string>) => createElement('section', null, createElement('h1', null, t.headline), createElement('a', null, t.cta_label));

test('copy_renders_the_decided_fields', async () => {
  const Copy = copyWith(() => decision('bold', { headline: 'Ships tonight', cta_label: 'Buy' }));
  const html = renderToStaticMarkup(await Copy({ id: 'hero', fallback, children: hero }));
  assert.match(html, /<h1>Ships tonight<\/h1>/);
  assert.match(html, /<a>Buy<\/a>/);
  assert.match(html, /data-prb-decision="d-hero"/);
});

test('copy_keeps_the_page_text_for_a_field_the_variant_leaves_out', async () => {
  const Copy = copyWith(() => decision('bold', { headline: 'Ships tonight' }));
  const html = renderToStaticMarkup(await Copy({ id: 'hero', fallback, children: hero }));
  assert.match(html, /<a>Shop now<\/a>/);
});

test('copy_baseline_renders_the_page_text', async () => {
  const Copy = copyWith(() => decision('own', null));
  const html = renderToStaticMarkup(await Copy({ id: 'hero', fallback, children: hero }));
  assert.match(html, /<h1>Our own headline<\/h1>/);
});

test('copy_text_is_escaped_never_html', async () => {
  // Core refuses < and >; this is the SDK's own line if a field ever carried markup anyway.
  const Copy = copyWith(() => decision('bad', { headline: '<img src=x onerror=alert(1)>' }));
  const html = renderToStaticMarkup(await Copy({ id: 'hero', fallback, children: hero }));
  assert.doesNotMatch(html, /<img/);
  assert.match(html, /&lt;img/);
});

test('copy_on_a_failed_decide_renders_the_page_text', async () => {
  const Copy = copyWith(() => new Error('core down'));
  const html = renderToStaticMarkup(await Copy({ id: 'hero', fallback, children: hero }));
  assert.match(html, /<h1>Our own headline<\/h1>/);
  assert.doesNotMatch(html, /data-prb-decision/);
});

test('copy_text_ignores_undeclared_and_non_string_fields', () => {
  assert.deepEqual(copyText(fallback, { headline: 'X', extra: 'Y', cta_label: 7 }), { headline: 'X', cta_label: 'Shop now' });
});
