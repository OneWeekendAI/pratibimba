import { test, expect, type Page } from '@playwright/test';

const CATS = ['electronics', 'kitchen', 'books', 'fashion', 'home', 'toys', 'grocery', 'beauty'];

// Collect console errors and failed requests (anything under /_prb is allowed to fail: Core is optional).
function watch(page: Page) {
  const bad: string[] = [];
  page.on('console', (m) => { if (m.type() === 'error' && !/\/_prb\//.test(m.text()) && !/status of 5\d\d|ERR_CONNECTION/.test(m.text() + (m.location().url || '')) ) bad.push('console: ' + m.text()); });
  page.on('pageerror', (e) => bad.push('pageerror: ' + e.message));
  page.on('requestfailed', (r) => { if (!r.url().includes('/_prb/')) bad.push('requestfailed: ' + r.url()); });
  page.on('response', (r) => { if (r.status() >= 400 && !r.url().includes('/_prb/')) bad.push(`http ${r.status()}: ${r.url()}`); });
  return bad;
}

test('every page type renders and nothing errors', async ({ page }) => {
  const bad = watch(page);

  await page.goto('/');
  await expect(page.getByTestId('hero')).toBeVisible();
  await expect(page.getByTestId('picks')).toBeVisible();
  await expect(page.getByTestId('deals')).toBeVisible();
  await expect(page.locator('[data-slot=category-row]')).toHaveCount(8);
  await expect(page.locator('[data-slot=promo-bar]')).toBeVisible();

  for (const c of CATS) {
    await page.goto(`/#/c/${c}`);
    await expect(page.getByTestId('cat-title')).toHaveText(new RegExp(c, 'i'));
    expect(await page.locator('[data-testid=grid] .card').count()).toBeGreaterThan(5);
    await expect(page.getByTestId('sort')).toHaveValue('popular');
  }

  // sort changes the grid, not the controls
  await page.goto('/#/c/kitchen');
  await page.getByTestId('sort').selectOption('price-low');
  const prices = await page.locator('[data-testid=grid] .card .price b').allTextContents();
  const nums = prices.map((t) => +t.replace(/[^\d]/g, ''));
  expect(nums).toEqual([...nums].sort((a, b) => a - b));

  // product page
  await page.locator('[data-testid=grid] .card-title').first().click();
  await expect(page.getByTestId('badge')).toBeVisible();
  await expect(page.getByTestId('buy')).toBeVisible();
  await expect(page.getByTestId('delivery')).toContainText('delivery');

  // slot regions must survive add-to-cart without re-rendering
  await page.getByTestId('buy').evaluate((el) => { (el as any).__mark = 1; });
  await page.getByTestId('buy').click();
  await expect(page.getByTestId('cart-count')).toHaveText('1');
  expect(await page.getByTestId('buy').evaluate((el) => (el as any).__mark)).toBe(1);

  // search
  await page.getByRole('searchbox').fill('speaker');
  await page.getByRole('button', { name: 'Search' }).click();
  await expect(page.getByTestId('search-title')).toContainText('speaker');
  expect(await page.locator('[data-testid=grid] .card').count()).toBeGreaterThan(0);

  // cart + cross-sell, then checkout with a real order id
  await page.getByTestId('cart-link').click();
  await expect(page.locator('[data-line]')).toHaveCount(1);
  await expect(page.getByTestId('cross-sell').locator('.card').first()).toBeVisible();
  await page.getByTestId('cross-sell').locator('[data-add]').first().click();
  await expect(page.locator('[data-line]')).toHaveCount(2);
  await page.getByTestId('to-checkout').click();
  await page.getByTestId('place-order').click();
  const id = await page.getByTestId('order-id').textContent();
  expect(id).toMatch(/^HAT-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('haat.orders')!));
  expect(stored.at(-1).id).toBe(id);
  await expect(page.getByTestId('cart-count')).toHaveText('0');

  expect(bad).toEqual([]);
});

test('home does not re-render on add-to-cart', async ({ page }) => {
  await page.goto('/');
  for (const s of ['hero', 'picks-strip', 'deals-strip', 'category-rows', 'promo-bar']) {
    await page.locator(`[data-slot=${s}]`).evaluate((el) => { (el as any).__mark = 1; });
  }
  await page.locator('[data-testid=picks] [data-add]').first().click();
  await expect(page.getByTestId('cart-count')).toHaveText('1');
  for (const s of ['hero', 'picks-strip', 'deals-strip', 'category-rows', 'promo-bar']) {
    expect(await page.locator(`[data-slot=${s}]`).evaluate((el) => (el as any).__mark)).toBe(1);
  }
});

test('home: countdown ticks, trust strip, deal pills, category tiles', async ({ page }) => {
  await page.goto('/');
  const cd = page.getByTestId('countdown');
  await expect(cd).toHaveText(/^\d\d:\d\d:\d\d$/);
  const a = await cd.textContent();
  await expect.poll(async () => cd.textContent(), { timeout: 4000 }).not.toBe(a);
  await expect(page.locator('[data-slot=trust-strip] > div')).toHaveCount(4);
  expect(await page.locator('[data-testid=deals] .pill-off').count()).toBeGreaterThan(5);
  await expect(page.locator('.cat-tile')).toHaveCount(8);
  await expect(page.locator('[data-slot=category-row] a', { hasText: 'See all' })).toHaveCount(8);
  await expect(page.locator('[data-slot=bestsellers] .best')).toHaveCount(10);
  // header: location chip, category select in the search, orders link
  await expect(page.locator('header .loc')).toContainText('560001');
  await page.locator('form.search select').selectOption('books');
  await page.getByRole('button', { name: 'Search' }).click();
  await expect(page.getByTestId('cat-title')).toHaveText(/books/i);
  await page.getByRole('link', { name: /Orders/ }).first().click();
  await expect(page.getByTestId('orders')).toBeVisible();
});

test('phone 390px: home has no horizontal overflow and header stays compact', async ({ browser }) => {
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 } });
  const page = await ctx.newPage();
  await page.goto('/');
  await expect(page.getByTestId('hero')).toBeVisible();
  const [sw, iw] = await page.evaluate(() => [document.documentElement.scrollWidth, innerWidth]);
  expect(sw).toBeLessThanOrEqual(iw);
  await expect(page.getByTestId('cart-link')).toBeVisible();
  await expect(page.getByRole('searchbox')).toBeVisible();
  await ctx.close();
});

