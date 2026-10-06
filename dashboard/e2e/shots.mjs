// Screenshots of every dashboard screen, light and dark: `node e2e/shots.mjs <outdir>`
// against a running stack (node e2e/stack.mjs).
import { chromium } from '@playwright/test';
import { mkdirSync } from 'node:fs';

const out = process.argv[2] ?? 'screenshots';
mkdirSync(out, { recursive: true });
const BASE = 'http://127.0.0.1:18083';
const b = await chromium.launch();
const errors = [];
for (const theme of ['light', 'dark']) {
  const ctx = await b.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: theme });
  const page = await ctx.newPage();
  page.on('pageerror', (e) => errors.push(`${theme} ${page.url()}: ${e.message}`));
  page.on('console', (m) => m.type() === 'error' && errors.push(`${theme} ${page.url()}: ${m.text()}`));
  await page.goto(`${BASE}/app/login/`);
  await page.screenshot({ path: `${out}/${theme}-0-login.png` });
  await page.getByLabel('Admin token').fill('dash-admin-token');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await page.waitForURL(/\/app\/site\//);
  await page.evaluate((t) => { document.documentElement.dataset.theme = t; localStorage.setItem('prb-theme', t); }, theme);
  const shot = async (name, url, prep) => {
    await page.goto(BASE + url);
    await page.waitForLoadState('networkidle').catch(() => {});
    await page.waitForTimeout(1500);
    if (prep) await prep();
    // Walk the page so every iframe preview below the fold has painted before the capture.
    await page.evaluate(async () => {
      for (let y = 0; y < document.body.scrollHeight; y += 500) {
        scrollTo(0, y);
        await new Promise((r) => setTimeout(r, 80));
      }
      scrollTo(0, 0);
    });
    await page.screenshot({ path: `${out}/${theme}-${name}.png`, fullPage: true });
  };
  await shot('1-site-home', '/app/site/?id=shop');
  await shot('2-slot-workbench', '/app/slot/?site=shop&slot=hero');
  await page.locator('[data-testid=decisions-table] tbody tr').first().click();
  await page.waitForURL(/decision/);
  const decisionUrl = new URL(page.url()).pathname + new URL(page.url()).search;
  await shot('3-decision', decisionUrl, async () => {
    await page.getByRole('button', { name: /What would/ }).click();
    await page.waitForTimeout(300);
  });
  await shot('4-results', '/app/results/?site=shop');
  await shot('5-approvals', '/app/approvals/?site=shop', async () => {
    await page.getByLabel('Reviewer name').fill('Ritwik');
  });
  // Chromium leaves iframes below the first viewport blank in a full-page capture; take the
  // second card on its own so its previews are visible.
  await page.getByTestId('card-popup_offer').screenshot({ path: `${out}/${theme}-5b-approvals-blocked-card.png` });
  await page.getByTestId('card-gift_box').screenshot({ path: `${out}/${theme}-5c-approvals-agent-card.png` });
  const audit = await page.getByTestId('card-gift_box').getByTestId('audit-link').getAttribute('href');
  await shot('5d-audit', audit);
  await shot('6-install', '/app/install/?site=shop');
  // The kill switch, pulled and then put back.
  await shot('7-paused', '/app/site/?id=shop', async () => {
    const banner = page.getByTestId('live-banner');
    const name = banner.getByLabel('Your name for the kill switch');
    if (await name.count()) await name.fill('Ritwik');
    await banner.getByRole('button', { name: 'Pause all slots' }).click();
    await page.waitForTimeout(500);
  });
  await page.getByRole('button', { name: 'Resume all slots' }).click();
  await page.waitForTimeout(300);
  await page.getByTestId('connect-agent').screenshot({ path: `${out}/${theme}-8-connect-agent.png` });
  await page.getByTestId('mcp-tail').screenshot({ path: `${out}/${theme}-9-mcp-tail.png` });
  // Webhooks: one endpoint that receives (the stand-in site answers 200), with deliveries open.
  if (theme === 'light') {
    const H = { 'content-type': 'application/json', authorization: 'Bearer dash-admin-token' };
    await fetch(`${BASE}/api/sites/shop/webhooks`, { method: 'POST', headers: H, body: JSON.stringify({ url: 'http://127.0.0.1:3400/hooks/pratibimba', events: ['decision', 'conversion', 'auto_rollback'] }) });
    for (let i = 0; i < 3; i++) {
      await fetch(`${BASE}/decide`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ site: 'shop', slot: 'hero', subject: `shot-${i}`, ctx: { returning: false } }) });
    }
    await new Promise((r) => setTimeout(r, 1500));
  }
  await shot('10-webhooks', '/app/webhooks/?site=shop', async () => {
    await page.getByRole('button', { name: 'Deliveries' }).first().click();
    await page.waitForTimeout(800);
  });
  await ctx.close();
}
await b.close();
console.log(errors.length ? errors.join('\n') : 'no page errors');
