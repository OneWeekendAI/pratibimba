// Records a calm walkthrough of the local stack (ci/local-stack.sh) in Chromium: the shop as three
// different visitors, then the dashboard (sites, a slot's workbench, a decision and its why,
// results, approving a draft, changing a traffic share), then the shop again showing the approved
// copy. Nothing hosted: Core on :8080, the shop on :3200.
//
//   ci/local-stack.sh --fresh            (in another terminal; wait for "local stack up")
//   node examples/shop/scripts/demo-record.mjs [--out ../pratibimba-demo] [--skip-seed]
//
// Seeding (skipped with --skip-seed): 90 s of cohort traffic through the shop (cohort.mjs, plan
// `demo-third` three times), each third moved back to its own hour so the worker closes their
// windows and the curves have points, plus two drafts for the approvals queue. The video is written
// to --out (default ../pratibimba-demo, outside git) as pratibimba-demo.webm.
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, readdirSync, renameSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';

const here = import.meta.dirname;
const root = path.resolve(here, '../../..');
const { chromium } = createRequire(path.join(root, 'dashboard/package.json'))('@playwright/test');
const arg = (n, d) => (process.argv.includes(`--${n}`) ? process.argv[process.argv.indexOf(`--${n}`) + 1] : d);
const OUT = path.resolve(arg('out', path.resolve(root, '../pratibimba-demo')));
const CORE = 'http://127.0.0.1:8080';
const SHOP = 'http://localhost:3200';
const TOKEN = process.env.PRB_ADMIN_TOKEN ?? 'local-admin-token';
const DB = 'postgres://postgres@localhost:55440/prb_local';
const H = { 'content-type': 'application/json', authorization: `Bearer ${TOKEN}` };
const psql = (sql) => execFileSync('psql', [DB, '-qAt', '-c', sql]).toString().trim();
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const api = async (p, body) => {
  const r = await fetch(CORE + p, body ? { method: 'POST', headers: H, body: JSON.stringify(body) } : { headers: H });
  if (!r.ok) throw new Error(`${p} ${r.status} ${await r.text()}`);
  return r.json();
};

async function seed() {
  // The come-back offer is for lapsed buyers only (audiences.mjs), as on the hosted shop.
  execFileSync('node', [path.join(here, 'audiences.mjs'), '--core', CORE], { env: { ...process.env, PRB_ADMIN_TOKEN: TOKEN }, stdio: 'inherit' });
  // Three 30 s thirds of cohort traffic, each moved back (with its events) to its own hour, four,
  // three and two hours ago: every window closes, and the lift curves get a point per hour. All
  // traffic in one hour would be one point, which a line cannot draw.
  for (const [i, hours] of [4, 3, 2].entries()) {
    console.log(`seed: cohort traffic, third ${i + 1} of 3`);
    const since = psql('SELECT now()::text');
    const c = spawnSync('node', [path.join(here, 'cohort.mjs'), '--base', SHOP, '--plan', 'demo-third', '--seed', String(7 + i), '--out', path.join(OUT, 'demo-cohort.jsonl')], {
      cwd: path.resolve(here, '..'), stdio: ['ignore', 'ignore', 'inherit'],
    });
    if (c.status) throw new Error('cohort failed');
    psql(`UPDATE decisions SET created_at = created_at - interval '${hours} hours' WHERE site_id = 'shop' AND created_at >= '${since}';
          UPDATE events SET received_at = received_at - interval '${hours} hours' WHERE site_id = 'shop' AND received_at >= '${since}';`);
  }
  for (let i = 0; i < 120; i++) {
    const open = Number(psql("SELECT count(*) FROM decisions WHERE site_id = 'shop' AND settled_at IS NULL AND created_at < now() - interval '1 hour'"));
    if (open === 0) break;
    await wait(1000);
  }
  // Two drafts to review: new hero copy (a copy slot: its fields as JSON) and a promo line.
  const copy = { headline: 'Fresh beans, roasted for your week', subhead: 'Order by Sunday, brewed by Thursday.', cta_label: 'Pick your roast' };
  await api('/api/sites/shop/slots/hero_copy/variants', { key: 'weekly-roast', html: JSON.stringify(copy), note: 'Shorter headline for mobile', author_name: 'Asha' }).catch((e) => console.log('draft:', e.message));
  await api('/api/sites/shop/slots/promo/variants', { key: 'grinder-free', html: '<p class="promo">Free hand grinder with any three bags.</p>', author_name: 'Asha' }).catch((e) => console.log('draft:', e.message));
}