test('listing: filters really filter, chips clear, search page shares the layout', async ({ page }) => {
  await page.goto('/#/c/electronics');
  const cards = page.locator('[data-testid=grid] .card');
  const n0 = await cards.count();
  await page.locator('[name=price][value="1000"]').check();
  const n1 = await cards.count();
  expect(n1).toBeLessThan(n0);
  const prices = (await page.locator('[data-testid=grid] .card .price b').allTextContents()).map((t) => +t.replace(/[^\d]/g, ''));
  for (const v of prices) { expect(v).toBeGreaterThanOrEqual(1000); expect(v).toBeLessThanOrEqual(2499); }
  await expect(page.getByTestId('count')).toHaveText(`${n1} products`);
  await page.locator('[name=rating][value="4"]').check();
  expect(await cards.count()).toBeLessThanOrEqual(n1);
  await page.locator('[data-chip=price]').click();
  await page.locator('[data-brand]').first().check();
  const nb = await cards.count();
  expect(nb).toBeLessThan(n0);
  await page.getByRole('button', { name: 'Clear all' }).click();
  expect(await cards.count()).toBe(n0);
  await page.getByTestId('f-stock').check();
  expect(await cards.count()).toBeLessThanOrEqual(n0);
  await page.goto('/#/search?q=a');
  await expect(page.getByTestId('filters')).toBeVisible();
  const s0 = await cards.count();
  await page.locator('[data-tag=gift]').check();
  expect(await cards.count()).toBeLessThan(s0);
});

