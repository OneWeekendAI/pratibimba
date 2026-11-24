// Capture four visitor kinds on the live store (real browser signals: phone UA, ad query, referrer, returning, clock),
// record what each was shown per slot and Core's own plain-English "why" for it. Output: proof/personas/*.png and
// demo/personas.json (read by scripts/build-demo.mjs). Needs Core + Vite already running.
import { chromium } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const here = import.meta.dirname, STORE = process.env.STORE ?? 'http://localhost:5174', CORE = process.env.CORE ?? 'http://localhost:8080';
const TOKEN = process.env.PRB_ADMIN_TOKEN ?? 'local-admin-token';
const cat = JSON.parse(readFileSync(path.join(here, '../src/data/catalogue.json'), 'utf8'));
const LOW = cat.find((p) => p.stock <= 4 && p.price >= 499).id;
const MOBILE_UA = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1';
const PERSONAS = [
  { key: 'bargain', name: 'Asha, on her phone', about: 'Tapped an Instagram ad, on mobile, mid-morning', mobile: true, q: '?utm_source=ig&utm_medium=cpc', ref: '', at: '2026-11-24T10:30:00+05:30', returning: false },
  { key: 'gift', name: 'Rohan, back for a gift', about: 'Returning visitor, evening, came via Google', mobile: false, q: '', ref: 'https://www.google.com/search?q=gift', tz: 'Pacific/Kiritimati', returning: true },
  { key: 'premium', name: 'Meera, at her desk', about: 'Desktop, typed the address in, mid-day', mobile: false, q: '', ref: '', at: '2026-11-24T12:15:00+05:30', returning: false },
  { key: 'newbie', name: 'Kabir, first visit', about: 'New visitor from a Google search, afternoon', mobile: true, q: '', ref: 'https://www.google.com/search?q=cookware', at: '2026-11-24T15:00:00+05:30', returning: false },
];
const SLOTS = ['promo-bar', 'hero', 'category-rows', 'picks', 'deals', 'listing-sort', 'product-badge', 'buy-button', 'delivery-message', 'cart-cross-sell'];
const explain = async (id) => { const r = await fetch(`${CORE}/api/decisions/${id}`, { headers: { authorization: `Bearer ${TOKEN}` } }); return r.ok ? (await r.json()) : null; };

const browser = await chromium.launch();
const sim = JSON.parse(readFileSync(path.join(here, '../proof/sim-haat.json'), 'utf8'));
// One browser visit is one random draw. Keep the visit whose hero is what Core serves that kind most often
// (the served_modal graded in the simulation), so the page shows the typical shopper; the tries are recorded.
const modalHero = (seg) => sim.graded.find((g) => g.segment === seg && g.slot === 'hero').served_modal;
const out = [];
async function capture(P) {
  const ctx = await browser.newContext({
    viewport: P.mobile ? { width: 390, height: 844 } : { width: 1280, height: 900 }, userAgent: P.mobile ? MOBILE_UA : undefined,
    isMobile: P.mobile, hasTouch: P.mobile, timezoneId: P.tz ?? 'Asia/Kolkata', extraHTTPHeaders: P.ref ? { referer: P.ref } : undefined,
  });
  // Core reads the hour from its own clock plus the visitor's tz_offset, so "evening" is a timezone where it is evening now.
  // "Returning" means the cookie predates this browser session: visit once in a tab, close it, and come back in a new tab.
  if (P.returning) { const warm = await ctx.newPage(); await warm.goto(`${STORE}/${P.q}#/`, { referer: P.ref || undefined }); await warm.waitForTimeout(1500); await warm.close(); }
  const page = await ctx.newPage();
  const decisions = {};
  page.on('response', async (r) => {
    if (!/\/_prb\/decide/.test(r.url())) return;
    try { const j = await r.json(); for (const d of j.decisions ?? [j]) if (d?.slot) decisions[d.slot] = { id: d.decision_id, variant: d.variant, order: d.order }; } catch { /* not json */ }
  });
  const go = async (hash, slot) => { await page.goto(`${STORE}/${P.q}${hash}`, { referer: P.ref || undefined }); await page.waitForFunction((s) => document.querySelector(`[data-prb-slot=${s}]`)?.hasAttribute('data-prb-done'), slot, { timeout: 8000 }); await page.waitForTimeout(400); };
  await go('#/', 'hero');
  await page.screenshot({ path: path.join(here, `../proof/personas/${P.key}-home.png`) });
  const home = await page.evaluate(() => ({
    promo: document.querySelector('[data-prb-slot=promo-bar]').textContent.trim(),
    hero: document.querySelector('[data-prb-slot=hero] h1').textContent.trim(),
    rows: [...document.querySelectorAll('[data-prb-slot=category-rows] > [data-prb-item]')].map((e) => e.dataset.prbItem),
    picks: [...document.querySelectorAll('[data-prb-slot=picks] > [data-prb-item]')].slice(0, 4).map((e) => e.dataset.prbItem),
  }));
  await page.goto(`${STORE}/${P.q}#/product/${LOW}`); await page.waitForFunction(() => document.querySelector('[data-prb-slot=product-badge]')?.hasAttribute('data-prb-done')); await page.waitForTimeout(400);
  await page.screenshot({ path: path.join(here, `../proof/personas/${P.key}-product.png`) });
  const prod = await page.evaluate(() => ({ badge: document.querySelector('[data-testid=badge]').textContent.trim(), buy: document.querySelector('[data-testid=buy]').textContent.trim(), delivery: document.querySelector('[data-testid=delivery]').textContent.trim() }));
  await page.goto(`${STORE}/${P.q}#/c/kitchen`); await page.waitForFunction(() => document.querySelector('[data-prb-slot=listing-sort]')?.hasAttribute('data-prb-done')); await page.waitForTimeout(300);
  const sort = await page.evaluate(() => document.querySelector('[data-testid=sort]').value);
  const why = {};
  for (const s of ['hero', 'category-rows', 'product-badge']) { const d = decisions[s]; if (d) { const e = await explain(d.id); why[s] = e?.sentence ?? e?.explanation ?? null; } }
  await ctx.close();
  return { ...P, home, product: prod, sort, variants: Object.fromEntries(SLOTS.map((s) => [s, decisions[s]?.variant ?? null])), why };
}
for (const P of PERSONAS) {
  let r, tries = 0;
  do { r = await capture(P); tries++; } while (r.variants.hero !== modalHero(P.key) && tries < 8);
  out.push({ ...r, tries });
}
await browser.close();
writeFileSync(path.join(here, '../demo/personas.json'), JSON.stringify(out, null, 1));
console.log(out.map((p) => `${p.key}: hero=${p.variants.hero} sort=${p.variants['listing-sort']} badge=${p.variants['product-badge']} promo=${p.variants['promo-bar']} rows[0]=${p.home.rows[0]} why=${JSON.stringify(p.why.hero)?.slice(0, 60)}`).join('\n'));
