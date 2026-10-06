// The Next.js install, end to end: the variant is in the HTML the server sends, so there is
// nothing to hide and nothing to swap. Exposure needs the page to hydrate; a click is credited
// by the running worker.
import { test, expect, request as pwRequest, type APIRequestContext } from '@playwright/test';
import { CORE, TOKEN, NEXT_PORT } from './env';

const SITE = `http://localhost:${NEXT_PORT}`;

async function core(): Promise<APIRequestContext> {
  return pwRequest.newContext({ baseURL: CORE, extraHTTPHeaders: { authorization: `Bearer ${TOKEN}` } });
}

/** Fresh cookie-less visitors until one is served `gift`. Returns the raw HTML and its cookie. */
async function giftVisitor() {
  for (let i = 0; i < 40; i++) {
    const ctx = await pwRequest.newContext();
    const res = await ctx.get(`${SITE}/`);
    const html = await res.text();
    if (/data-prb-slot="hero" data-prb-variant="gift"/.test(html)) {
      const subject = (await ctx.storageState()).cookies.find((c) => c.name === '_prb_subject')!.value;
      await ctx.dispose();
      return { html, subject };
    }
    await ctx.dispose();
  }
  throw new Error('no visitor was served gift in 40 tries');
}

function decisionId(html: string, slot: string) {
  return new RegExp(`data-prb-slot="${slot}" data-prb-variant="[^"]+" data-prb-decision="([^"]+)"`).exec(html)![1];
}

async function decision(id: string) {
  const c = await core();
  const d = await (await c.get(`/api/decisions/${id}`)).json();
  await c.dispose();
  return d;
}

test('first_html_byte_carries_the_variant', async () => {
  const { html } = await giftVisitor();
  // The raw server response, no JavaScript involved, already holds the repo's component.
  expect(html).toContain('Gift sets, wrapped and ready');
  expect(html).not.toContain('Single-origin espresso, roasted on Monday');
  // Nothing hidden, nothing to swap.
  expect(html).not.toMatch(/prb-hide|visibility:\s*hidden/);
  // And it is the decision Core recorded.
  expect((await decision(decisionId(html, 'hero'))).variant).toBe('gift');
});

test('javascript_off_still_shows_the_variant', async ({ browser }) => {
  const { html, subject } = await giftVisitor();
  const ctx = await browser.newContext({ javaScriptEnabled: false });
  await ctx.addCookies([{ name: '_prb_subject', value: subject, url: SITE }]);
  const page = await ctx.newPage();
  await page.goto('/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Gift sets, wrapped and ready');
  // The same visitor inside the decision TTL gets the same decision, not a re-roll.
  expect(await page.locator('[data-prb-slot="hero"]').getAttribute('data-prb-decision')).toBe(decisionId(html, 'hero'));
  await ctx.close();
});

test('exposure_is_recorded_after_hydration', async ({ page }) => {
  await page.goto('/');
  const id = await page.locator('[data-prb-slot="hero"]').getAttribute('data-prb-decision');
  await expect.poll(async () => (await decision(id!)).exposed_at, { timeout: 8000 }).not.toBeNull();
});

test('javascript_off_records_no_exposure', async ({ browser }) => {
  const ctx = await browser.newContext({ javaScriptEnabled: false });
  const page = await ctx.newPage();
  await page.goto('/');
  const id = await page.locator('[data-prb-slot="hero"]').getAttribute('data-prb-decision');
  await page.waitForTimeout(1500);
  expect((await decision(id!)).exposed_at).toBeNull();
  await ctx.close();
});

test('click_is_credited_by_the_worker', async ({ page }) => {
  await page.goto('/');
  const hero = page.locator('[data-prb-slot="hero"]');
  const id = await hero.getAttribute('data-prb-decision');
  await expect.poll(async () => (await decision(id!)).exposed_at, { timeout: 8000 }).not.toBeNull();
  await hero.getByRole('link').click();
  await expect.poll(async () => (await decision(id!)).outcome, { timeout: 8000 }).toBe(1);
});

test('one_render_makes_one_decision_per_slot', async () => {
  const ctx = await pwRequest.newContext();
  const html = await (await ctx.get(`${SITE}/`)).text();
  await ctx.dispose();
  const hero = await decision(decisionId(html, 'hero'));
  const promo = await decision(decisionId(html, 'promo'));
  expect(hero.subject).toBe(promo.subject);
  expect([hero.slot, promo.slot]).toEqual(['hero', 'promo']);
});

test('the_install_wizard_probes_see_it', async () => {
  const c = await core();
  const steps = (await (await c.get('/api/sites/shop/install')).json()).steps;
  await c.dispose();
  const state = Object.fromEntries(steps.map((s: any) => [s.id, s.state]));
  expect(state.package).toBe('done');
  expect(state.reach).toBe('done');
  expect(steps.find((s: any) => s.id === 'package').detail).toContain('next/0.1.0');
});
