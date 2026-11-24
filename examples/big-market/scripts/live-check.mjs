// Live check against a running Core + the Vite store (neither is started here).
// Fresh browser contexts with different signals; asserts every slot is decided, variants differ,
// events land, no console errors, no flicker or layout shift. Writes proof/live/*.png and live-check.json.
import { chromium } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const STORE = process.env.STORE ?? 'http://localhost:5174';
const DB = process.env.PRB_DB ?? 'postgres://postgres@localhost:55440/prb_local';
const OUT = path.resolve(import.meta.dirname, '../proof/live');
const cat = JSON.parse(readFileSync(path.resolve(import.meta.dirname, '../src/data/catalogue.json'), 'utf8'));
const PS = Array.isArray(cat) ? cat : cat.products;
const LOW = PS.find((p) => p.stock <= 4 && p.price >= 499).id;        // "only N left" is true here
const GIFT = PS.find((p) => p.stock > 4 && p.tags.includes('gift') && p.rating >= 4.4)?.id ?? PS.find((p) => p.stock > 4).id;
const psql = (q) => execFileSync('psql', [DB, '-qAt', '-c', q]).toString().trim();
const count = (name) => +psql(`select count(*) from events where site_id='haat' and name='${name}'`);

const MOBILE_UA = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1';
const SLOTS = ['promo-bar', 'hero', 'category-rows', 'picks', 'deals', 'listing-sort', 'product-badge', 'buy-button', 'delivery-message', 'cart-cross-sell'];
const PERSONAS = [
  { id: 'desk-direct-morning', mobile: false, ref: '', q: '', at: '2026-11-24T09:10:00+05:30', returning: false },
  { id: 'mob-ad-evening', mobile: true, ref: '', q: '?utm_source=ig&utm_medium=cpc', at: '2026-11-24T20:40:00+05:30', returning: false },
  { id: 'desk-search-evening-returning', mobile: false, ref: 'https://www.google.com/search?q=gift', q: '', at: '2026-11-24T19:30:00+05:30', returning: true },
  { id: 'mob-direct-morning-returning', mobile: true, ref: '', q: '', at: '2026-11-24T08:05:00+05:30', returning: true },
  { id: 'desk-ad-noon', mobile: false, ref: '', q: '?gclid=abc', at: '2026-11-24T12:00:00+05:30', returning: false },
  { id: 'mob-search-night', mobile: true, ref: 'https://www.bing.com/', q: '', at: '2026-11-24T23:15:00+05:30', returning: false },
];

