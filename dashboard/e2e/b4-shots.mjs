// B4 screenshots against the e2e stack (node e2e/stack.mjs, already running): the persona picker
// in Preview and a templated draft's approval card, light and dark. Usage: node e2e/b4-shots.mjs outdir
import { chromium } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const OUT = process.argv[2] ?? '/tmp/b4-shots';
const CORE = 'http://127.0.0.1:18083';
const TOKEN = 'dash-admin-token';
const DB = (process.env.PRB_E2E_ADMIN_DB ?? 'postgres://postgres@localhost:55440/postgres').replace(/\/[^/]*$/, '/prb_dash_e2e');
const file = { site: 'dhaba', origin: 'http://localhost:3400', privacy_mode: 'full', slots: [
  { key: 'hero_copy', selector: '#hero', kind: 'copy', copy: { fields: [{ name: 'title', max: 40, required: true }] },
    variants: [{ key: 'own', baseline: true }, { key: 'your_usual', requires: ['visitor.top_item'], copy: { title: 'Your usual {visitor.top_item.name | truncate: 14} is ready' } }] },
] };
const tmp = path.join(mkdtempSync(path.join(tmpdir(), 'prb-b4s-')), 'site.json');
writeFileSync(tmp, JSON.stringify(file));
execFileSync(path.resolve(import.meta.dirname, '../../target/debug/pratibimba'), ['apply', tmp], { env: { ...process.env, PRB_DB: DB, RUST_LOG: 'warn' }, stdio: 'pipe' });
const auth = { authorization: `Bearer ${TOKEN}`, 'content-type': 'application/json' };
await fetch(`${CORE}/api/sites/dhaba/slots/hero_copy/variants`, { method: 'POST', headers: auth, body: JSON.stringify({ key: 'usual_again', html: JSON.stringify({ title: 'Your usual {visitor.top_item.name | truncate: 14}, again?' }), requires: ['visitor.top_item'] }) });

const browser = await chromium.launch();
for (const scheme of ['light', 'dark']) {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: scheme });
  const page = await ctx.newPage();
  await page.request.post(`${CORE}/api/session`, { data: { token: TOKEN } });
  await page.goto(`${CORE}/app/preview/?site=dhaba`);
  await page.getByTestId('persona-usual_rajma').click();
  await page.waitForTimeout(2500);
  await page.getByTestId('variant-your_usual').click();
  await page.waitForTimeout(2000);
  await page.screenshot({ path: `${OUT}/preview-persona-${scheme}.png` });
  await page.getByRole('button', { name: 'First visit' }).click();
  await page.waitForTimeout(2500);
  await page.screenshot({ path: `${OUT}/preview-first-visit-${scheme}.png` });
  await page.goto(`${CORE}/app/approvals/?site=dhaba`);
  const card = page.getByTestId('card-usual_again');
  await card.waitFor();
  await card.screenshot({ path: `${OUT}/approval-card-${scheme}.png` });
  await page.goto(`${CORE}/app/drafts/?site=dhaba`);
  await page.getByTestId('history-settings').getByRole('radio', { name: 'A session' }).click();
  await page.getByTestId('history-settings').screenshot({ path: `${OUT}/history-settings-${scheme}.png` });
  await ctx.close();
}
await browser.close();