// A caption strip drawn over the page, so the video says what each step shows.
async function caption(page, text) {
  await page.evaluate((t) => {
    let el = document.getElementById('__demo_caption');
    if (!el) {
      el = document.createElement('div');
      el.id = '__demo_caption';
      el.style.cssText = 'position:fixed;left:50%;bottom:24px;transform:translateX(-50%);z-index:2147483647;max-width:80%;' +
        'background:rgba(20,20,20,.86);color:#fff;font:500 18px/1.4 system-ui,sans-serif;padding:10px 18px;border-radius:10px;' +
        'pointer-events:none;transition:opacity .3s';
      document.body.appendChild(el);
    }
    el.textContent = t;
  }, text);
}
async function step(page, text, ms = 3500) {
  await caption(page, text);
  await wait(ms);
}

const MOBILE_UA = 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1';

async function visitor(browser, label, { ua, url = SHOP + '/', cookies = [] } = {}) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, recordVideo: { dir: path.join(OUT, 'raw'), size: { width: 1280, height: 800 } }, userAgent: ua });
  if (cookies.length) await ctx.addCookies(cookies);
  const page = await ctx.newPage();
  await page.goto(url);
  await step(page, label, 4500);
  const order = await page.locator('[data-prb-item]').evaluateAll((els) => els.map((e) => e.getAttribute('data-prb-item')));
  const headline = await page.locator('h1').first().innerText();
  await page.mouse.wheel(0, 380);
  await step(page, `Hero: “${headline}” · grid order: ${order.join(' → ')}`, 4500);
  await ctx.close();
  return { headline, order };
}

