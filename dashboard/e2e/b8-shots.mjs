// B8 screenshots against the e2e stack (node e2e/stack.mjs, already running): the advisor's pause
// card in Review, its line on Results after approval, and the Advisor settings panel, light and
// dark. Usage: node e2e/b8-shots.mjs outdir
import { chromium } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const OUT = process.argv[2] ?? '/tmp/b8-shots';
const CORE = 'http://127.0.0.1:18083';
const TOKEN = 'dash-admin-token';
const DB = (process.env.PRB_E2E_ADMIN_DB ?? 'postgres://postgres@localhost:55440/postgres').replace(/\/[^/]*$/, '/prb_dash_e2e');
const psql = (sql) => execFileSync('psql', [DB, '-v', 'ON_ERROR_STOP=1', '-qAtc', sql], { stdio: 'pipe' }).toString().trim();
const file = { site: 'kitchen', origin: 'http://localhost:3400', privacy_mode: 'full', slots: [
  { key: 'kitchen_strip', selector: '#strip', metric: 'purchase', variants: [
    { key: 'own', baseline: true }, { key: 'function_orders', html: '<p>We cater functions</p>' }, { key: 'fresh_daily', html: '<p>Cooked fresh daily</p>' }] },
  { key: 'hero', selector: '#hero', metric: 'purchase', variants: [{ key: 'own', baseline: true }, { key: 'thali', html: '<h1>Thali today</h1>' }] },
] };
const tmp = path.join(mkdtempSync(path.join(tmpdir(), 'prb-b8s-')), 'site.json');
writeFileSync(tmp, JSON.stringify(file));
execFileSync(path.resolve(import.meta.dirname, '../../target/debug/pratibimba'), ['apply', tmp], { env: { ...process.env, PRB_DB: DB, RUST_LOG: 'warn' }, stdio: 'pipe' });
const evidence = {
  claim: "kitchen_strip: all 2 versions are within ±5 orders per 100 of the page's own over 8,147 visits. It makes no difference.",
  rows: 8147, visits_per_week: 1900, margin: 5, p: 0.004, state: 'confident',
  arms: [{ key: 'function_orders', uplift: 1.44, lo: -1.22, hi: 4.11, n: 2700 }, { key: 'fresh_daily', uplift: -1.58, lo: -4.77, hi: 1.61, n: 2650 }],
  held_out: [{ key: 'function_orders', uplift: -1.09, lo: -5, hi: 3, n: 900 }, { key: 'fresh_daily', uplift: 0.72, lo: -3, hi: 4, n: 880 }],
};
psql(`INSERT INTO advisor_cards (site_id, slot_id, kind, state, evidence, created_at) SELECT 'kitchen', id, 'pause_slot', 'confident', '${JSON.stringify(evidence).replace(/'/g, "''")}', now() - interval '3 hours' FROM slots WHERE site_id = 'kitchen' AND key = 'kitchen_strip' ON CONFLICT DO NOTHING`);
psql(`UPDATE sites SET advisor_settings = '{"margin": 5}' WHERE id = 'kitchen'`);
const auth = { authorization: `Bearer ${TOKEN}`, 'content-type': 'application/json' };

const browser = await chromium.launch();
const shoot = async (scheme, fn) => {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 }, colorScheme: scheme, deviceScaleFactor: 2 });
  const page = await ctx.newPage();
  await page.request.post(`${CORE}/api/session`, { data: { token: TOKEN } });
  await page.addInitScript(() => localStorage.setItem('prb-approver', 'Asha'));
  await fn(page);
  await ctx.close();
};
for (const scheme of ['light', 'dark']) {
  await shoot(scheme, async (page) => {
    await page.goto(`${CORE}/app/approvals/?site=kitchen`);
    const card = page.getByTestId('advisor-card-kitchen_strip');
    await card.waitFor();
    await page.screenshot({ path: `${OUT}/review-${scheme}.png` });
    await card.getByRole('button', { name: 'Details' }).click();
    await card.screenshot({ path: `${OUT}/advisor-card-${scheme}.png` });
    const panel = page.getByTestId('advisor-settings');
    await panel.scrollIntoViewIfNeeded();
    await panel.getByLabel('About margin (per 100)').focus();
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${OUT}/settings-${scheme}.png`, clip: await panel.boundingBox().then((b) => ({ x: b.x - 8, y: b.y - 8, width: b.width + 16, height: b.height + 90 })) });
  });
}
// Approve, then give the slot ten days on each side of the pause, treated and held out.
const id = psql(`SELECT id FROM advisor_cards WHERE site_id = 'kitchen' AND status = 'pending'`);
if (id) await fetch(`${CORE}/api/advisor-cards/${id}/approve`, { method: 'POST', headers: auth, body: JSON.stringify({ actor: 'Asha' }) });
psql(`UPDATE advisor_cards SET decided_at = now() - interval '10 days' WHERE site_id = 'kitchen'`);
psql(`INSERT INTO decisions (id, site_id, slot_id, variant_id, subject, pool, features, scores, policy, policy_version, seed, in_holdout, created_at, exposed_at, outcome, settled_at)
      SELECT gen_random_uuid(), 'kitchen', s.id, v.id, 'k' || g, '{}', '{}', '{}', 'lints', 0, g, g % 10 = 0,
             now() - make_interval(hours => 1 + g % 480), now() - make_interval(hours => 1 + g % 480),
             CASE WHEN random() < 0.10 THEN 1 ELSE 0 END, now()
        FROM slots s JOIN variants v ON v.slot_id = s.id AND v.key = 'own', generate_series(1, 6000) g
       WHERE s.site_id = 'kitchen' AND s.key = 'kitchen_strip' AND NOT EXISTS (SELECT 1 FROM decisions WHERE site_id = 'kitchen')`);
for (const scheme of ['light', 'dark']) {
  await shoot(scheme, async (page) => {
    await page.goto(`${CORE}/app/results/?site=kitchen`);
    const slot = page.getByTestId('result-kitchen_strip');
    await slot.getByTestId('advisor-line').waitFor();
    await slot.screenshot({ path: `${OUT}/results-line-${scheme}.png` });
    await page.getByTestId('result-hero').screenshot({ path: `${OUT}/results-not-enough-${scheme}.png` });
  });
}
await browser.close();
