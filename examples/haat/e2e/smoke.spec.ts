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