async function main() {
  mkdirSync(path.join(OUT, 'raw'), { recursive: true });
  for (const f of readdirSync(path.join(OUT, 'raw'))) if (f.endsWith('.webm')) renameSync(path.join(OUT, 'raw', f), path.join(OUT, 'raw', `old-${f}`));
  if (!process.argv.includes('--skip-seed')) await seed();
  const browser = await chromium.launch({ slowMo: 120 });
  const seen = [];

  // 1. The shop, three visitors. Each gets its own decision for the hero, the hero copy and the grid.
  seen.push(await visitor(browser, 'Visitor 1 · desktop, first visit'));
  seen.push(await visitor(browser, 'Visitor 2 · phone, arriving from an ad', { ua: MOBILE_UA, url: SHOP + '/?utm_source=ig&utm_campaign=filter' }));
  seen.push(await visitor(browser, 'Visitor 3 · desktop, evening in Bengaluru', { cookies: [{ name: '_prb_tz', value: 'Asia/Kolkata', url: SHOP }] }));

  // 2. The dashboard, one context so the whole tour is one video.
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, recordVideo: { dir: path.join(OUT, 'raw'), size: { width: 1280, height: 800 } } });
  const page = await ctx.newPage();
  await page.addInitScript(() => { try { localStorage.setItem('prb-approver', 'Ritwik'); } catch {} });
  await page.goto(CORE + '/app/login/');
  await step(page, 'The dashboard is served by Core itself, at /app', 2500);
  await page.getByLabel('Admin token').pressSequentially(TOKEN, { delay: 40 });
  await page.getByRole('button', { name: 'Sign in' }).click();
  await page.waitForURL(/\/app\/site\/\?id=shop/);
  await step(page, 'Site home: live decisions, conversions, lift against the holdout, and every slot', 5000);
  await page.mouse.wheel(0, 500);
  await step(page, 'Each slot: what it decides, and how it is doing', 3500);

  await page.goto(CORE + '/app/slot/?site=shop&slot=hero');
  await step(page, 'The hero slot: each variant with its share of traffic and its lift', 5000);
  await page.mouse.wheel(0, 700);
  await step(page, 'Every decision is recorded: who saw what, and whether they clicked', 4000);
  await page.locator('[data-testid=decisions-table] tbody tr').nth(2).click();
  await page.waitForURL(/\/app\/decision\//);
  await step(page, 'One decision, explained: the features the model saw and the score of each variant', 5500);
  const cf = page.getByRole('button', { name: 'What would the policy have shown them?' });
  if (await cf.count()) {
    await cf.click();
    await step(page, 'The counterfactual: what the policy would have picked', 3500);
  }
  await page.mouse.wheel(0, 600);
  await step(page, 'Scores and the event timeline for this visitor', 4000);

  await page.goto(CORE + '/app/results/?site=shop');
  await step(page, 'Results: conversion and revenue per visitor, treated against the holdout', 5500);
  await page.mouse.wheel(0, 600);
  await step(page, 'Slots ranked by what they contribute', 3500);

  await page.goto(CORE + '/app/slot/?site=shop&slot=grid');
  await step(page, 'A sorted slot (the product grid): the order is learned per visitor', 4000);
  await page.mouse.wheel(0, 600);
  await step(page, 'Per item: how often it was shown in the top two, its mean position, and clicks', 4500);

  // 3. The gate: drafts wait for a person.
  await page.goto(CORE + '/app/approvals/?site=shop');
  await step(page, 'Approvals: nothing reaches visitors until a person approves it', 4500);
  const card = page.getByTestId('card-weekly-roast');
  if (await card.count()) {
    await card.scrollIntoViewIfNeeded();
    await step(page, 'A copy draft: plain text fields, checked for length and markup', 4500);
    const name = page.getByLabel('Reviewer name');
    if (await name.count()) await name.fill('Ritwik');
    const thumb = card.getByRole('slider');
    await thumb.focus();
    for (let i = 0; i < 15; i++) await page.keyboard.press('ArrowRight');
    await step(page, 'Pick the share of traffic it may take (at most 50%)', 3000);
    await card.getByRole('button', { name: /^Approve/ }).click();
    await step(page, 'Approved: it goes live on the next decision', 3500);
  }

  // 4. A live variant's share, changed on its card and audited.
  await page.goto(CORE + '/app/slot/?site=shop&slot=hero_copy');
  await step(page, 'The hero copy slot, now with the approved copy live', 4000);
  const live = page.getByTestId('variant-weekly-roast');
  if (await live.count()) {
    await live.scrollIntoViewIfNeeded();
    await live.getByRole('button', { name: 'Change share' }).click();
    const s = live.getByRole('slider');
    await s.focus();
    await page.keyboard.press('Home');
    for (let i = 0; i < 49; i++) await page.keyboard.press('ArrowRight');
    await step(page, 'Change a live variant’s share of traffic', 3000);
    await live.getByRole('button', { name: /^Save/ }).click();
    await step(page, 'Saved, and recorded in the audit log with the value it replaced', 4000);
  }
  await page.goto(CORE + '/app/approvals/?site=shop');
  await page.getByText('Recent decisions').scrollIntoViewIfNeeded();
  await step(page, 'Every approval and share change is recorded, by whom', 4500);
  await page.goto(CORE + '/app/slot/?site=shop&slot=comeback');
  await step(page, 'An audience gate: the come-back offer is shown only to lapsed buyers', 4500);
  await page.goto(CORE + '/app/audit/');
  await step(page, 'The audit log: every tool call a coding agent makes, refused or not', 4000);
  await ctx.close();

  // 5. Back on the shop: new visitors can now get the approved copy.
  // The copy slot sits inside the baseline hero, so find a new visitor who gets both (off camera),
  // then show that visitor: a decision is sticky for a minute, so the reload shows the same page.
  let cookies = null;
  for (let i = 0; i < 60 && !cookies; i++) {
    const c = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const q = await c.newPage();
    await q.goto(SHOP + '/');
    if ((await q.content()).includes('Fresh beans, roasted for your week')) cookies = await c.cookies();
    await c.close();
  }
  const end = await browser.newContext({ viewport: { width: 1280, height: 800 }, recordVideo: { dir: path.join(OUT, 'raw'), size: { width: 1280, height: 800 } } });
  if (cookies) await end.addCookies(cookies);
  const p = await end.newPage();
  await p.goto(SHOP + '/');
  await step(p, cookies ? 'A new visitor on the shop, getting the copy approved a minute ago' : 'A new visitor on the shop', 5000);
  await end.close();
  await browser.close();

  // One file: the raw clips in the order they were made, joined with ffmpeg.
  const clips = readdirSync(path.join(OUT, 'raw')).filter((f) => f.endsWith('.webm') && !f.startsWith('old-'))
    .map((f) => path.join(OUT, 'raw', f));
  clips.sort((a, b) => statTime(a) - statTime(b));
  const list = path.join(OUT, 'raw', 'list.txt');
  execFileSync('sh', ['-c', `printf "file '%s'\\n" ${clips.map((c) => `'${c}'`).join(' ')} > '${list}'`]);
  const video = path.join(OUT, 'pratibimba-demo.mp4');
  execFileSync('ffmpeg', ['-y', '-loglevel', 'error', '-f', 'concat', '-safe', '0', '-i', list, '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-r', '25', video]);
  console.log(JSON.stringify({ video, visitors: seen }, null, 1));
}
import { statSync } from 'node:fs';
const statTime = (f) => statSync(f).birthtimeMs;

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
