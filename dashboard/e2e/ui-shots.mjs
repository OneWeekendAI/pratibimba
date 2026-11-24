// The UI pass's screenshots: every page, light and dark, desktop (1440) and phone (390).
// `PRB_DB=<db url> node e2e/ui-shots.mjs <outdir>` against a running stack (stack.mjs).
// Seeds the extras the stack leaves out (a copy site, audiences, a copy draft) once, idempotently.
import { chromium } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const out = process.argv[2] ?? 'screenshots';
mkdirSync(out, { recursive: true });
const BASE = 'http://127.0.0.1:18083';
const H = { 'content-type': 'application/json', authorization: 'Bearer dash-admin-token' };
const BIN = path.resolve(import.meta.dirname, '../../target/debug/pratibimba');
const post = async (p, body) => { const r = await fetch(BASE + p, { method: 'POST', headers: H, body: JSON.stringify(body) }); return r.ok ? r.json().catch(() => null) : null; };
const get = (p) => fetch(BASE + p, { headers: H }).then((r) => r.json());

await fetch('http://127.0.0.1:3400/__seed-sorted');
await fetch('http://127.0.0.1:3400/__seed-kiosk');
if (process.env.PRB_DB) {
  const file = { site: 'dhaba', origin: 'http://localhost:3400', privacy_mode: 'full', slots: [
    { key: 'hero_copy', selector: '#hero', kind: 'copy', copy: { fields: [{ name: 'title', max: 40, required: true }, { name: 'cta_label', max: 24 }] },
      variants: [{ key: 'own', baseline: true }, { key: 'your_usual', requires: ['visitor.top_item'], copy: { title: 'Your usual {visitor.top_item.name | truncate: 14} is ready', cta_label: 'Order again' } }] },
  ] };
  const tmp = path.join(mkdtempSync(path.join(tmpdir(), 'prb-ui-')), 'site.json');
  writeFileSync(tmp, JSON.stringify(file));
  try { execFileSync(BIN, ['apply', tmp], { env: { ...process.env, RUST_LOG: 'warn' }, stdio: 'pipe' }); } catch {}
}
const dr = await get('/api/sites/dhaba/slots/hero_copy').catch(() => null);
if (dr && !(dr.variants ?? []).some((v) => v.key === 'monday_special')) {
  await post('/api/sites/dhaba/slots/hero_copy/variants', { key: 'monday_special', html: JSON.stringify({ title: 'Monday thali, hot at noon', cta_label: 'See the menu' }), note: 'Weekday lunch push', author_name: 'Asha' });
}
// Audiences on shop's hero: one promoted, the gift variant gated to it.
const hero = await get('/api/sites/shop/slots/hero').catch(() => null);
if (hero && !(hero.audiences ?? []).length) {
  const seg = await post('/api/sites/shop/segments', { predicate: { purchase_count: { gte: 2 } }, actor: 'Ritwik' });
  if (seg) await post(`/api/segments/${seg.id}/promote`, { actor: 'Ritwik' });
  const seg2 = await post('/api/sites/shop/segments', { predicate: { days_since_last_purchase: { gte: 45 } }, actor: 'Ritwik' });
  if (seg2) await post(`/api/segments/${seg2.id}/promote`, { actor: 'Ritwik' });
}

const b = await chromium.launch();
const errors = [];
const sizes = { desktop: { width: 1440, height: 900 }, phone: { width: 390, height: 844 } };
for (const theme of ['light', 'dark']) for (const [sz, viewport] of Object.entries(sizes)) {
  const ctx = await b.newContext({ viewport, colorScheme: theme, deviceScaleFactor: 1 });
  const page = await ctx.newPage();
  page.on('pageerror', (e) => errors.push(`${theme}/${sz} ${page.url()}: ${e.message}`));
  await page.goto(`${BASE}/app/login/`);
  await page.screenshot({ path: `${out}/${theme}-${sz}-00-login.png` });
  await page.request.post(`${BASE}/api/session`, { data: { token: 'dash-admin-token' } });
  await page.goto(`${BASE}/app/site/?id=shop`);
  await page.evaluate((t) => { document.documentElement.dataset.theme = t; localStorage.setItem('prb-theme', t); localStorage.setItem('prb-approver', 'Ritwik'); }, theme);
  const shot = async (name, url, prep) => {
    await page.goto(BASE + url);
    await page.waitForLoadState('networkidle').catch(() => {});
    await page.waitForTimeout(1500);
    if (prep) await prep().catch((e) => errors.push(`prep ${name}: ${e.message}`));
    await page.evaluate(async () => { for (let y = 0; y < document.body.scrollHeight; y += 500) { scrollTo(0, y); await new Promise((r) => setTimeout(r, 80)); } scrollTo(0, 0); });
    await page.screenshot({ path: `${out}/${theme}-${sz}-${name}.png`, fullPage: true });
  };
  await shot('01-site-home', '/app/site/?id=shop');
  await shot('02-slot-hero', '/app/slot/?site=shop&slot=hero');
  await shot('03-slot-banner', '/app/slot/?site=shop&slot=banner');
  await shot('04-slot-sorted', '/app/slot/?site=rack&slot=grid');
  await shot('05-slot-copy', '/app/slot/?site=kiosk&slot=tagline');
  await shot('06-slot-copy-dhaba', '/app/slot/?site=dhaba&slot=hero_copy');
  await shot('07-results', '/app/results/?site=shop');
  await shot('07b-results-open', '/app/results/?site=shop', async () => { for (const d of await page.getByRole('button', { name: 'Details' }).all()) await d.click(); await page.waitForTimeout(500); });
  await shot('08-results-sorted', '/app/results/?site=rack');
  await shot('09-approvals', '/app/approvals/?site=shop');
  await shot('10-approvals-copy', '/app/approvals/?site=dhaba');
  await shot('11-drafts', '/app/drafts/?site=shop');
  await shot('12-drafts-copy', '/app/drafts/?site=dhaba');
  await shot('13-preview', '/app/preview/?site=shop');
  await shot('14-install', '/app/install/?site=shop');
  await shot('15-agent', '/app/agent/?site=shop');
  await shot('16-webhooks', '/app/webhooks/?site=shop');
  await shot('17-time-travel', '/app/time-travel/?site=shop');
  await page.goto(`${BASE}/app/approvals/?site=shop`);
  await page.waitForTimeout(1500);
  const audit = await page.getByTestId('audit-link').first().getAttribute('href').catch(() => null);
  await shot('18-audit', audit ? (audit.startsWith('/app/') ? audit : '/app' + audit) : '/app/audit/?id=1');
  await shot('18b-audit-log', '/app/audit/?site=shop');
  await shot('18c-audit-bare', '/app/audit/');
  await shot('20-home', '/app/');
  await ctx.close();
}
await b.close();
console.log(errors.length ? errors.join('\n') : 'no page errors');