test('product: gallery thumbs, qty, PIN delivery, FBT add-all', async ({ page }) => {
  await page.goto('/#/product/p001');
  const main = page.getByTestId('main-image');
  const before = await main.innerHTML();
  await page.getByTestId('thumb').nth(2).click();
  await expect(main).toHaveAttribute('data-v', '2');
  expect(await main.innerHTML()).not.toBe(before);
  await page.getByTestId('pin').fill('110001');
  await page.getByTestId('pin-check').click();
  await expect(page.getByTestId('delivery')).toContainText('110001');
  await page.getByTestId('pin').fill('12');
  await page.getByTestId('pin-check').click();
  await expect(page.getByTestId('pin-err')).toContainText('6-digit');
  await expect(page.getByTestId('histogram')).toBeVisible();
  await expect(page.getByTestId('review')).toHaveCount(3);
  await page.getByTestId('qty').selectOption('2');
  await page.getByTestId('buy').click();
  await expect(page.getByTestId('cart-count')).toHaveText('2');
  await page.getByTestId('fbt-add').click();
  await expect(page.getByTestId('cart-count')).toHaveText('5');
  await expect(page.getByTestId('also-viewed').locator('.card').first()).toBeVisible();
  await page.getByTestId('buy-now').click();
  await expect(page).toHaveURL(/#\/checkout/);
});

test('phone 390px: listing + product no overflow, filter drawer opens', async ({ browser }) => {
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 } });
  const page = await ctx.newPage();
  await page.goto('/#/c/electronics');
  await expect(page.getByTestId('grid').locator('.card').first()).toBeVisible();
  await expect(page.getByTestId('filters')).toBeHidden();
  await page.getByTestId('filter-toggle').click();
  await expect(page.getByTestId('filters')).toBeVisible();
  await page.locator('[name=price][value="u500"]').check();
  await page.getByRole('button', { name: /^Show \d+ result/ }).click();
  await expect(page.getByTestId('filters')).toBeHidden();
  for (const path of ['/#/c/electronics', '/#/search?q=a', '/#/product/p001']) {
    await page.goto(path);
    await page.waitForTimeout(200);
    const [sw, iw] = await page.evaluate(() => [document.documentElement.scrollWidth, innerWidth]);
    expect(sw, path).toBeLessThanOrEqual(iw);
  }
  await ctx.close();
});

const seed = (page: Page, items: Record<string, number>) => page.addInitScript((c) => { if (!sessionStorage.getItem('seeded')) { localStorage.setItem('haat.cart', JSON.stringify(c)); sessionStorage.setItem('seeded', '1'); } }, items);
const num = (t: string | null) => +(t || '').replace(/[^\d]/g, '');

test('cart: qty stepper updates totals, free-delivery bar, remove, empty state', async ({ page }) => {
  await seed(page, { p034: 1 }); // 269: below the 499 free-delivery line
  await page.goto('/#/cart');
  await expect(page.getByTestId('free-msg')).toContainText('more for free delivery');
  await expect(page.getByTestId('cart-total')).toHaveText('₹309'); // 269 + 40 delivery
  await page.locator('[data-inc=p034]').click();
  await expect(page.getByTestId('line-qty')).toHaveText('2');
  await expect(page.getByTestId('line-sub')).toHaveText('₹538');
  await expect(page.getByTestId('free-msg')).toContainText('free delivery on this order');
  await expect(page.getByTestId('cart-total')).toHaveText('₹538');
  await expect(page.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '100');
  await expect(page.getByTestId('cart-count')).toHaveText('2');
  await page.locator('[data-dec=p034]').click();
  await expect(page.getByTestId('cart-total')).toHaveText('₹309');
  await page.locator('[data-rm=p034]').click();
  await expect(page.locator('[data-line]')).toHaveCount(0);
  await expect(page.getByTestId('cart-empty')).toBeVisible();
  await expect(page.getByTestId('cart-empty')).toContainText('Your cart is empty');
  await expect(page.getByTestId('cart-count')).toHaveText('0');
  await expect(page.getByTestId('cross-sell').locator('.card').first()).toBeVisible();
});

test('checkout: prefilled address, delivery + payment choices, confirmation id, orders list', async ({ page }) => {
  await seed(page, { p001: 1 });
  await page.goto('/#/checkout');
  await expect(page.locator('[name=name]')).not.toHaveValue('');
  await expect(page.locator('[name=pin]')).toHaveValue('560001');
  const total = page.getByTestId('co-total');
  await expect(total).toHaveText('₹1,019');
  await page.locator('[name=speed][value=express]').check();
  await expect(total).toHaveText('₹1,118');
  await expect(page.locator('[data-for=upi]')).toBeVisible();
  await page.locator('[name=pay][value=card]').check();
  await expect(page.locator('[data-for=card]')).toBeVisible();
  await expect(page.locator('[data-for=upi]')).toBeHidden();
  await page.locator('[name=pay][value=cod]').check();
  await expect(page.locator('[data-for=cod]')).toBeVisible();
  await page.getByTestId('place-order').click();
  await expect(page.getByTestId('order-id')).toHaveText(/^HAT-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('haat.orders')!).at(-1));
  expect(stored.payment).toBe('cod');
  expect(stored.speed).toBe('express');
  expect(stored.total).toBe(1118);
  await page.getByRole('link', { name: 'Your orders' }).last().click();
  await expect(page.getByTestId('orders')).toContainText(stored.id);
  await page.goto('/#/checkout');
  await expect(page.getByTestId('cart-empty')).toBeVisible();
});

