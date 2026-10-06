// The edge install, end to end: the origin's own HTML, rewritten as it streams.
import { test, expect, request as pwRequest } from '@playwright/test';
import { CORE } from '../playwright.config';

const EDGE = 'http://localhost:3200';

async function decision(id: string) {
  const c = await pwRequest.newContext({ baseURL: CORE, extraHTTPHeaders: { authorization: 'Bearer e2e-admin-token' } });
  const d = await (await c.get(`/api/decisions/${id}`)).json();
  await c.dispose();
  return d;
}

function decisionsIn(html: string): { slot: string; id: string }[] {
  const m = /data-prb='([^']+)'/.exec(html);
  return m ? JSON.parse(m[1].replace(/&#39;/g, "'").replace(/&lt;/g, '<').replace(/&amp;/g, '&')).decisions : [];
}

test('edge_first_byte_carries_the_variant', async () => {
  for (let i = 0; i < 40; i++) {
    const ctx = await pwRequest.newContext();
    const res = await ctx.get(`${EDGE}/`);
    const html = await res.text();
    await ctx.dispose();
    const [d] = decisionsIn(html);
    const recorded = await decision(d.id);
    if (recorded.variant !== 'gift') continue;
    expect(html).toContain('<!-- prb:slot="hero" --><h1>Gift sets, wrapped and ready</h1>');
    expect(html).not.toContain('Single-origin espresso');
    expect(res.headers()['cache-control']).toBe('private, no-store');
    return;
  }
  throw new Error('no visitor was served gift in 40 tries');
});

test('edge_exposure_is_recorded_after_load', async ({ page }) => {
  const res = await page.goto('/');
  const [d] = decisionsIn(await res!.text());
  await expect.poll(async () => (await decision(d.id)).exposed_at, { timeout: 8000 }).not.toBeNull();
});

test('edge_click_in_slot_is_credited', async ({ page }) => {
  const res = await page.goto('/');
  const [d] = decisionsIn(await res!.text());
  await expect.poll(async () => (await decision(d.id)).exposed_at, { timeout: 8000 }).not.toBeNull();
  await page.locator('#hero a').click();
  await expect.poll(async () => (await decision(d.id)).outcome, { timeout: 8000 }).toBe(1);
});

test('edge_click_outside_slot_is_not_credited', async ({ page }) => {
  const res = await page.goto('/');
  const [d] = decisionsIn(await res!.text());
  await expect.poll(async () => (await decision(d.id)).exposed_at, { timeout: 8000 }).not.toBeNull();
  await page.locator('#nav-link').click();
  await page.waitForTimeout(1200);
  expect((await decision(d.id)).outcome).toBeNull();
});
