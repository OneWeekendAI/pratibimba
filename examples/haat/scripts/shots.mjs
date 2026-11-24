// Screenshot pass. Expects the dev server already up on 5174 (npm run dev). Writes proof/design/*.png.
// Usage: node scripts/shots.mjs [only-substring]
import { chromium } from '@playwright/test';
import { mkdirSync } from 'node:fs';

const BASE = 'http://localhost:5174';
const OUT = new URL('../proof/design/', import.meta.url).pathname;
mkdirSync(OUT, { recursive: true });
const only = process.argv[2] || '';
const CATS = ['electronics', 'kitchen', 'books', 'fashion', 'home', 'toys', 'grocery', 'beauty'];
const pages = [
  ['home', '/#/'], ['electronics', '/#/c/electronics'], ['books', '/#/c/books'], ['product', '/#/product/p001'],
  ['cart', '/#/cart', true], ['checkout', '/#/checkout', true], ['search', '/#/search?q=speaker'],
  ...CATS.map((c) => [`sheet-${c}`, `/#/art-sheet?cat=${c}`]),
];
const sizes = [['desktop', { width: 1440, height: 900 }], ['phone', { width: 390, height: 844 }]];

const browser = await chromium.launch();
let bad = 0;
for (const [sname, viewport] of sizes) {
  const ctx = await browser.newContext({ viewport, deviceScaleFactor: 1 });
  const page = await ctx.newPage();
  page.on('pageerror', (e) => { console.log('pageerror', e.message); bad++; });
  for (const [name, path, withCart] of pages) {
    if (only && !name.includes(only)) continue;
    if (sname === 'phone' && name.startsWith('sheet-')) continue;
    await page.goto(BASE + '/');
    await page.evaluate((c) => localStorage.setItem('haat.cart', c ? JSON.stringify({ p001: 1, p002: 1 }) : '{}'), !!withCart);
    await page.goto(BASE + path);
    await page.reload();
    await page.waitForTimeout(350);
    await page.screenshot({ path: `${OUT}${name}-${sname}.png`, fullPage: true });
    if (sname === 'phone') {
      const [sw, iw] = await page.evaluate(() => [document.documentElement.scrollWidth, innerWidth]);
      if (sw > iw) { console.log(`OVERFLOW ${name}: scrollWidth ${sw} > ${iw}`); bad++; }
    }
  }
  await ctx.close();
}
await browser.close();
console.log(bad ? `done with ${bad} problem(s)` : 'done, no overflow');
process.exit(bad ? 1 : 0);