test('404 state and brand store link', async ({ page }) => {
  await page.goto('/#/nope');
  await expect(page.getByTestId('not-found')).toContainText('could not find');
  await page.goto('/#/product/p001');
  await page.getByRole('link', { name: /Visit the Kiran store/ }).click();
  await expect(page).toHaveURL(/#\/search\?q=Kiran/);
  await expect(page.getByTestId('search-title')).toContainText('Kiran');
  await expect(page.locator('[data-testid=grid] .card').first()).toBeVisible();
});

test('phone 390px: cart, checkout, confirmation, orders, empty, 404 do not overflow', async ({ browser }) => {
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 } });
  const page = await ctx.newPage();
  const fits = async (label: string) => {
    await page.waitForTimeout(200);
    const [sw, iw] = await page.evaluate(() => [document.documentElement.scrollWidth, innerWidth]);
    expect(sw, label).toBeLessThanOrEqual(iw);
  };
  await page.goto('/');
  await page.evaluate(() => localStorage.setItem('haat.cart', JSON.stringify({ p001: 1, p002: 2 })));
  await page.goto('/#/cart'); await page.reload(); await fits('cart');
  await page.goto('/#/checkout'); await fits('checkout');
  await page.locator('[name=pay][value=card]').check(); await fits('checkout card');
  await page.getByTestId('place-order').click();
  await expect(page.getByTestId('order-id')).toBeVisible(); await fits('confirmation');
  await page.goto('/#/orders'); await fits('orders');
  await page.goto('/#/cart'); await expect(page.getByTestId('cart-empty')).toBeVisible(); await fits('empty cart');
  await page.goto('/#/nope'); await fits('404');
  await ctx.close();
});

// ---------------------------------------------------------------------------------------------
// Pratibimba integration, Core ABSENT: the SDK tags are requested and fail; the store must be
// exactly its own baseline. (Nothing here starts Core.)
// ---------------------------------------------------------------------------------------------
const PAGES = ['/', '/#/c/kitchen', '/#/search?q=speaker', '/#/product/p001', '/#/cart', '/#/checkout', '/#/orders', '/#/nope'];
const SLOT_KEYS = ['hero', 'category-rows', 'picks', 'deals', 'listing-sort', 'product-badge', 'buy-button', 'delivery-message', 'cart-cross-sell', 'promo-bar'];
const ANTI_FLICKER_MS = 400; // the SDK's documented default (data-timeout); the tag in index.html sets none

test('Core absent: SDK tags requested and failing, every page renders its baseline with no errors', async ({ page }) => {
  const bad = watch(page);
  const prbReqs: string[] = [];
  page.on('request', (r) => { if (r.url().includes('/_prb/')) prbReqs.push(new URL(r.url()).pathname); });
  await page.goto('/#/c/kitchen');
  for (const path of PAGES) {
    await page.goto('/');
    await page.evaluate(() => localStorage.setItem('haat.cart', JSON.stringify({ p001: 1 })));
    await page.goto(path);
    await page.reload();
    await expect(page.locator('#view > *').first()).toBeVisible();
    // nothing the SDK would hide is hidden, and it left no hide rule or wait marker behind
    expect(await page.locator('#prb-hide, #prb-wait').count(), path).toBe(0);
    expect(await page.locator('[data-prb-wait]').count(), path).toBe(0);
    const hidden = await page.$$eval('[data-prb-slot]', (els) => els.filter((e) => getComputedStyle(e).visibility === 'hidden').length);
    expect(hidden, path).toBe(0);
    expect(await page.evaluate(() => (window as any).prb), path).toBeUndefined();
  }
  expect(prbReqs).toContain('/_prb/manifest/haat.js');
  expect(prbReqs).toContain('/_prb/sdk.js');
  expect(bad).toEqual([]);
});

