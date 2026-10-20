// S11: an image field of a copy slot. The page marks the <img> (or a <picture>/wrapper holding
// one) with data-prb-field; the SDK sets src, alt and size as attributes, drops srcset, sizes and
// <source> elements that would override the new src, and never writes markup.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { load, ok, sleep } from './harness.mjs';

const ID = 'ab'.repeat(32);
const MANIFEST = {
  site: 'demo',
  slots: [{ key: 'hero', selector: '#hero', metric: 'click', ttl_secs: 60, kind: 'copy',
            variants: [{ key: 'own', baseline: true }, { key: 'autumn', baseline: false }] }],
};
const BODY = '<section id="hero"><h1 data-prb-field="headline">Our own</h1>' +
  '<picture id="pic"><source id="src1" srcset="/own.avif" type="image/avif"><img id="banner" data-prb-field="banner" src="/own.jpg" srcset="/own-2x.jpg 2x" sizes="100vw" alt="Our own beans" width="800" height="300"></picture>' +
  '<div data-prb-field="side"><img id="side" src="/side.jpg" alt="Side"></div></section>';
const decision = (copy) => ({ decision_id: 'd-1', slot: 'hero', variant: 'autumn', payload: null, copy, explanation: {} });
const page = (copy, body = BODY) => load({ manifest: MANIFEST, body, decide: () => ok(decision(copy)) });
const $ = (p, sel) => p.w.document.querySelector(sel);

test('image_field_from_an_asset_sets_src_alt_and_size', async () => {
  const p = page({ banner: { asset: ID, alt: 'Autumn beans', width: 1600, height: 500 } });
  await sleep(20);
  const img = $(p, '#banner');
  assert.equal(img.getAttribute('src'), `https://core.test/assets/demo/${ID}`);
  assert.equal(img.getAttribute('alt'), 'Autumn beans');
  assert.equal(img.getAttribute('width'), '1600');
  assert.equal(img.getAttribute('height'), '500');
  assert.equal(img.hasAttribute('srcset'), false, 'srcset would override the new src');
  assert.equal(img.hasAttribute('sizes'), false);
  assert.equal($(p, '#src1'), null, 'a <source> would override the new src');
  assert.equal($(p, 'h1').textContent, 'Our own', 'a field the variant leaves out keeps the page');
});

test('image_field_from_a_url_and_one_without_a_size', async () => {
  const p = page({ banner: { url: 'https://cdn.example.com/a.webp', alt: 'Beans' } });
  await sleep(20);
  const img = $(p, '#banner');
  assert.equal(img.getAttribute('src'), 'https://cdn.example.com/a.webp');
  assert.equal(img.hasAttribute('width'), false, 'the page size would distort the new image');
  assert.equal(img.hasAttribute('height'), false);
});

test('image_field_on_a_wrapper_sets_the_img_inside', async () => {
  const p = page({ side: { url: '/new-side.png', alt: 'New side' } });
  await sleep(20);
  assert.equal($(p, '#side').getAttribute('src'), '/new-side.png');
  assert.equal($(p, '#side').getAttribute('alt'), 'New side');
});

test('image_alt_markup_stays_an_attribute', async () => {
  const p = page({ banner: { url: '/a.png', alt: '"><script id="x"></script>' } });
  await sleep(20);
  assert.equal($(p, '#x'), null);
  assert.equal($(p, '#banner').getAttribute('alt'), '"><script id="x"></script>');
});

test('an_unsafe_image_address_is_ignored', async () => {
  for (const bad of [{ url: 'javascript:alert(1)', alt: 'x' }, { url: '//evil.example/a.png', alt: 'x' }, { url: 'data:image/svg+xml,<svg>', alt: 'x' },
                     { asset: '../../etc/passwd', alt: 'x' }, { asset: ID.toUpperCase(), alt: 'x' }, { url: '/a.png' }]) {
    const p = page({ banner: bad });
    await sleep(20);
    assert.equal($(p, '#banner').getAttribute('src'), '/own.jpg', JSON.stringify(bad));
    assert.equal($(p, '#banner').getAttribute('alt'), 'Our own beans');
  }
});

test('kinds_do_not_cross', async () => {
  // Text to an image field and an image to a text field change nothing.
  const p = page({ banner: 'just text', headline: { url: '/a.png', alt: 'x' } });
  await sleep(20);
  assert.equal($(p, '#banner').getAttribute('src'), '/own.jpg');
  assert.equal($(p, 'h1').textContent, 'Our own');
  assert.equal($(p, 'h1').querySelector('img'), null);
});
