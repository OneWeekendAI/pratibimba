// S11: an image field of a copy slot in <prb.Copy>. The fallback gives the page's own image as
// {src, alt}; a decided image (an uploaded asset or a URL) replaces it, resolved to a src the
// browser fetches through the site's own /_prb door. <prb.Image> renders one as a plain <img>.
import { test } from 'node:test';
import assert from 'node:assert/strict';

const { createElement } = await import('react');
const { renderToStaticMarkup } = await import('react-dom/server');
const { createSlot, copyText, PrbImage } = await import('../src/slot.ts');
const { createRoute } = await import('../src/route.ts');

const ID = 'cd'.repeat(32);
const visitorHeaders = async () => new Headers({ 'x-prb-subject': 'v1', 'x-prb-ctx': 'r1m0a0' });

function slotWith(copy: Record<string, unknown> | null, extra: Record<string, unknown> = {}) {
  const fetch = (async () =>
    new Response(JSON.stringify({ decisions: [{ decision_id: 'd-hero', slot: 'hero', variant: copy ? 'autumn' : 'own', payload: null, copy, explanation: {} }] }), {
      status: 200, headers: { 'content-type': 'application/json' },
    })) as unknown as typeof globalThis.fetch;
  return createSlot({ core: 'http://core', site: 'shop', fetch, headers: visitorHeaders, requestScope: () => ({}), ...extra });
}

const fallback = { headline: 'Our own', banner: { src: '/own.jpg', alt: 'Our own beans', width: 800, height: 300 } };
type F = typeof fallback;
const hero = (t: F) => createElement('section', null, createElement('h1', null, t.headline), createElement(PrbImage, { image: t.banner, className: 'hero' }));

test('copy_image_from_an_asset_renders_through_the_prb_door', async () => {
  const { Copy } = slotWith({ banner: { asset: ID, alt: 'Autumn beans', width: 1600, height: 500 } });
  const html = renderToStaticMarkup(await Copy({ id: 'hero', fallback, children: hero }));
  assert.match(html, new RegExp(`<img src="/_prb/assets/shop/${ID}" alt="Autumn beans" width="1600" height="500" class="hero"/>`));
  assert.match(html, /<h1>Our own<\/h1>/);
});

test('copy_image_from_a_url_has_no_size_unless_given', async () => {
  const { Copy } = slotWith({ banner: { url: 'https://cdn.example.com/a.webp', alt: 'Beans' } });
  const html = renderToStaticMarkup(await Copy({ id: 'hero', fallback, children: hero }));
  assert.match(html, /<img src="https:\/\/cdn.example.com\/a.webp" alt="Beans" class="hero"\/>/);
});

test('copy_image_baseline_and_unsafe_values_keep_the_page_image', async () => {
  for (const copy of [null, { banner: { url: 'javascript:alert(1)', alt: 'x' } }, { banner: { url: '//evil.example/a', alt: 'x' } },
                      { banner: { asset: '../x', alt: 'x' } }, { banner: { url: '/a.png' } }, { banner: 'text' }]) {
    const { Copy } = slotWith(copy);
    const html = renderToStaticMarkup(await Copy({ id: 'hero', fallback, children: hero }));
    assert.match(html, /<img src="\/own.jpg" alt="Our own beans" width="800" height="300"/, JSON.stringify(copy));
  }
});

test('copy_image_alt_is_escaped', async () => {
  const { Copy } = slotWith({ banner: { url: '/a.png', alt: '"><script>x</script>' } });
  const html = renderToStaticMarkup(await Copy({ id: 'hero', fallback, children: hero }));
  assert.doesNotMatch(html, /<script>/);
});

test('copy_image_honours_a_custom_assets_path', async () => {
  const { Copy } = slotWith({ banner: { asset: ID, alt: 'A' } }, { assetsPath: 'https://img.shop.test/prb' });
  const html = renderToStaticMarkup(await Copy({ id: 'hero', fallback, children: hero }));
  assert.match(html, new RegExp(`src="https://img.shop.test/prb/shop/${ID}"`));
});

test('copy_text_never_takes_an_image_and_an_image_never_takes_text', () => {
  const t = copyText(fallback, { headline: { url: '/a.png', alt: 'x' }, banner: 'text' }, '/_prb/assets/shop');
  assert.deepEqual(t, fallback);
});

test('route_forwards_an_asset_with_its_safety_headers', async () => {
  const calls: string[] = [];
  const f = (async (url: string) => {
    calls.push(url);
    return new Response(new Uint8Array([137, 80, 78, 71]), { status: 200, headers: {
      'content-type': 'image/png', 'cache-control': 'public, max-age=31536000, immutable', etag: `"${ID}"`,
      'content-security-policy': "default-src 'none'; sandbox", 'x-content-type-options': 'nosniff', 'content-disposition': 'inline', 'set-cookie': 'leak=1' } });
  }) as unknown as typeof fetch;
  const r = createRoute({ core: 'http://core:8080', fetch: f });
  const res = await r.GET(new Request(`https://shop.test/_prb/assets/shop/${ID}`));
  assert.equal(res.status, 200);
  assert.equal(calls[0], `http://core:8080/assets/shop/${ID}`);
  assert.equal(res.headers.get('content-security-policy'), "default-src 'none'; sandbox");
  assert.equal(res.headers.get('x-content-type-options'), 'nosniff');
  assert.equal(res.headers.get('etag'), `"${ID}"`);
  assert.equal(res.headers.get('set-cookie'), null);
  assert.deepEqual([...new Uint8Array(await res.arrayBuffer())], [137, 80, 78, 71]);
  for (const bad of ['assets/shop/../../api/sites', `assets/shop/${ID}/x`, 'assets/shop/abc', `assets/../${ID}`]) {
    assert.equal((await r.GET(new Request(`https://shop.test/_prb/${bad}`))).status, 404, bad);
  }
  assert.equal(calls.length, 1);
});