test('Core absent: baseline is what the markup says, and no slot was touched', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('[data-prb-slot=promo-bar] [data-prb-field=text]')).toContainText('Free delivery on orders above');
  await expect(page.locator('[data-prb-slot=hero] h1')).toHaveText('Everything for your home, at market prices');
  expect(await page.locator('[data-prb-slot=category-rows] > [data-prb-item]').evaluateAll((els) => els.map((e) => e.getAttribute('data-prb-item')))).toEqual(CATS);
  expect(await page.locator('[data-prb-slot=picks] > [data-prb-item]').count()).toBe(12);
  expect(await page.locator('[data-prb-slot=deals] > [data-prb-item]').count()).toBe(12);
  expect(await page.locator('[data-prb-variant]').count()).toBe(0);
  await page.goto('/#/product/p001');
  await expect(page.getByTestId('buy')).toHaveText('Add to cart');
  await expect(page.getByTestId('delivery')).toHaveText(/^(Free delivery|Standard delivery for ₹40) by .+ to 560001$/);
  await page.goto('/#/c/kitchen');
  await expect(page.getByTestId('sort')).toHaveValue('popular');
  await page.goto('/#/cart');
  await expect(page.locator('[data-prb-slot=cart-cross-sell] > [data-prb-item]').first()).toBeVisible();
});

// Cumulative layout shift, measured with a buffered PerformanceObserver from the first byte.
async function cls(page: Page, path: string, settleMs = 1200) {
  await page.addInitScript(() => {
    (window as any).__cls = 0;
    new PerformanceObserver((l) => { for (const e of l.getEntries() as any[]) if (!e.hadRecentInput) (window as any).__cls += e.value; }).observe({ type: 'layout-shift', buffered: true });
  });
  await page.goto(path);
  await expect(page.locator('#view > *').first()).toBeVisible();
  await page.waitForTimeout(settleMs);
  return page.evaluate(() => (window as any).__cls as number);
}

for (const [name, path] of [['home', '/'], ['product', '/#/product/p001']] as const) {
  test(`Core absent: no layout shift on ${name} (SDK requested, failing)`, async ({ page }) => {
    expect(await cls(page, path)).toBeLessThan(0.01);
  });
}

test('Core hung (slow-failing /_prb): page still renders its baseline, no layout shift, no hidden content', async ({ page }) => {
  // Worst realistic case for blocking tags: Core accepts the connection and then answers an error late.
  await page.route('**/_prb/**', async (route) => { await new Promise((r) => setTimeout(r, 900)); await route.fulfill({ status: 504, body: '' }); });
  const t0 = Date.now();
  const v = await cls(page, '/', 600);
  expect(v).toBeLessThan(0.01);
  await expect(page.getByTestId('hero')).toBeVisible();
  expect(await page.locator('#prb-hide').count()).toBe(0);
  expect(Date.now() - t0).toBeLessThan(8000);
});

test('real sdk.js, manifest from site.json, /decide never answers: slots hidden at most data-timeout, then baseline, no shift', async ({ page }) => {
  // No Core: the browser is handed the real packages/client/sdk.js and a manifest built from site.json,
  // and /_prb/decide hangs. The SDK must reveal the page's own baseline once its timeout (400 ms) passes.
  const { readFileSync } = await import('node:fs');
  const sdk = readFileSync(new URL('../../../packages/client/sdk.js', import.meta.url), 'utf8');
  const site = JSON.parse(readFileSync(new URL('../site.json', import.meta.url), 'utf8'));
  const manifest = { site: 'haat', privacy_mode: site.privacy_mode, features: site.features, slots: site.slots.map((s: any) => ({ key: s.key, selector: s.selector, kind: s.kind || 'swap', metric: s.metric })) };
  await page.route('**/_prb/manifest/haat.js', (r) => r.fulfill({ contentType: 'application/javascript', body: `window.__prb_manifest=${JSON.stringify(manifest)};` }));
  await page.route('**/_prb/sdk.js', (r) => r.fulfill({ contentType: 'application/javascript', body: sdk }));
  await page.route('**/_prb/decide*', () => { /* never answered */ });
  await page.addInitScript(() => {
    (window as any).__cls = 0;
    new PerformanceObserver((l) => { for (const e of l.getEntries() as any[]) if (!e.hadRecentInput) (window as any).__cls += e.value; }).observe({ type: 'layout-shift', buffered: true });
  });
  await page.addInitScript(() => {
    (window as any).__sawHide = 0;
    new MutationObserver(() => { if (document.getElementById('prb-hide') && !(window as any).__sawHide) (window as any).__sawHide = performance.now(); }).observe(document, { childList: true, subtree: true });
  });
  await page.goto('/');
  // the SDK's hide rule is present right after load, and gone shortly after its timeout
  await page.waitForSelector('[data-prb-slot=hero]', { state: 'attached' });
  const t0 = Date.now();
  await expect(page.locator('#prb-hide')).toHaveCount(0, { timeout: 2000 });
  const hiddenFor = Date.now() - t0;
  expect(hiddenFor).toBeLessThan(ANTI_FLICKER_MS + 400);
  expect(await page.evaluate(() => (window as any).__sawHide), 'the SDK really hid the slots first').toBeGreaterThan(0);
  await expect(page.locator('[data-prb-slot=hero] h1')).toBeVisible();
  const stillHidden = await page.$$eval('[data-prb-slot]', (els) => els.filter((e) => getComputedStyle(e).visibility === 'hidden').length);
  expect(stillHidden).toBe(0);
  expect(await page.locator('[data-prb-variant]').count()).toBe(0); // baseline, nothing swapped
  await expect(page.locator('[data-prb-slot=hero] h1')).toHaveText('Everything for your home, at market prices');
  await page.waitForTimeout(500);
  expect(await page.evaluate(() => (window as any).__cls as number)).toBeLessThan(0.01);
});