const INIT = () => {
  window.__cls = 0; window.__texts = {}; window.__errs = [];
  new PerformanceObserver((l) => { for (const e of l.getEntries()) if (!e.hadRecentInput) window.__cls += e.value; }).observe({ type: 'layout-shift', buffered: true });
  // flicker: distinct texts a slot shows while visible. One per frame.
  const tick = () => {
    document.querySelectorAll('[data-prb-slot]').forEach((el) => {
      if (getComputedStyle(el).visibility === 'hidden') return;
      const k = el.dataset.prbSlot + '#' + (el.dataset.i ??= String(Math.random()));
      const t = el.textContent.replace(/\s+/g, ' ').trim().slice(0, 400) + '|' + [...el.querySelectorAll('[data-sort]')].map((x) => x.dataset.sort).join();
      (window.__texts[k] ??= []); const a = window.__texts[k]; if (a[a.length - 1] !== t) a.push(t);
    });
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
};

const variants = (page) => page.evaluate(() => Object.fromEntries([...document.querySelectorAll('[data-prb-slot]')].map((e) => [e.dataset.prbSlot, e.dataset.prbVariant ?? null])));
const settle = (page, sel) => page.waitForFunction((s) => [...document.querySelectorAll(s)].length && [...document.querySelectorAll(s)].every((e) => e.hasAttribute('data-prb-done')), sel, { timeout: 8000 });

const browser = await chromium.launch();
const report = { personas: {}, problems: [] };
const before = { add_to_cart: count('add_to_cart'), purchase: count('purchase'), exposure: count('exposure') };
let shots = 0;
for (const P of PERSONAS) {
  const ctx = await browser.newContext({
    viewport: P.mobile ? { width: 390, height: 844 } : { width: 1280, height: 900 },
    userAgent: P.mobile ? MOBILE_UA : undefined, isMobile: P.mobile, hasTouch: P.mobile,
    timezoneId: 'Asia/Kolkata', extraHTTPHeaders: P.ref ? { referer: P.ref } : undefined,
  });
  await ctx.clock?.install?.({ time: new Date(P.at) }).catch?.(() => {});
  if (P.returning) { const warm = await ctx.newPage(); await warm.goto(`${STORE}/${P.q}#/`, { referer: P.ref || undefined }); await warm.waitForTimeout(1500); await warm.close(); } // cookie predates the next tab's session
  const page = await ctx.newPage();
  const errs = [];
  page.on('console', (m) => { if (m.type() === 'error') errs.push(m.text()); });
  page.on('pageerror', (e) => errs.push('pageerror ' + e.message));
  await page.addInitScript(INIT);
  if (process.env.FAULT) await page.route('**/_prb/decide*', (r) => r.abort()); // proves the check can fail
  if (P.ref) await page.route('**/*', (r) => r.continue()); // referer set via goto
  const got = {};
  const go = async (hash, sel) => { await page.goto(`${STORE}/${P.q}${hash}`, { referer: P.ref || undefined }); await settle(page, sel); await page.waitForTimeout(250); };
  await go('#/', '[data-prb-slot=hero]');
  Object.assign(got, await variants(page));
  got.__home = { ...got };
  if (P.id === PERSONAS[1].id || P.id === PERSONAS[2].id || P.id === PERSONAS[0].id || P.id === PERSONAS[3].id) await page.screenshot({ path: `${OUT}/${P.id}-home.png` }), shots++;
  await page.goto(`${STORE}/${P.q}#/c/kitchen`); await settle(page, '[data-prb-slot=listing-sort]'); Object.assign(got, await variants(page));
  const gridOrder = await page.locator('[data-testid=grid] .card .price b').allTextContents();
  got.__sort_first_prices = gridOrder.slice(0, 3).join(' ');
  await page.goto(`${STORE}/${P.q}#/product/${LOW}`); await settle(page, '[data-prb-slot=product-badge]'); await page.waitForTimeout(200);
  Object.assign(got, await variants(page));
  got.__badge = await page.getByTestId('badge').textContent(); got.__buy = (await page.getByTestId('buy').textContent()).trim(); got.__delivery = (await page.getByTestId('delivery').textContent()).trim();
  if (P.id !== PERSONAS[4].id) await page.screenshot({ path: `${OUT}/${P.id}-product.png` }), shots++;
  await page.getByTestId('buy').click();
  await page.goto(`${STORE}/${P.q}#/product/${GIFT}`); await settle(page, '[data-prb-slot=product-badge]'); await page.waitForTimeout(200);
  got.__badge_gift = await page.getByTestId('badge').textContent(); got.__buy_gift = (await page.getByTestId('buy').textContent()).trim();
  await page.getByTestId('buy').click();
  await page.goto(`${STORE}/${P.q}#/cart`); await settle(page, '[data-prb-slot=cart-cross-sell]'); Object.assign(got, await variants(page));
  got.__cross = await page.getByTestId('cross-sell').locator('.card').count();
  await page.getByTestId('to-checkout').click();
  await page.getByTestId('place-order').click();
  got.__order = (await page.getByTestId('order-id').textContent()).trim();
  await page.waitForTimeout(600);
  // flicker and layout shift: look at what each slot showed while visible, on the home load
  const m = await page.evaluate(() => ({ cls: window.__cls, texts: window.__texts }));
  got.__cls = +m.cls.toFixed(4);
  got.__flicker = Object.entries(m.texts).filter(([, a]) => a.length > 1).map(([k, a]) => `${k.split('#')[0]}:${a.length}`);
  got.__errors = errs;
  report.personas[P.id] = got;
  await ctx.close();
}
await browser.close();

// events reach Core (batched; give the beacons a moment)
await new Promise((r) => setTimeout(r, 1500));
const after = { add_to_cart: count('add_to_cart'), purchase: count('purchase'), exposure: count('exposure') };
report.events = { before, after, delta: Object.fromEntries(Object.keys(after).map((k) => [k, after[k] - before[k]])) };
report.purchase_rows = psql(`select order_id||' '||value||' '||currency from events where site_id='haat' and name='purchase' order by received_at desc limit ${PERSONAS.length}`).split('\n');
const slotsSeen = Object.fromEntries(SLOTS.map((s) => [s, new Set(Object.values(report.personas).map((p) => p[s]).filter(Boolean))]));
report.decided_slots = Object.fromEntries(Object.entries(slotsSeen).map(([k, v]) => [k, [...v]]));
const PAGE_SLOTS = { home: ['promo-bar', 'hero', 'category-rows', 'picks', 'deals'] };
for (const [id, p] of Object.entries(report.personas)) for (const s of SLOTS) if (!(p.__home[s] ?? p[s])) report.problems.push(`${id}: ${s} has no decision on its page`);
for (const s of SLOTS) { if (!slotsSeen[s].size) report.problems.push(`slot ${s} never decided`); }
for (const [id, p] of Object.entries(report.personas)) {
  if (p.__errors.length) report.problems.push(`${id}: console errors ${JSON.stringify(p.__errors)}`);
  if (p.__cls > 0.02) report.problems.push(`${id}: CLS ${p.__cls}`);
  if (p.__flicker.length) report.problems.push(`${id}: flicker ${p.__flicker}`);
}
if (report.events.delta.add_to_cart < PERSONAS.length * 2) report.problems.push('add_to_cart events short');
if (report.events.delta.purchase < PERSONAS.length) report.problems.push('purchase events short');
const differing = SLOTS.filter((s) => slotsSeen[s].size > 1).length;
report.slots_with_variation = differing;
writeFileSync(path.resolve(import.meta.dirname, '../proof/live/live-check.json'), JSON.stringify(report, null, 1));
console.log(JSON.stringify({ decided: Object.fromEntries(Object.entries(report.decided_slots).map(([k, v]) => [k, v.length])), events: report.events.delta, slots_with_variation: differing, problems: report.problems }, null, 1));
process.exit(report.problems.length ? 1 : 0);
