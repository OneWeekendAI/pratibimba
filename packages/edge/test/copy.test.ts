// F4 + S11: the marker rewriter serves a copy slot's fields, as the client and next SDKs do. A
// text field's element gets the text, escaped; an image field's <img> (or the <img> inside the
// marked <picture> or wrapper) gets src, alt and size, losing srcset, sizes and <source>s.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';
import { applyCopy } from '../src/markers.ts';
import { createEdge } from '../src/index.ts';

const ID = 'ef'.repeat(32);
const ASSETS = '/_prb/assets/shop';
const REGION = '<section><h1 class="t" data-prb-field="headline">Our <em>own</em></h1>' +
  '<picture><source srcset="/own.avif" type="image/avif"><img data-prb-field="banner" src="/own.jpg" srcset="/own-2x.jpg 2x" sizes="100vw" alt="Our own beans" width="800" height="300" class="hero"></picture>' +
  '<div data-prb-field="side"><span><img src="/side.jpg" alt="Side"></span></div><a data-prb-field="cta">Shop</a></section>';
const doc = (html: string) => new JSDOM(`<body>${html}</body>`).window.document;

test('text_fields_are_written_escaped', () => {
  const out = applyCopy(REGION, { headline: 'Fresh <b>&</b> fast', cta: 'Buy' }, ASSETS);
  const d = doc(out);
  assert.equal(d.querySelector('h1')!.textContent, 'Fresh <b>&</b> fast');
  assert.equal(d.querySelector('h1 b'), null);
  assert.equal(d.querySelector('h1')!.getAttribute('class'), 't');
  assert.equal(d.querySelector('a')!.textContent, 'Buy');
  assert.equal(d.querySelector('img[data-prb-field]')!.getAttribute('src'), '/own.jpg', 'a field left out keeps the page');
});

test('an_image_field_sets_src_alt_size_and_drops_what_would_override_it', () => {
  const out = applyCopy(REGION, { banner: { asset: ID, alt: 'Autumn "beans" <3', width: 1600, height: 500 } }, ASSETS);
  const d = doc(out);
  const img = d.querySelector('img[data-prb-field="banner"]')!;
  assert.equal(img.getAttribute('src'), `${ASSETS}/${ID}`);
  assert.equal(img.getAttribute('alt'), 'Autumn "beans" <3');
  assert.equal(img.getAttribute('width'), '1600');
  assert.equal(img.getAttribute('height'), '500');
  assert.equal(img.getAttribute('class'), 'hero', 'the page keeps its own attributes');
  assert.equal(img.hasAttribute('srcset'), false);
  assert.equal(img.hasAttribute('sizes'), false);
  assert.equal(d.querySelector('source'), null);
  assert.equal(d.querySelector('h1')!.innerHTML, 'Our <em>own</em>');
});

test('an_image_field_on_a_wrapper_sets_its_first_img', () => {
  const d = doc(applyCopy(REGION, { side: { url: 'https://cdn.example.com/s.webp', alt: 'New side' } }, ASSETS));
  const img = d.querySelector('div[data-prb-field="side"] img')!;
  assert.equal(img.getAttribute('src'), 'https://cdn.example.com/s.webp');
  assert.equal(img.getAttribute('alt'), 'New side');
  assert.equal(img.hasAttribute('width'), false);
});

test('unsafe_or_mismatched_values_change_nothing', () => {
  for (const copy of [{ banner: { url: 'javascript:alert(1)', alt: 'x' } }, { banner: { url: '//evil.example/a', alt: 'x' } },
                      { banner: { asset: '../x', alt: 'x' } }, { banner: { url: '/a.png' } }, { banner: 'text' },
                      { headline: { url: '/a.png', alt: 'x' } }, { banner: { url: '/a.png" onerror="alert(1)', alt: 'x' } }]) {
    assert.equal(applyCopy(REGION, copy as any, ASSETS), REGION, JSON.stringify(copy));
  }
});

test('markup_it_cannot_balance_keeps_the_page_text', () => {
  for (const region of ['<h1 data-prb-field="headline">Own<b>bold</h1>', '<h1 data-prb-field="headline">Own<b>bold</h1></b>']) {
    assert.equal(applyCopy(region, { headline: 'New' }, ASSETS), region);
  }
});

test('the_edge_serves_copy_fields_from_core', async () => {
  const body = `<html><body><!-- prb:slot="hero" -->${REGION}<!-- /prb:slot --></body></html>`;
  const origin = async () => new Response(body, { headers: { 'content-type': 'text/html' } });
  const fetch = (async () => Response.json({ decision_id: 'd-h', slot: 'hero', variant: 'autumn', payload: null,
    copy: { headline: 'Autumn roast', banner: { asset: ID, alt: 'Autumn beans' } }, explanation: {} })) as unknown as typeof globalThis.fetch;
  const res = await createEdge({ core: 'http://core:8080', site: 'shop', origin, fetch }).handleRequest(new Request('https://shop.test/'));
  const d = doc(await res.text());
  assert.equal(d.querySelector('h1')!.textContent, 'Autumn roast');
  assert.equal(d.querySelector('img[data-prb-field]')!.getAttribute('src'), `/_prb/assets/shop/${ID}`);
});

test('the_edge_forwards_assets_and_nothing_else_under_assets', async () => {
  const calls: string[] = [];
  const fetch = (async (url: string) => {
    calls.push(String(url));
    return new Response('PNG', { headers: { 'content-type': 'image/png', 'content-security-policy': "default-src 'none'; sandbox", 'x-content-type-options': 'nosniff', etag: `"${ID}"`, 'set-cookie': 'x=1' } });
  }) as unknown as typeof globalThis.fetch;
  const edge = createEdge({ core: 'http://core:8080', site: 'shop', fetch });
  const res = await edge.handleRequest(new Request(`https://shop.test/_prb/assets/shop/${ID}`));
  assert.equal(res.status, 200);
  assert.equal(res.headers.get('content-security-policy'), "default-src 'none'; sandbox");
  assert.equal(res.headers.get('x-content-type-options'), 'nosniff');
  assert.equal(res.headers.get('set-cookie'), null);
  for (const bad of ['assets/shop/../../api/sites', `assets/shop/${ID}/x`, 'assets/shop/zz']) {
    assert.equal((await edge.handleRequest(new Request(`https://shop.test/_prb/${bad}`))).status, 404, bad);
  }
  assert.equal(calls.length, 1);
});