test('outcome events: add_to_cart per add, purchase with the real order id, value and INR', async ({ page }) => {
  await page.addInitScript(() => {
    (window as any).__calls = [];
    (window as any).prb = {
      addToCart: (x: any) => (window as any).__calls.push(['add_to_cart', x]),
      purchase: (x: any) => (window as any).__calls.push(['purchase', x]),
    };
  });
  await page.goto('/#/product/p001');
  await page.getByTestId('qty').selectOption('2');
  await page.getByTestId('buy').click();
  await page.locator('[data-testid=also-viewed] [data-add]').first().click();
  await page.goto('/#/checkout');
  await page.getByTestId('place-order').click();
  const id = (await page.getByTestId('order-id').textContent())!;
  const calls: any[] = await page.evaluate(() => (window as any).__calls);
  const adds = calls.filter((c) => c[0] === 'add_to_cart');
  expect(adds.length).toBe(2);
  expect(adds[0][1]).toMatchObject({ id: 'p001', quantity: 2 });
  expect(typeof adds[0][1].price).toBe('number');
  const buys = calls.filter((c) => c[0] === 'purchase');
  expect(buys.length).toBe(1);
  expect(buys[0][1]).toMatchObject({ order_id: id, currency: 'INR' });
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('haat.orders')!).at(-1));
  expect(buys[0][1].value).toBe(stored.total);
  expect(buys[0][1].items.length).toBeGreaterThan(0);
});

test('delivery slot template: the page fills {fee} {day} {pin} when a decided variant writes them', async ({ page }) => {
  await page.goto('/#/product/p001');
  const line = page.locator('[data-prb-slot=delivery-message] [data-prb-field=line]');
  // what the SDK does for a copy variant: textContent on the field
  await line.evaluate((el) => { el.textContent = '{fee}, arriving {day} to {pin}'; });
  await expect(line).toHaveText(/^Free delivery, arriving .+ to 560001$/);
  await page.getByTestId('pin').fill('110001');
  await page.getByTestId('pin-check').click();
  await expect(line).toHaveText(/arriving .+ to 110001$/);
});

test('default-sort slot sets the control only until the visitor chooses', async ({ page }) => {
  await page.goto('/#/c/kitchen');
  const marker = page.locator('[data-prb-slot=listing-sort]');
  await marker.evaluate((el) => { el.innerHTML = '<i data-sort="price-low"></i>'; });
  await expect(page.getByTestId('sort')).toHaveValue('price-low');
  await page.getByTestId('sort').selectOption('rating');
  await marker.evaluate((el) => { el.innerHTML = '<i data-sort="popular"></i>'; });
  await expect(page.getByTestId('sort')).toHaveValue('rating');
});

test('Core absent: screenshots of the baseline home (desktop, phone)', async ({ browser }) => {
  const { mkdirSync } = await import('node:fs');
  mkdirSync('proof/design', { recursive: true });
  for (const [name, viewport] of [['desktop', { width: 1440, height: 900 }], ['phone', { width: 390, height: 844 }]] as const) {
    const ctx = await browser.newContext({ viewport });
    const page = await ctx.newPage();
    await page.goto('/');
    await expect(page.getByTestId('hero')).toBeVisible();
    await page.waitForTimeout(600);
    await page.screenshot({ path: `proof/design/home-core-absent-${name}.png` });
    await ctx.close();
  }
});
