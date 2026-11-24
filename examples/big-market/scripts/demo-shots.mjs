// Screenshots of the two demo pages (the dev server must be running).
import { chromium } from '@playwright/test';
const b = await chromium.launch();
for (const [n, w] of [['personas', 1440], ['proof', 1280]]) {
  const p = await b.newPage({ viewport: { width: w, height: 900 } });
  const errs = []; p.on('console', (m) => m.type() === 'error' && errs.push(m.text()));
  await p.goto(`http://localhost:5174/demo/${n}.html`); await p.waitForLoadState('networkidle');
  await p.screenshot({ path: `proof/demo-${n}.png`, fullPage: true });
  console.log(n, 'errors:', errs.length);
}
await b.close();
