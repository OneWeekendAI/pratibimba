// Asserts site.json and the rendered markup agree. Expects the dev server up (npm run dev, port 5174).
//  - every slot in site.json has its selector matching exactly one element on some page
//  - every [data-prb-slot] element in the markup is a slot in site.json
//  - variants are well-formed (one baseline except sorted, unique slug keys, copy fits the fields)
//  - sorted: every marked item is a declared variant; copy: every field exists; swap: html present
// Usage: node scripts/check-slots.mjs     (BASE=http://localhost:5174 to override)
import { chromium } from '@playwright/test';
import { readFileSync } from 'node:fs';

const BASE = process.env.BASE || 'http://localhost:5174';
const site = JSON.parse(readFileSync(new URL('../site.json', import.meta.url), 'utf8'));
const errs = [];
const bad = (m) => errs.push(m);
const slug = /^[a-z0-9_-]{1,64}$/;

// ---- site.json shape ----
if (!slug.test(site.site || '')) bad('site id is not a slug');
const keys = new Set();
for (const s of site.slots) {
  if (!slug.test(s.key)) bad(`${s.key}: key is not a slug`);
  if (keys.has(s.key)) bad(`${s.key}: duplicate slot`);
  keys.add(s.key);
  if (s.selector !== `[data-prb-slot=${s.key}]`) bad(`${s.key}: selector ${s.selector} is not the [data-prb-slot=key] convention`);
  const kind = s.kind || 'swap';
  const vk = new Set();
  const baselines = s.variants.filter((v) => v.baseline);
  for (const v of s.variants) {
    if (!slug.test(v.key)) bad(`${s.key}/${v.key}: variant key is not a slug`);
    if (vk.has(v.key)) bad(`${s.key}/${v.key}: duplicate variant`);
    vk.add(v.key);
    if (v.baseline && (v.html || v.copy)) bad(`${s.key}/${v.key}: baseline must carry no html or copy`);
    if (kind === 'swap' && !v.baseline && !(typeof v.html === 'string' && v.html.trim())) bad(`${s.key}/${v.key}: swap variant needs html`);
    if (kind === 'copy' && !v.baseline) {
      for (const f of s.copy.fields) {
        const t = v.copy?.[f.name];
        if (typeof t !== 'string' || !t.trim()) bad(`${s.key}/${v.key}: field ${f.name} missing`);
        else {
          const shown = t.replace(/\{\{/g, '{').replace(/\}\}/g, '}');
          if (shown.length > f.max) bad(`${s.key}/${v.key}: ${f.name} is ${shown.length} chars, max ${f.max}`);
          if (/(^|[^{])\{[^{]/.test(t.replace(/\{\{|\}\}/g, ''))) bad(`${s.key}/${v.key}: lone brace in ${f.name} (write {{ }})`);
        }
      }
    }
  }
  if (kind === 'sorted') {
    if (baselines.length) bad(`${s.key}: a sorted slot flags no baseline`);
    if (s.variants.length < 2 || s.variants.length > 50) bad(`${s.key}: sorted slot needs 2 to 50 items`);
  } else if (baselines.length !== 1) bad(`${s.key}: needs exactly one baseline (has ${baselines.length})`);
  if (kind === 'copy' && (!s.copy || s.copy.fields.length < 1 || s.copy.fields.length > 8)) bad(`${s.key}: copy needs 1-8 fields`);
}

// ---- rendered markup ----
const pages = [
  ['home', '/#/', null], ['category', '/#/c/kitchen', null], ['search', '/#/search?q=speaker', null],
  ['product', '/#/product/p001', null], ['cart', '/#/cart', { p001: 1 }],
];
const found = new Map(); // key -> { pages:Set, count:max per page, items:Set, fields:Set }
const browser = await chromium.launch();
try {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();
  await page.route('**/_prb/**', (r) => r.fulfill({ status: 502, body: 'down' })); // judge the markup, never what a live Core swapped in
  for (const [name, path, cart] of pages) {
    await page.goto(BASE + '/');
    await page.evaluate((c) => localStorage.setItem('haat.cart', JSON.stringify(c || {})), cart);
    await page.goto(BASE + path);
    await page.waitForSelector('#view > *');
    const slots = await page.$$eval('[data-prb-slot]', (els) => els.map((el) => ({
      key: el.getAttribute('data-prb-slot'),
      items: [...el.children].filter((c) => c.hasAttribute('data-prb-item')).map((c) => c.getAttribute('data-prb-item')),
      fields: [...el.querySelectorAll('[data-prb-field]')].map((f) => f.getAttribute('data-prb-field')),
      hasText: !!el.textContent.trim() || el.children.length > 0,
    })));
    const perPage = new Map();
    for (const s of slots) {
      perPage.set(s.key, (perPage.get(s.key) || 0) + 1);
      const f = found.get(s.key) || { pages: new Set(), items: new Set(), fields: new Set(), empty: false };
      f.pages.add(name); s.items.forEach((i) => f.items.add(i)); s.fields.forEach((i) => f.fields.add(i)); f.empty ||= !s.hasText;
      found.set(s.key, f);
    }
    for (const [k, n] of perPage) if (n !== 1) bad(`${k}: ${n} elements on ${name} (selector must match exactly one)`);
    // the selector in site.json really resolves, on the pages that carry the slot
    for (const s of site.slots) {
      if (!slots.some((x) => x.key === s.key)) continue;
      const n = await page.locator(s.selector).count();
      if (n !== 1) bad(`${s.key}: selector ${s.selector} matches ${n} on ${name}`);
    }
  }
} finally { await browser.close(); }

for (const s of site.slots) {
  const f = found.get(s.key);
  if (!f) { bad(`${s.key}: in site.json but no element in the rendered markup`); continue; }
  const kind = s.kind || 'swap';
  if (f.empty && kind !== 'swap') bad(`${s.key}: baseline is empty in markup`);
  if (kind === 'sorted') {
    const declared = new Set(s.variants.map((v) => v.key));
    for (const i of f.items) if (!declared.has(i)) bad(`${s.key}: item ${i} is in the markup but not in site.json`);
    if (!f.items.size) bad(`${s.key}: no data-prb-item children in the markup`);
  }
  if (kind === 'copy') for (const fl of s.copy.fields) if (!f.fields.has(fl.name)) bad(`${s.key}: markup has no data-prb-field="${fl.name}"`);
}
for (const k of found.keys()) if (!keys.has(k)) bad(`${k}: marked in the markup but not in site.json`);

if (errs.length) { console.error('check:slots FAILED\n - ' + errs.join('\n - ')); process.exit(1); }
console.log(`check:slots ok: ${site.slots.length} slots in site.json, ${found.size} marked in markup, ${[...found.values()].reduce((a, f) => a + f.items.size, 0)} items`);
