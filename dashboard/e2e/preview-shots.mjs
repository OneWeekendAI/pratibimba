// Screenshots of the Preview room against the local stack (Core :8080, the kitchen demo :5173).
// The page's /_prb/sdk.js is served from packages/client, so a Core that has not been restarted
// still shows the explorer as the current SDK draws it. Usage: node e2e/preview-shots.mjs [outdir]
import { chromium } from '@playwright/test';
import { readFileSync } from 'node:fs';
import path from 'node:path';

const OUT = process.argv[2] ?? '/tmp/preview-shots';
const SDK = readFileSync(path.resolve(import.meta.dirname, '../../packages/client/sdk.js'), 'utf8');
const BASE = process.env.PRB_CORE ?? 'http://localhost:8080';
const TOKEN = process.env.PRB_ADMIN_TOKEN ?? 'local-admin-token';
const SITE = process.env.PRB_SITE ?? 'kitchen';

const browser = await chromium.launch();
for (const [tag, opts] of [
  ['desktop-light', { viewport: { width: 1440, height: 900 }, colorScheme: 'light' }],
  ['desktop-dark', { viewport: { width: 1440, height: 900 }, colorScheme: 'dark' }],
  ['phone', { viewport: { width: 390, height: 844 }, colorScheme: 'light', isMobile: true, hasTouch: true, deviceScaleFactor: 2 }],
]) {
  const ctx = await browser.newContext(opts);
  await ctx.route('**/_prb/sdk.js', (r) => r.fulfill({ contentType: 'application/javascript', body: SDK }));
  const page = await ctx.newPage();
  await page.request.post(`${BASE}/api/session`, { data: { token: TOKEN } });
  await page.goto(`${BASE}/app/preview/?site=${SITE}`);
  const slot = process.env.PRB_SLOT ?? 'hero_copy';
  await page.getByTestId(`slot-tab-${slot}`).click();
  await page.waitForTimeout(3500);
  if (process.env.PRB_PICK) await page.getByTestId(`variant-${process.env.PRB_PICK}`).click();
  await page.waitForTimeout(2500);
  await page.screenshot({ path: `${OUT}/${tag}${process.env.PRB_SUFFIX ?? ''}.png` });
  await ctx.close();
}
await browser.close();
