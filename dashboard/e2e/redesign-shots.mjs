// Screenshots of every page at 1280px and 390px against a running Core (default :8080).
import { chromium } from '@playwright/test';
const BASE = process.env.BASE ?? 'http://localhost:8080';
const TOKEN = process.env.TOKEN ?? 'local-admin-token';
const OUT = process.argv[2];
const only = process.argv[3];
// COLOR=dark renders through prefers-color-scheme, as a visitor's OS would.
const COLOR = process.env.COLOR ?? 'light';
const SUFFIX = COLOR === 'dark' ? '-dark' : '';
const browser = await chromium.launch();
async function run(width, tag) {
  const ctx = await browser.newContext({ viewport: { width, height: width > 600 ? 900 : 844 }, hasTouch: width < 600, isMobile: width < 600, colorScheme: COLOR });
  const page = await ctx.newPage();
  await page.goto(`${BASE}/app/login/`);
  await page.getByLabel('Admin token').fill(TOKEN);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await page.waitForURL(/\/app\/(?!login)/);
  const api = async (p) => (await page.request.get(`${BASE}/api${p}`)).json();
  const sites = await api('/sites');
  const site = sites[0].id;
  const ov = await api(`/sites/${site}/overview`);
  const slot = ov.slots[0]?.key;
  const ds = slot ? await api(`/sites/${site}/decisions?slot=${slot}&limit=5`) : [];
  const calls = await api(`/sites/${site}/mcp-calls?limit=1`);
  const pages = {
    live: `/app/site/?id=${site}`,
    slot: `/app/slot/?site=${site}&slot=${slot}`,
    decision: ds[0] ? `/app/decision/?id=${ds[0].id}&site=${site}` : null,
    results: `/app/results/?site=${site}`,
    install: `/app/install/?site=${site}`,
    agent: `/app/agent/?site=${site}`,
    drafts: `/app/drafts/?site=${site}`,
    review: `/app/approvals/?site=${site}`,
    preview: `/app/preview/?site=${site}`,
    audit: `/app/audit/?site=${site}`,
    auditcall: calls.calls?.[0] ? `/app/audit/?id=${calls.calls[0].id}` : null,
    webhooks: `/app/webhooks/?site=${site}`,
  };
  for (const [name, url] of Object.entries(pages)) {
    if (!url || (only && !only.split(',').includes(name))) continue;
    await page.goto(`${BASE}${url}`);
    await page.waitForLoadState('networkidle', { timeout: 8000 }).catch(() => {});
    await page.waitForTimeout(800);
    await page.screenshot({ path: `${OUT}/${name}-${tag}${SUFFIX}.png`, fullPage: name !== 'preview' });
  }
  // Help popover and tooltip, opened by keyboard and by tap.
  if (!only || only.includes('help')) {
    await page.goto(`${BASE}${pages.live}`);
    await page.waitForTimeout(800);
    const help = page.getByRole('button', { name: 'What is this?' }).first();
    if (width < 600) await help.tap(); else { await help.focus(); await page.keyboard.press('Enter'); }
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${OUT}/help-${tag}${SUFFIX}.png` });
    await page.keyboard.press('Escape');
    const stage = page.getByRole('link', { name: /Learn/ }).first();
    if (width < 600) { await page.getByRole('button', { name: 'Pause all slots' }).dispatchEvent('pointerdown', { pointerType: 'touch' }); }
    else { await page.keyboard.press('Tab'); await stage.focus(); }
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${OUT}/tooltip-${tag}${SUFFIX}.png` });
  }
  await ctx.close();
}
if (process.env.WIDTH !== 'phone') await run(1280, 'desktop');
if (process.env.WIDTH !== 'desktop') await run(390, 'phone');
await browser.close();
