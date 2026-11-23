// The dashboard, driven in Chromium against real Core and a seeded history. One test per
// behaviour; the gate tests run last because they change data.
import { test, expect, request as pwRequest, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const CORE = 'http://127.0.0.1:18083';
const TOKEN = 'dash-admin-token';
// The stack's database (stack.mjs): on PRB_E2E_ADMIN_DB's server when set.
const E2E_DB = (process.env.PRB_E2E_ADMIN_DB ?? 'postgres://postgres@localhost:55440/postgres').replace(/\/[^/]*$/, '/prb_dash_e2e');

async function api<T = any>(path: string): Promise<T> {
  const c = await pwRequest.newContext({ baseURL: CORE, extraHTTPHeaders: { authorization: `Bearer ${TOKEN}` } });
  const r = await (await c.get(`/api${path}`)).json();
  await c.dispose();
  return r;
}
async function apiPost(path: string, body: unknown) {
  const c = await pwRequest.newContext({ baseURL: CORE, extraHTTPHeaders: { authorization: `Bearer ${TOKEN}` } });
  const r = await c.post(path, { data: body });
  await c.dispose();
  return r;
}
async function signIn(page: Page) {
  await page.request.post(`${CORE}/api/session`, { data: { token: TOKEN } });
}
const pp = (x: number) => `${x >= 0 ? '+' : '−'}${Math.abs(x * 100).toFixed(1)} pp`;

test('login_refuses_a_wrong_token', async ({ page }) => {
  await page.goto('/app/login/');
  await page.getByLabel('Admin token').fill('not-the-token');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.locator('p[role=alert]')).toHaveText('That token was not accepted.');
  await expect(page).toHaveURL(/\/app\/login\//);
});

test('login_lands_on_the_site_home', async ({ page }) => {
  await page.goto('/app/login/');
  await page.getByLabel('Admin token').fill(TOKEN);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page).toHaveURL(/\/app\/site\/\?id=shop/);
  await expect(page.getByRole('heading', { name: 'shop' })).toBeVisible();
});

test('signed_out_pages_send_you_to_login', async ({ page }) => {
  await page.goto('/app/results/?site=shop');
  await expect(page).toHaveURL(/\/app\/login\/\?next=/);
});

// The Preview room's explorer: the page is cut into its slots; a click on one lists every
// design of it, each drawn by the page; a click on a design shows it in place, without a reload.
test('preview_opens_a_compartment_and_shows_a_design_in_place', async ({ page }) => {
  await signIn(page);
  const designs = async (slot: string) => (await api(`/sites/shop/slots/${slot}`)).variants.filter((v: { status: string }) => ['approved', 'draft', 'paused'].includes(v.status)).length;
  const [heroN, bannerN] = [await designs('hero'), await designs('banner')];
  await page.goto('/app/preview/?site=shop');
  const site = page.frameLocator('iframe[title="Your site, in preview"]');
  await expect(site.locator('#prb-zones [data-prb-zone=hero]')).toBeAttached();
  await expect(site.locator('#prb-zones [data-prb-zone=banner]')).toBeAttached();
  await site.locator('[data-prb-slot=banner]').click();
  await expect(page.getByTestId('slot-tab-banner')).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByTestId('design-count')).toHaveText(`${bannerN} designs`);
  await site.locator('[data-prb-slot=hero]').click();
  await expect(page.getByTestId('slot-tab-hero')).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByTestId('design-count')).toHaveText(`${heroN} designs`);
  const gift = page.getByTestId('variant-gift');
  // Its thumbnail is the page itself, showing that design.
  await expect(gift.locator('[data-ready]')).toBeVisible({ timeout: 15_000 });
  const thumb = gift.frameLocator('iframe');
  await expect(thumb.locator('[data-prb-slot=hero] h1')).toHaveText('Gift sets, wrapped and ready');
  // A mark on the frame's window: a reload would drop it.
  const frame = (await (await page.locator('iframe[title="Your site, in preview"]').elementHandle())!.contentFrame())!;
  await frame.evaluate(() => ((window as unknown as { __kept: number }).__kept = 1));
  await page.getByTestId('variant-baseline').click();
  await expect(site.locator('[data-prb-slot=hero] h1')).toHaveText('Kaapi Co.');
  await gift.click();
  await expect(site.locator('[data-prb-slot=hero] h1')).toHaveText('Gift sets, wrapped and ready');
  await expect(gift).toHaveAttribute('aria-selected', 'true');
  // Up and down move through the designs, and the page follows.
  await page.keyboard.press('ArrowUp');
  await expect(gift).toHaveAttribute('aria-selected', 'false');
  await expect(site.locator('[data-prb-slot=hero] h1')).not.toHaveText('Gift sets, wrapped and ready');
  await page.keyboard.press('ArrowDown');
  await expect(site.locator('[data-prb-slot=hero] h1')).toHaveText('Gift sets, wrapped and ready');
  expect(await frame.evaluate(() => (window as unknown as { __kept?: number }).__kept)).toBe(1);
});

test('preview_says_how_to_add_a_design_to_a_slot_with_only_its_own', async ({ page }) => {
  await signIn(page);
  // The banner as if it had only the page's own design.
  await page.route('**/api/sites/shop/slots/banner', async (r) => {
    const res = await r.fetch();
    const d = await res.json();
    await r.fulfill({ response: res, json: { ...d, variants: d.variants.filter((v: { is_baseline: boolean }) => v.is_baseline) } });
  });
  await page.goto('/app/preview/?site=shop');
  await page.getByTestId('slot-tab-banner').click();
  await expect(page.getByTestId('only-own').getByRole('link', { name: 'Add a design' })).toHaveAttribute('href', /\/drafts\/\?site=shop&slot=banner/);
});

test('site_home_tiles_match_the_api', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/site/?id=shop');
  const o = await api('/sites/shop/overview');
  await expect(page.getByTestId('tile-Conversions in the last hour')).toContainText(String(o.conversions_last_hour));
  await expect(page.getByTestId('tile-Lift today vs holdout')).toContainText(pp(o.lift_today.estimate));
  await expect(page.getByTestId('slot-row-hero')).toContainText('hero');
});

test('live_tile_updates_over_sse_without_reload', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/site/?id=shop');
  const tile = page.getByTestId('tile-Decisions in the last minute');
  await expect(page.getByText('live', { exact: true })).toBeVisible();
  const before = Number((await tile.locator('div').nth(1).innerText()).replace(/,/g, ''));
  for (let i = 0; i < 7; i++) {
    await (await pwRequest.newContext()).post(`${CORE}/decide`, { data: { site: 'shop', slot: 'hero', subject: `sse-${Date.now()}-${i}` } });
  }
  await expect.poll(async () => Number((await tile.locator('div').nth(1).innerText()).replace(/,/g, '')), { timeout: 10_000 }).toBeGreaterThanOrEqual(before + 7);
});

test('slot_row_opens_the_workbench', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/site/?id=shop');
  await page.getByTestId('slot-row-hero').click();
  await expect(page).toHaveURL(/\/app\/slot\/\?site=shop&slot=hero/);
  await expect(page.getByRole('heading', { name: 'hero' })).toBeVisible();
});

test('workbench_shows_each_variant_with_share_and_lift', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/slot/?site=shop&slot=hero');
  const s = await api('/sites/shop/slots/hero');
  for (const key of ['baseline', 'gift', 'free_ship']) await expect(page.getByTestId(`variant-${key}`)).toBeVisible();
  const gift = s.variants.find((v: any) => v.key === 'gift');
  await expect(page.getByTestId('variant-gift')).toContainText(pp(gift.lift.estimate));
  await expect(page.getByTestId('variant-gift')).toContainText(`${Math.round(gift.share * 100)}% of traffic`);
});

test('paused_variant_shows_its_death_certificate', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/slot/?site=shop&slot=hero');
  const card = page.getByTestId('variant-free_ship');
  await expect(card).toContainText('paused by rollback');
  await expect(card.getByTestId('death-certificate')).toContainText('Model restored to v');
});

test('decisions_filter_by_variant', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/slot/?site=shop&slot=hero');
  await page.getByLabel('Filter by variant').selectOption('gift');
  const cells = page.locator('[data-testid=decisions-table] tbody tr td:nth-child(3)');
  // The table keeps the previous rows on screen while the filtered query loads, so wait for
  // the filtered rows rather than reading whatever is drawn first.
  await expect.poll(async () => [...new Set(await cells.allInnerTexts())]).toEqual(['gift']);
  expect((await cells.allInnerTexts()).length).toBeGreaterThan(10);
});

test('decision_row_opens_the_why_sentence', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/slot/?site=shop&slot=hero');
  await page.locator('[data-testid=decisions-table] tbody tr').nth(3).click();
  await expect(page).toHaveURL(/\/app\/decision\/\?id=/);
  const id = new URL(page.url()).searchParams.get('id')!;
  const d = await api(`/decisions/${id}`);
  await expect(page.getByTestId('why')).toHaveText(d.sentence.replace(/`/g, ''));
  await expect(page.getByText('replay reproduces the scores')).toBeVisible();
});

test('counterfactual_reveals_the_other_variant', async ({ page }) => {
  await signIn(page);
  // A holdout visitor the policy would have shown something else: then the two sides differ.
  const rows = await api('/sites/shop/decisions?slot=hero&outcome=holdout&limit=40');
  let d: any = null;
  for (const r of rows) {
    const x = await api(`/decisions/${r.id}`);
    if (x.counterfactual && x.counterfactual.variant !== x.variant) {
      d = x;
      break;
    }
  }
  expect(d, 'no holdout decision whose policy pick differs from the baseline').not.toBeNull();
  await page.goto(`/app/decision/?id=${d.id}&site=shop`);
  await expect(page.getByTestId('counterfactual')).toHaveCount(0);
  await page.getByRole('button', { name: 'What would the policy have shown them?' }).click();
  await expect(page.getByTestId('counterfactual')).toContainText(`${d.counterfactual.variant} (the policy’s pick)`);
});

test('counterfactual_before_any_lesson_says_nothing_was_learned', async ({ page }) => {
  await signIn(page);
  // A decision scored by a model with no lessons (the seed's first half, so the oldest listed):
  // its inferred rates are the prior's 50%.
  const rows = await api('/sites/shop/decisions?slot=hero&limit=5000');
  let d: any = null;
  for (const r of rows.reverse().slice(0, 40)) {
    const x = await api(`/decisions/${r.id}`);
    if (x.learned === false && x.counterfactual) {
      d = x;
      break;
    }
  }
  expect(d, 'no decision scored before the model learned anything').not.toBeNull();
  await page.goto(`/app/decision/?id=${d.id}&site=shop`);
  await expect(page.getByTestId('scores-unlearned')).toContainText('Nothing learned yet');
  await page.getByRole('button', { name: /What would/ }).click();
  await expect(page.getByTestId('counterfactual')).toContainText('had learned nothing yet');
  await expect(page.getByTestId('counterfactual')).not.toContainText('%');
});

test('results_and_slot_say_how_many_decisions_are_still_in_their_window', async ({ page }) => {
  await signIn(page);
  // Fresh decisions: their windows are open, so they are pending, not counted.
  for (let i = 0; i < 3; i++) {
    await (await pwRequest.newContext()).post(`${CORE}/decide`, { data: { site: 'shop', slot: 'hero', subject: `pending-${Date.now()}-${i}` } });
  }
  const r = await api('/sites/shop/results');
  const n = r.results.slots.find((s: any) => s.key === 'hero').pending.decisions;
  expect(n).toBeGreaterThanOrEqual(3);
  await page.goto('/app/results/?site=shop');
  await expect(page.getByTestId('pending-hero')).toContainText(`${n.toLocaleString('en-US')} decisions still in their window`);
  await page.goto('/app/slot/?site=shop&slot=hero');
  await expect(page.getByTestId('slot-pending')).toContainText(`${n.toLocaleString('en-US')} decisions still in their window`);
});

test('results_headline_matches_the_api', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/results/?site=shop');
  const r = await api('/sites/shop/results');
  await expect(page.getByTestId('headline')).toContainText(pp(r.results.lift.estimate));
  await expect(page.getByTestId('headline')).toContainText(`${pp(r.results.lift.lo)} to ${pp(r.results.lift.hi)}`);
});

test('results_plain_sentences_use_the_api_numbers', async ({ page }) => {
  // "about N more/fewer per 1,000 visits" is the lift times 1,000, for the site and each slot.
  await signIn(page);
  await page.goto('/app/results/?site=shop');
  const r = await api('/sites/shop/results');
  const per = (x: number) => Math.round(x * 1000);
  const words = (x: number) => (per(x) === 0 ? 'about the same' : `about ${Math.abs(per(x)).toLocaleString('en-US')} ${per(x) > 0 ? 'more' : 'fewer'}`);
  await expect(page.getByTestId('headline-sentence')).toContainText(words(r.results.lift.estimate));
  await expect(page.getByTestId('headline')).toContainText(`Based on ${(r.results.treated.n + r.results.holdout.n).toLocaleString('en-US')} visits`);
  for (const s of r.results.slots.filter((s: any) => s.lift)) {
    await expect(page.getByTestId(`verdict-${s.key}`)).toContainText(words(s.lift.estimate));
  }
});

test('results_card_shows_revenue_per_visitor', async ({ page }) => {
  // The stack seeds two hero orders (USD 40 + 49.50); the line shows each arm's revenue per visitor.
  await signIn(page);
  await page.goto('/app/results/?site=shop');
  const r = await api('/sites/shop/results');
  const hero = r.results.slots.find((s: any) => s.key === 'hero');
  expect(hero.treated.revenue + hero.holdout.revenue).toBeCloseTo(89.5, 2);
  const usd = (x: number) => new Intl.NumberFormat(undefined, { style: 'currency', currency: 'USD', maximumFractionDigits: 2 }).format(x);
  const line = page.getByTestId('revenue-hero');
  await expect(line).toContainText(`Revenue per visitor ${usd(hero.treated.revenue_per_visitor.estimate)} vs ${usd(hero.holdout.revenue_per_visitor.estimate)} holdout`);
});

test('learnings_feed_lists_the_rollback', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/results/?site=shop');
  await expect(page.getByTestId('learning').filter({ hasText: 'rollback' })).toContainText('free_ship on hero was paused by auto-rollback');
});

test('install_steps_are_probes', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/install/?site=shop');
  await expect(page.getByTestId('step-reach')).toContainText('observed');
  await (await pwRequest.newContext()).get('http://127.0.0.1:3400/__mode?reach=off');
  try {
    await page.getByRole('button', { name: 'Check now' }).click();
    await expect(page.getByTestId('step-reach')).toContainText('waiting');
    await expect(page.getByTestId('detail-reach')).toContainText('route.ts');
    await expect(page.getByTestId('step-package')).toContainText('observed');
  } finally {
    await (await pwRequest.newContext()).get('http://127.0.0.1:3400/__mode?reach=on');
  }
});

test('dark_mode_toggle_persists', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/site/?id=shop');
  await page.getByRole('button', { name: /Dark mode/ }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
});

test('install_strip_shows_the_observed_sdk', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/site/?id=shop');
  const strip = page.getByTestId('install-strip');
  // The stand-in site answers with x-prb-sdk: next/0.1.0; the strip shows what the probe saw.
  await expect(strip).toContainText('SDK next/0.1.0');
  await strip.click();
  await expect(page).toHaveURL(/\/app\/install\/\?site=shop/);
});

test('traffic_bar_colours_match_the_workbench', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/site/?id=shop');
  // hero's variants are baseline, free_ship (paused), gift: gift is third of all, second of the live.
  const segs = page.getByTestId('slot-row-hero').locator('[role=img] > div');
  await expect(segs.first()).toBeVisible();
  const bar = await segs.evaluateAll((els) => els.map((e) => [e.getAttribute('title')!.split(':')[0], (e as HTMLElement).style.background]));
  expect(bar.map((b) => b[0])).toContain('gift');
  await page.goto('/app/slot/?site=shop&slot=hero');
  for (const [key, bg] of bar) {
    const stripe = await page.getByTestId(`variant-${key}`).getByTestId('series-dot').evaluate((e) => (e as HTMLElement).style.background);
    expect(stripe, key).toBe(bg);
  }
});

test('lift_curve_draws_a_line_per_variant', async ({ page }) => {
  await signIn(page);
  const s = await api('/sites/shop/slots/hero');
  const n = s.variants.filter((v: any) => s.curve.some((p: any) => p.variants[v.key])).length;
  expect(n).toBeGreaterThanOrEqual(2);
  await page.goto('/app/slot/?site=shop&slot=hero');
  await expect(page.locator('.recharts-line')).toHaveCount(n);
});

test('decisions_filter_by_outcome', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/slot/?site=shop&slot=hero');
  await page.getByLabel('Filter by outcome').selectOption('converted');
  const cells = page.locator('[data-testid=decisions-table] tbody tr td:nth-child(6)');
  await expect(cells.first()).toHaveText('converted');
  const texts = await cells.allInnerTexts();
  expect(texts.length).toBeGreaterThan(10);
  expect(new Set(texts)).toEqual(new Set(['converted']));
});

test('decision_detail_shows_features_scores_and_timeline', async ({ page }) => {
  await signIn(page);
  const [row] = await api('/sites/shop/decisions?slot=hero&outcome=converted&limit=1');
  const d = await api(`/decisions/${row.id}`);
  await page.goto(`/app/decision/?id=${d.id}&site=shop`);
  for (const [k, v] of Object.entries(d.features)) {
    await expect(page.locator('tr', { has: page.locator(`td:text-is("${k}")`) }).locator('td').last()).toHaveText(String(v));
  }
  const rows = page.getByRole('table', { name: 'Arm scores' }).getByRole('row');
  await expect(rows).toHaveCount(d.scores.length);
  for (const [i, s] of d.scores.entries()) await expect(rows.nth(i)).toContainText(`${s.variant}${s.score.toFixed(3)}`);
  const tl = page.getByTestId('timeline');
  // The decision, then every event in order, then the outcome.
  const names = d.timeline.map((e: any) => e.name);
  expect(names.indexOf('exposure')).toBeGreaterThanOrEqual(0);
  // One batch carries both, with one received_at: the exposure still reads first.
  expect(names.indexOf('exposure')).toBeLessThan(names.indexOf('click'));
  await expect(tl.locator('li > span.font-mono')).toHaveText([d.variant, ...d.timeline.map((e: any) => e.name)]);
  await expect(tl.locator('li').last()).toContainText('credited as a conversion (click)');
});

test('decision_detail_carries_the_reviewer_comment', async ({ page }) => {
  await signIn(page);
  const [won] = await api('/sites/shop/decisions?slot=banner&variant=ten_off&limit=1');
  await page.goto(`/app/decision/?id=${won.id}&site=shop`);
  await expect(page.getByTestId('review')).toContainText('Ritwik approved ten_off');
  await expect(page.getByTestId('review')).toContainText('Copy checked against the pricing page');
  // The baseline was never reviewed, so its decisions carry no gate section.
  const [plain] = await api('/sites/shop/decisions?slot=banner&variant=plain&limit=1');
  await page.goto(`/app/decision/?id=${plain.id}&site=shop`);
  await expect(page.getByTestId('why')).toBeVisible();
  await expect(page.getByTestId('review')).toHaveCount(0);
});

test('results_slots_are_ranked_by_contribution', async ({ page }) => {
  await signIn(page);
  const r = await api('/sites/shop/results');
  // The API lists slots by key (banner, hero); hero's lift on as many visitors must put it first.
  expect(r.results.slots.map((s: any) => s.key)).toEqual(['banner', 'hero']);
  const c = (s: any) => (s.lift?.estimate ?? -Infinity) * s.treated.n;
  expect(c(r.results.slots[1])).toBeGreaterThan(c(r.results.slots[0]));
  await page.goto('/app/results/?site=shop');
  const cards = page.locator('[data-testid^=result-]');
  await expect(cards).toHaveCount(2);
  expect(await cards.evaluateAll((es) => es.map((e) => e.getAttribute('data-testid')))).toEqual(['result-hero', 'result-banner']);
});

test('install_path_switches_the_snippet', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/install/?site=shop');
  const pkg = page.getByTestId('step-package');
  await expect(pkg).toContainText('npm install @pratibimba/next@0.1.0');
  await page.getByRole('radio', { name: /Another code-based framework/ }).check();
  await expect(pkg).toContainText('npm install @pratibimba/edge@0.1.0');
  await expect(pkg).not.toContainText('@pratibimba/next');
  await page.getByRole('radio', { name: /Script tag fallback/ }).check();
  await expect(pkg).toContainText('<script src="/_prb/sdk.js" data-site="shop">');
  await expect(pkg).toContainText('can flicker');
  await page.reload();
  await expect(page.getByRole('radio', { name: /Script tag fallback/ })).toBeChecked();
});

// ---- Phase 2: the agent affordances, and the Phase 1 leftovers ----

async function mcpCall(token: string, name: string, args: Record<string, unknown>) {
  const c = await pwRequest.newContext({ baseURL: CORE, extraHTTPHeaders: { authorization: `Bearer ${token}` } });
  const r = await c.post('/mcp', { data: { jsonrpc: '2.0', id: Date.now(), method: 'tools/call', params: { name, arguments: args } } });
  expect(r.ok()).toBe(true);
  await c.dispose();
}

test('agent_badge_names_the_agent_and_its_justification', async ({ page }) => {
  await signIn(page);
  const a = await api('/sites/shop/approvals');
  const draft = a.drafts.find((d: any) => d.key === 'gift_box');
  expect(draft.agent.name).toBe('Claude Code');
  await page.goto('/app/approvals/?site=shop');
  const badge = page.getByTestId('card-gift_box').getByTestId('agent-badge');
  await expect(badge).toContainText('Claude Code');
  await expect(badge).toContainText('for Ritwik');
  await expect(badge.getByTestId('agent-justification')).toHaveText(`“${draft.agent.justification}”`);
  // A person's draft carries no badge.
  await expect(page.getByTestId('card-gift_ribbon').getByTestId('agent-badge')).toHaveCount(0);
});

test('agent_badge_links_to_the_audit_record', async ({ page }) => {
  await signIn(page);
  const a = await api('/sites/shop/approvals');
  const id = a.drafts.find((d: any) => d.key === 'gift_box').agent.audit_id;
  await page.goto('/app/approvals/?site=shop');
  await page.getByTestId('card-gift_box').getByTestId('audit-link').click();
  await expect(page).toHaveURL(new RegExp(`/app/audit/\\?id=${id}$`));
  const rec = await api(`/mcp-calls/${id}`);
  await expect(page.getByRole('heading', { name: `Tool call #${id}` })).toBeVisible();
  const record = page.getByTestId('audit-record');
  await expect(record).toContainText('propose_variant');
  await expect(record).toContainText('on behalf of Ritwik');
  await expect(record).toContainText('ritwik-laptop');
  await expect(page.getByTestId('audit-justification')).toHaveText(`“${rec.justification}”`);
  await expect(page.getByTestId('audit-arguments')).toContainText('"key": "gift_box"');
  await expect(page.getByText('What it wrote')).toBeVisible();
});

test('component_draft_gets_a_code_diff', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/approvals/?site=shop');
  const code = page.getByTestId('card-gift_box').getByTestId('code-diff');
  await expect(code).toBeVisible();
  const added = await code.locator('tr[data-kind=add]').allInnerTexts();
  const removed = await code.locator('tr[data-kind=del]').allInnerTexts();
  // Only the call to action changed; the wrapper is new; headline and copy are unchanged lines.
  expect(added.join('\n')).toContain('Build a gift box');
  expect(removed.join('\n')).toContain('Shop gifts');
  expect(removed).toHaveLength(1);
  expect((await code.locator('tr[data-kind=same]').allInnerTexts()).join('\n')).toContain('Gift sets, wrapped and ready');
  expect(added.join('\n')).not.toContain('Gift sets');
});

test('inline_draft_diffs_by_word_and_can_switch_to_code', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/approvals/?site=shop');
  const card = page.getByTestId('card-gift_ribbon');
  await expect(card.getByTestId('diff')).toBeVisible();
  await expect(card.getByTestId('code-diff')).toHaveCount(0);
  await card.getByRole('tab', { name: 'Code' }).click();
  const code = card.getByTestId('code-diff');
  await expect(code.locator('tr[data-kind=same]')).toContainText(['Three roasts, a tasting card']);
  await expect(code.locator('tr[data-kind=add]').first()).toContainText('kraft');
});

test('connect_card_shows_endpoint_mint_command_and_pack', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/agent/?site=shop');
  const card = page.getByTestId('connect-agent');
  await expect(card.getByTestId('copy-endpoint')).toHaveText(`${CORE}/mcp`);
  await expect(card.getByTestId('copy-mint')).toHaveText('pratibimba mcp-token shop <name>');
  await expect(card.getByTestId('copy-pack')).toContainText(`npx @pratibimba/agent install --url ${CORE}`);
  await expect(card.getByTestId('copy-pack')).toContainText('PRATIBIMBA_MCP_TOKEN=');
  await expect(card.getByTestId('copy-config')).toContainText(`claude mcp add --transport http pratibimba ${CORE}/mcp`);
  await card.getByRole('tab', { name: 'Cursor' }).click();
  await expect(card.getByTestId('copy-config')).toContainText('"mcpServers"');
  await expect(card.getByTestId('agent-status')).toContainText('token last used');
  await expect(card.getByTestId('agent-status')).toContainText('agent connected');
});

test('connect_card_mints_a_token_that_works', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/agent/?site=shop');
  const card = page.getByTestId('connect-agent');
  await card.getByLabel('Token name').fill('e2e-cursor');
  await card.getByRole('button', { name: 'Or mint one here' }).click();
  await expect(card.getByTestId('minted')).toBeVisible();
  const token = /Bearer (prb_mcp_\S+?)"/.exec(await card.getByTestId('copy-config').innerText())?.[1];
  expect(token).toBeTruthy();
  await expect(card.getByTestId('copy-pack')).toContainText(`PRATIBIMBA_MCP_TOKEN=${token}`);
  await mcpCall(token!, 'list_slots', { agent: 'Cursor', justification: 'check the minted token' });
  const t = await api('/sites/shop/mcp-calls');
  expect(t.calls[0]).toMatchObject({ token: 'e2e-cursor', agent: 'Cursor', outcome: 'ok' });
});

test('mcp_tail_shows_new_calls_without_reload', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/agent/?site=shop');
  const tail = page.getByTestId('mcp-tail');
  const t = await api('/sites/shop/mcp-calls');
  await expect(tail.getByTestId('mcp-call').first()).toContainText(t.calls[0].tool);
  await expect(tail).toContainText('refused');
  const c = await pwRequest.newContext({ baseURL: CORE, extraHTTPHeaders: { authorization: `Bearer ${TOKEN}` } });
  const { token } = await (await c.post('/api/sites/shop/mcp-tokens', { data: { name: 'e2e-tail' } })).json();
  await c.dispose();
  const why = `tail check ${Date.now()}`;
  await mcpCall(token, 'get_site', { agent: 'Tail Agent', justification: why });
  await expect(tail.getByTestId('mcp-call').first()).toContainText('Tail Agent', { timeout: 8000 });
  await expect(tail.getByTestId('mcp-call').first()).toContainText(why);
  await tail.getByTestId('mcp-call').first().click();
  await expect(page.getByTestId('audit-justification')).toHaveText(`“${why}”`);
});

test('install_agent_step_is_observed', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/install/?site=shop');
  await expect(page.getByTestId('step-agent')).toContainText('observed');
  await expect(page.getByTestId('detail-agent')).toContainText('called');
  await expect(page.getByTestId('step-agent').getByTestId('connect-agent')).toBeVisible();
});

test('kill_switch_pauses_every_slot_and_resumes', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/results/?site=shop');
  const banner = page.getByTestId('live-banner');
  await expect(banner).toHaveAttribute('data-state', 'live');
  await expect(banner).toContainText('Live');
  try {
    await page.getByLabel('Your name', { exact: true }).fill('Ritwik'); // the one name field, in the header
    await banner.getByRole('button', { name: 'Pause all slots' }).click();
    await expect(banner).toHaveAttribute('data-state', 'paused');
    await expect(banner).toContainText('Paused by Ritwik');
    expect((await api('/sites/shop/overview')).paused.by).toBe('Ritwik');
    const c = await pwRequest.newContext({ baseURL: CORE });
    const seen = new Set<string>();
    for (let i = 0; i < 40; i++) {
      const r = await (await c.post('/decide/batch', { data: { site: 'shop', subject: `kill-${Date.now()}-${i}`, slots: ['hero', 'banner'] } })).json();
      for (const d of r.decisions) seen.add(`${d.slot}:${d.variant}`);
    }
    await c.dispose();
    expect([...seen].sort()).toEqual(['banner:plain', 'hero:baseline']);
    await banner.getByRole('button', { name: 'Resume all slots' }).click();
    await expect(banner).toHaveAttribute('data-state', 'live');
    expect((await api('/sites/shop/overview')).paused).toBeNull();
  } finally {
    await apiPost('/api/sites/shop/pause', { actor: 'e2e', paused: false });
  }
});

test('no_warning_banner_without_a_deanonymiser', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/results/?site=shop');
  await expect(page.getByTestId('live-banner')).toBeVisible();
  await expect(page.getByTestId('warning-banner')).toHaveCount(0);
});

test('installing_a_deanonymiser_shows_a_warning_banner', async ({ page }) => {
  await signIn(page);
  const r = await apiPost('/api/plugins/sink.identity_resolver/install', { site: 'shop', actor: 'e2e', config: { url: 'https://resolver.example.com/in' } });
  expect(r.status()).toBe(201);
  try {
    await page.goto('/app/results/?site=shop');
    const banner = page.getByTestId('warning-banner');
    await expect(banner).toContainText('sink.identity_resolver is a de-anonymiser');
    await expect(banner).toContainText('https://resolver.example.com/in');
  } finally {
    await apiPost('/api/plugins/sink.identity_resolver/uninstall', { site: 'shop', actor: 'e2e' });
  }
});

test('lift_axis_ticks_sit_on_round_values', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/slot/?site=shop&slot=hero');
  await expect(page.locator('.recharts-line').first()).toBeVisible();
  // Recharts 3 draws tick labels in a layer outside the axis group; the y labels are the signed
  // integers (the x labels are dates).
  await expect(page.locator('.recharts-cartesian-axis-tick-value').first()).toBeVisible();
  const labels = (await page.locator('.recharts-cartesian-axis-tick-value').allTextContents()).filter((l) => /^[+−]?\d+(\.\d+)?$/.test(l));
  const ticks = labels.map((l) => Number(l.replace('−', '-').replace('+', '')));
  expect(ticks.length).toBeGreaterThanOrEqual(3);
  expect(ticks).toContain(0);
  const step = ticks[1] - ticks[0];
  const mag = 10 ** Math.floor(Math.log10(step));
  expect([1, 2, 5]).toContain(Math.round(step / mag));
  for (let i = 1; i < ticks.length; i++) expect(ticks[i] - ticks[i - 1]).toBeCloseTo(step, 6);
  for (const t of ticks) expect(Math.abs(t / step - Math.round(t / step))).toBeLessThan(1e-6);
});

test('header_stays_in_view_on_a_long_page', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/slot/?site=shop&slot=hero');
  await expect(page.getByTestId('decisions-table')).toBeVisible();
  const total = await page.evaluate(() => document.documentElement.scrollHeight);
  expect(total).toBeGreaterThan(1200);
  await page.evaluate(() => scrollTo(0, document.body.scrollHeight));
  await expect(page.getByTestId('sidebar')).toBeInViewport();
  await expect(page.getByRole('button', { name: 'Sign out' })).toBeInViewport();
});

test('approval_card_diffs_against_the_champion', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/approvals/?site=shop');
  const card = page.getByTestId('card-gift_ribbon');
  await expect(card).toContainText('Champion: gift');
  const diff = card.getByTestId('diff');
  await expect(diff).toBeVisible();
  const ins = (await diff.locator('ins').allInnerTexts()).join(' ');
  const del = (await diff.locator('del').allInnerTexts()).join(' ');
  expect(ins).toContain('kraft');
  expect(ins).toContain('Choose');
  expect(del).toContain('ready');
  expect(ins + del).not.toContain('Three roasts');
  await expect(diff).toContainText('Three roasts, a tasting card');
});

test('no_bulk_approve_control', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/approvals/?site=shop');
  const cards = page.locator('[data-testid^=card-]');
  await expect(cards.first()).toBeVisible();
  // One approve button per card, and nothing that selects or approves several.
  expect(await page.getByRole('button', { name: /^Approve/ }).count()).toBe(await cards.count());
  expect(await page.getByRole('checkbox').count()).toBe(0);
  // Scoped to the page body: the site banner's "Pause all slots" kill switch is not an approval.
  expect(await page.getByRole('main').getByRole('button', { name: /all|selected|bulk/i }).count()).toBe(0);
});

test('share_slider_stops_at_fifty', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/approvals/?site=shop');
  const card = page.getByTestId('card-gift_ribbon');
  const thumb = card.getByRole('slider');
  await thumb.focus();
  await page.keyboard.press('End');
  await page.keyboard.press('ArrowRight');
  await expect(card.getByTestId('share')).toHaveText('50%');
  await expect(card.getByRole('button', { name: /^Approve/ })).toHaveText('Approve gift_ribbon at 50%');
});

test('failing_check_disables_approve', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/approvals/?site=shop');
  await page.getByLabel('Your name', { exact: true }).fill('Ritwik');
  const card = page.getByTestId('card-popup_offer');
  await expect(card.getByTestId('checks')).toContainText('technical · fail');
  await expect(card.getByRole('button', { name: /^Approve/ })).toBeDisabled();
});

test('approve_takes_one_card_and_leaves_the_other', async ({ page }) => {
  await apiPost('/api/sites/shop/slots/banner/variants', { key: 'spring_beans', html: '<p>Spring harvest is in.</p>' });
  await signIn(page);
  await page.goto('/app/approvals/?site=shop');
  await page.getByLabel('Your name', { exact: true }).fill('Ritwik');
  const card = page.getByTestId('card-gift_ribbon');
  await card.getByRole('slider').focus();
  for (let i = 0; i < 5; i++) await page.keyboard.press('ArrowRight');
  await card.getByRole('button', { name: 'Approve gift_ribbon at 15%' }).click();
  await expect(page.getByTestId('card-gift_ribbon')).toHaveCount(0);
  await expect(page.getByTestId('card-spring_beans')).toBeVisible();
  const s = await api('/sites/shop/slots/hero');
  const v = s.variants.find((x: any) => x.key === 'gift_ribbon');
  expect([v.status, v.approved_by, Math.round(v.traffic_share * 100)]).toEqual(['approved', 'Ritwik', 15]);
  const banner = await api('/sites/shop/slots/banner');
  expect(banner.variants.find((x: any) => x.key === 'spring_beans').status).toBe('draft');
});

test('reject_needs_a_comment', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/approvals/?site=shop');
  await page.getByLabel('Your name', { exact: true }).fill('Ritwik');
  const card = page.getByTestId('card-popup_offer');
  await expect(card.getByRole('button', { name: 'Reject' })).toBeDisabled();
  await card.getByLabel('Comment (stays on the variant)').fill('Popups are off brand');
  await card.getByRole('button', { name: 'Reject' }).click();
  await expect(page.getByTestId('card-popup_offer')).toHaveCount(0);
  const s = await api('/sites/shop/slots/hero');
  expect(s.variants.find((x: any) => x.key === 'popup_offer').status).toBe('rejected');
});

test('draft_written_here_waits_for_review', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/drafts/?site=shop&slot=hero');
  await page.getByLabel('Key').fill('autumn_hero');
  await page.getByLabel('HTML').fill('<h1>Autumn roasts are here</h1>');
  await page.getByRole('button', { name: 'Save as draft' }).click();
  await page.getByTestId('draft-saved').click();
  await expect(page).toHaveURL(/\/app\/approvals\/\?site=shop/);
  await expect(page.getByTestId('card-autumn_hero')).toBeVisible();
  const s = await api('/sites/shop/slots/hero');
  expect(s.variants.find((x: any) => x.key === 'autumn_hero').status).toBe('draft');
});

// ---- webhooks ----

async function subscribe(url: string, events: string[]): Promise<number> {
  const c = await pwRequest.newContext({ baseURL: CORE, extraHTTPHeaders: { authorization: `Bearer ${TOKEN}` } });
  const r = await (await c.post('/api/sites/shop/webhooks', { data: { url, events } })).json();
  await c.dispose();
  return r.id;
}

test('webhook_secret_is_shown_once', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/webhooks/?site=shop');
  await page.getByLabel('Endpoint URL').fill('https://example.com/once');
  await page.getByLabel('decision').check();
  await page.getByRole('button', { name: 'Add endpoint' }).click();
  await expect(page.getByTestId('webhook-secret-value')).toHaveText(/^whsec_[0-9a-f]{64}$/);
  const secret = await page.getByTestId('webhook-secret-value').textContent();
  await page.reload();
  await expect(page.getByText('https://example.com/once')).toBeVisible();
  await expect(page.getByTestId('webhook-secret')).toHaveCount(0);
  expect(await page.content()).not.toContain(secret!);
});

test('webhook_add_needs_an_event', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/webhooks/?site=shop');
  await page.getByLabel('Endpoint URL').fill('https://example.com/none');
  await expect(page.getByRole('button', { name: 'Add endpoint' })).toBeDisabled();
});

test('webhook_delivery_record_shows_a_real_delivery', async ({ page }) => {
  await signIn(page);
  // The stand-in owner site answers 200 to any path, so it serves as the receiver here.
  const id = await subscribe('http://127.0.0.1:3400/hook', ['decision']);
  const c = await pwRequest.newContext({ baseURL: CORE });
  await c.post('/decide', { data: { site: 'shop', slot: 'hero', subject: `wh-${Date.now()}`, ctx: { returning: false } } });
  await c.dispose();
  await page.goto('/app/webhooks/?site=shop');
  const card = page.getByTestId(`webhook-${id}`);
  await expect(card.getByTestId('webhook-counts')).toContainText('1 delivered', { timeout: 10_000 });
  await card.getByRole('button', { name: 'Deliveries' }).click();
  const row = card.getByTestId('webhook-deliveries').locator('tbody tr').first();
  await expect(row).toContainText('decision');
  await expect(row).toContainText('delivered');
  await expect(row).toContainText('200');
});

test('webhook_delete_asks_to_confirm', async ({ page }) => {
  await signIn(page);
  const id = await subscribe('https://example.com/gone', ['conversion']);
  await page.goto('/app/webhooks/?site=shop');
  const card = page.getByTestId(`webhook-${id}`);
  await card.getByRole('button', { name: 'Delete' }).click();
  await expect(card).toBeVisible();
  await card.getByRole('button', { name: 'Confirm delete' }).click();
  await expect(card).toHaveCount(0);
  expect((await api('/sites/shop/webhooks')).subscriptions.some((s: any) => s.id === id)).toBe(false);
});

test('history_audience_is_written_and_gates_a_variant', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/slot/?site=shop&slot=hero');
  // A fresh N each run, so a stack whose database outlives a run still gets a new audience.
  const n = 31 + (Date.now() % 300);
  const form = page.getByRole('form', { name: 'New audience' });
  await form.getByLabel('Rule').selectOption('lapsed');
  await form.getByLabel('N').fill(String(n));
  await form.getByRole('button', { name: 'Add audience' }).click();
  const pick = page.getByLabel('Audience for gift', { exact: true });
  const option = pick.locator('option', { hasText: `last order days ago ${n}+` });
  await expect(option).toHaveCount(1);
  await expect(page.getByText('match identified visitors only')).toBeVisible();
  await pick.selectOption({ label: (await option.textContent())! });
  await expect.poll(async () => (await api('/sites/shop/slots/hero')).variants.find((v: any) => v.key === 'gift').audience).toEqual(expect.any(Number));
  const s = await api('/sites/shop/slots/hero');
  const aud = s.audiences.find((a: any) => a.id === s.variants.find((v: any) => v.key === 'gift').audience);
  expect(aud.predicate).toEqual({ days_since_last_purchase: { gte: n } });
  await page.reload();
  await expect(page.getByLabel('Audience for gift', { exact: true })).toHaveValue(String(aud.id));
  await page.getByLabel('Audience for gift', { exact: true }).selectOption('');
  await expect.poll(async () => (await api('/sites/shop/slots/hero')).variants.find((v: any) => v.key === 'gift').audience).toBeNull();
});

test('audience_select_is_as_wide_as_its_longest_audience', async ({ page }) => {
  await signIn(page);
  // A long audience name: the select must show it whole and stay inside the sidebar card.
  const name = `Lapsed buyers, autumn ${Date.now() % 1000}`;
  const headers = { authorization: `Bearer ${TOKEN}` };
  const seg = await page.request.post(`${CORE}/api/sites/shop/segments`, { headers, data: { predicate: { purchase_count: { gte: 2 + (Date.now() % 500) } }, name, actor: 'e2e' } });
  expect(seg.status()).toBe(201);
  expect((await page.request.post(`${CORE}/api/segments/${(await seg.json()).id}/promote`, { headers, data: { actor: 'e2e' } })).status()).toBe(200);
  await page.goto('/app/slot/?site=shop&slot=hero');
  const pick = page.getByLabel('Audience for gift', { exact: true });
  await expect(pick.locator('option', { hasText: name })).toHaveCount(1);
  const fit = await pick.evaluate((el: HTMLSelectElement) => {
    const ctx = document.createElement('canvas').getContext('2d')!;
    const cs = getComputedStyle(el);
    ctx.font = `${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
    const widest = Math.max(...[...el.options].map((o) => ctx.measureText(o.text).width));
    const box = el.getBoundingClientRect();
    const card = el.closest('[aria-label="Variant audiences"]')!.getBoundingClientRect();
    return { widest, width: box.width, right: box.right, cardRight: card.right };
  });
  expect(fit.width).toBeGreaterThanOrEqual(fit.widest + 20);
  expect(fit.right).toBeLessThanOrEqual(fit.cardRight + 0.5);
});

test('audit_without_an_id_lists_the_tool_calls', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/audit/');
  await expect(page.getByRole('heading', { name: 'Audit log' })).toBeVisible();
  await expect(page.getByText('Tool call #')).toHaveCount(0);
  const rows = page.getByTestId('mcp-call');
  await expect(rows.first()).toBeVisible();
  const tail = await api('/sites/shop/mcp-calls?limit=25');
  await expect(rows).toHaveCount(tail.calls.length);
  await rows.first().click();
  await expect(page).toHaveURL(new RegExp(`/app/audit/\\?id=${tail.calls[0].id}`));
  await expect(page.getByTestId('audit-record')).toBeVisible();
});

// G2: a live variant's share is editable after approval; these change data, so they run last.
const giftShare = async () => (await api('/sites/shop/slots/hero')).variants.find((v: any) => v.key === 'gift').traffic_share;

test('a_live_variants_share_is_changed_on_its_card', async ({ page }) => {
  await signIn(page);
  await page.addInitScript(() => localStorage.setItem('prb-approver', 'Ritwik'));
  await page.goto('/app/slot/?site=shop&slot=hero');
  const card = page.getByTestId('variant-gift');
  await card.getByRole('button', { name: 'Change share' }).click();
  const thumb = card.getByRole('slider');
  await thumb.focus();
  await page.keyboard.press('Home');
  for (let i = 0; i < 19; i++) await page.keyboard.press('ArrowRight');
  await card.getByRole('button', { name: 'Save 20%' }).click();
  await expect.poll(async () => Math.round((await giftShare()) * 100)).toBe(20);
  await expect(card.getByTestId('planned-share')).toHaveText('planned share 20% · edited by Ritwik');
  const a = await api('/sites/shop/approvals');
  expect(a.recent[0]).toMatchObject({ action: 'share', actor: 'Ritwik', variant: 'gift' });
});

test('an_agents_share_proposal_is_accepted_in_approvals', async ({ page }) => {
  await signIn(page);
  await page.addInitScript(() => localStorage.setItem('prb-approver', 'Ritwik'));
  const c = await pwRequest.newContext({ baseURL: CORE, extraHTTPHeaders: { authorization: `Bearer ${TOKEN}` } });
  const { token } = await (await c.post('/api/sites/shop/mcp-tokens', { data: { name: 'e2e-share' } })).json();
  await c.dispose();
  await mcpCall(token, 'update_slot', { agent: 'Claude Code', justification: 'gift keeps winning', slot: 'hero', traffic_share: { variant: 'gift', share: 0.35 } });
  expect(Math.round((await giftShare()) * 100)).not.toBe(35);
  await page.goto('/app/approvals/?site=shop');
  const row = page.getByTestId('share-request-gift');
  await expect(row).toContainText('35%');
  await expect(row).toContainText('gift keeps winning');
  await row.getByRole('button', { name: 'Accept' }).click();
  await expect.poll(async () => Math.round((await giftShare()) * 100)).toBe(35);
  await expect(row).toHaveCount(0);
});

async function agentToken(name: string): Promise<string> {
  const c = await pwRequest.newContext({ baseURL: CORE, extraHTTPHeaders: { authorization: `Bearer ${TOKEN}` } });
  const { token } = await (await c.post('/api/sites/shop/mcp-tokens', { data: { name } })).json();
  await c.dispose();
  return token;
}
const heroVariant = async (key: string) => (await api('/sites/shop/slots/hero')).variants.find((v: { key: string }) => v.key === key);

test('an_agents_retire_proposal_can_be_kept_with_a_reason', async ({ page }) => {
  await signIn(page);
  await page.addInitScript(() => localStorage.setItem('prb-approver', 'Ritwik'));
  const token = await agentToken('e2e-retire-keep');
  await mcpCall(token, 'retire_variant', { agent: 'Claude Code', justification: 'free_ship has not moved in weeks', slot: 'hero', variant: 'free_ship' });
  await page.goto('/app/approvals/?site=shop');
  const row = page.getByTestId('retire-request-free_ship');
  await expect(row).toContainText('free_ship has not moved in weeks');
  await expect(row.getByRole('button', { name: 'Keep' })).toBeDisabled();
  await row.getByLabel('Why not').fill('seasonal, back in December');
  await row.getByRole('button', { name: 'Keep' }).click();
  await expect(row).toHaveCount(0);
  expect((await heroVariant('free_ship')).status).not.toBe('retired');
});

test('an_agents_retire_proposal_is_confirmed_in_approvals', async ({ page }) => {
  await signIn(page);
  await page.addInitScript(() => localStorage.setItem('prb-approver', 'Ritwik'));
  const token = await agentToken('e2e-retire-yes');
  await mcpCall(token, 'retire_variant', { agent: 'Claude Code', justification: 'free_ship is dead weight', slot: 'hero', variant: 'free_ship' });
  expect((await heroVariant('free_ship')).status).not.toBe('retired');
  await page.goto('/app/approvals/?site=shop');
  const row = page.getByTestId('retire-request-free_ship');
  await row.getByRole('button', { name: 'Retire' }).click();
  await expect(row).toHaveCount(0);
  await expect.poll(async () => (await heroVariant('free_ship')).status).toBe('retired');
});

test('an_agents_plugin_request_is_listed_and_installed_or_declined', async ({ page }) => {
  await signIn(page);
  await page.addInitScript(() => localStorage.setItem('prb-approver', 'Ritwik'));
  const token = await agentToken('e2e-plugins');
  const ask = (plugin: string) => mcpCall(token, 'install_plugin', { agent: 'Claude Code', justification: `wire up ${plugin}`, plugin });
  await ask('attribution.linear');
  await page.goto('/app/approvals/?site=shop');
  const row = page.getByTestId('plugin-request-attribution.linear');
  await expect(row).toContainText('wire up attribution.linear');
  await expect(row.getByRole('button', { name: 'Decline' })).toBeDisabled();
  await row.getByLabel('Why not').fill('not this quarter');
  await row.getByRole('button', { name: 'Decline' }).click();
  await expect(row).toHaveCount(0);
  const linear = async () => (await api('/sites/shop/plugins')).plugins.find((p: { name: string }) => p.name === 'attribution.linear');
  expect((await linear()).installed).toBeNull();
  expect((await linear()).request).toMatchObject({ status: 'declined', comment: 'not this quarter' });
  // Ask again and say yes.
  await ask('attribution.linear');
  await page.reload();
  await page.getByTestId('plugin-request-attribution.linear').getByRole('button', { name: 'Install' }).click();
  await expect(page.getByTestId('plugin-request-attribution.linear')).toHaveCount(0);
  expect((await linear()).installed).toMatchObject({ by: 'Ritwik' });
  await apiPost('/api/plugins/attribution.linear/uninstall', { site: 'shop', actor: 'e2e' });
});

// S9: a share the site file set says so, and the editor states the re-apply rule. Re-applies
// shop's file (which approves every listed variant again), so it runs after the shop tests.
test('a_share_from_the_site_file_says_so_on_its_card', async ({ page }) => {
  const file = JSON.parse(readFileSync(path.join(import.meta.dirname, 'site.json'), 'utf8'));
  file.slots[0].variants.find((v: any) => v.key === 'gift').traffic_share = 0.4;
  const tmp = path.join(mkdtempSync(path.join(tmpdir(), 'prb-s9-')), 'site.json');
  writeFileSync(tmp, JSON.stringify(file));
  execFileSync(path.resolve(import.meta.dirname, '../../target/debug/pratibimba'), ['apply', tmp], {
    env: { ...process.env, PRB_DB: E2E_DB, RUST_LOG: 'warn' }, stdio: 'pipe',
  });
  await signIn(page);
  await page.goto('/app/slot/?site=shop&slot=hero');
  const card = page.getByTestId('variant-gift');
  await expect(card.getByTestId('planned-share')).toHaveText('planned share 40% · site file');
  await card.getByRole('button', { name: 'Change share' }).click();
  await expect(card.getByTestId('share-rule')).toHaveText("Re-applying the site file keeps your value until the file's own value changes.");
});

test('sorted_slot_shows_a_row_per_item', async ({ page }) => {
  // Last, because it adds a second site (see stack.mjs).
  test.setTimeout(90_000);
  expect((await fetch('http://127.0.0.1:3400/__seed-sorted')).status).toBe(204);
  await signIn(page);
  await page.goto('/app/slot/?site=rack&slot=grid');
  const s = await api('/sites/rack/slots/grid');
  expect(s.items.map((i: any) => i.key)).toEqual(['kaapi', 'filter', 'decoction', 'gift']);
  const rows = page.locator('[data-testid=items-table] tbody tr');
  await expect(rows).toHaveCount(4);
  for (const [n, i] of s.items.entries()) {
    const rate = i.rate === null ? '—' : `${(i.rate * 100).toFixed(0)}%`;
    const pos = i.mean_position === null ? '—' : i.mean_position.toFixed(1);
    await expect(rows.nth(n).locator('td')).toHaveText([String(n + 1), i.key, String(i.shown), pos, String(i.acted), rate]);
  }
});

test('sorted_slot_items_are_a_numbered_list_without_share_controls', async ({ page }) => {
  expect((await fetch('http://127.0.0.1:3400/__seed-sorted')).status).toBe(204);
  await signIn(page);
  await page.goto('/app/slot/?site=rack&slot=grid');
  await expect(page.locator('[data-testid=items-table] tbody tr td:nth-child(2)')).toHaveText(['kaapi', 'filter', 'decoction', 'gift']);
  // Items are not variants: no variant cards, no share to change, no "next draft".
  await expect(page.locator('[data-testid^=variant-]')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Change share' })).toHaveCount(0);
  await expect(page.getByText('of traffic')).toHaveCount(0);
});

// The kiosk site (stack.mjs): a one-hour curve and a copy slot nobody has seen. Last, like rack.
test('copy_slot_card_shows_its_fields_as_labelled_text', async ({ page }) => {
  test.setTimeout(90_000);
  expect((await fetch('http://127.0.0.1:3400/__seed-kiosk')).status).toBe(204);
  await signIn(page);
  await page.goto('/app/slot/?site=kiosk&slot=tagline');
  const card = page.getByTestId('variant-monday');
  const fields = card.getByTestId('copy-fields');
  await expect(fields.locator('dt')).toHaveText(['headline', 'cta_label']);
  await expect(fields.locator('dd')).toHaveText(['Roasted Monday, at your door by Thursday', 'Get this week\'s roast']);
  await expect(card.locator('iframe')).toHaveCount(0);
  await expect(card).not.toContainText('{');
});

test('one_closed_hour_draws_a_dot_per_variant', async ({ page }) => {
  test.setTimeout(90_000);
  expect((await fetch('http://127.0.0.1:3400/__seed-kiosk')).status).toBe(204);
  await signIn(page);
  const s = await api('/sites/kiosk/slots/note');
  expect(s.curve).toHaveLength(1);
  const n = Object.values(s.curve[0].variants).filter(Boolean).length;
  expect(n).toBeGreaterThanOrEqual(1);
  await page.goto('/app/results/?site=kiosk');
  const card = page.getByTestId('result-note');
  // The curve over time sits behind the card's Details.
  await card.getByRole('button', { name: 'Details' }).click();
  await expect(card.getByTestId('lift-dots')).toBeVisible();
  await expect(card.locator('.recharts-scatter-symbol')).toHaveCount(n);
  await expect(card.locator('.recharts-errorBar')).toHaveCount(n);
});

test('results_card_with_no_closed_window_draws_no_empty_chart', async ({ page }) => {
  test.setTimeout(90_000);
  expect((await fetch('http://127.0.0.1:3400/__seed-kiosk')).status).toBe(204);
  await signIn(page);
  await page.goto('/app/results/?site=kiosk');
  const card = page.getByTestId('result-tagline');
  await expect(card).toContainText('no closed windows yet');
  await expect(card.locator('.recharts-wrapper')).toHaveCount(0);
  // Its own height, not stretched to the charted card beside it.
  await page.getByTestId('result-note').getByRole('button', { name: 'Details' }).click();
  await expect(page.getByTestId('result-note').locator('.recharts-wrapper')).toHaveCount(1);
  await expect(card.locator('.recharts-wrapper')).toHaveCount(0);
  expect((await card.boundingBox())!.height).toBeLessThan(120);
  // The workbench says it in one short line, not a frame of axes.
  await page.goto('/app/slot/?site=kiosk&slot=tagline');
  await expect(page.getByTestId('curve-empty')).toHaveText('No closed windows yet.');
  expect((await page.getByTestId('curve-empty').boundingBox())!.height).toBeLessThan(40);
});

// S11: images. A site of its own (applied here), so nothing above sees it.
const PNG_1x1 = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64');
test('an_uploaded_image_becomes_a_draft_shown_as_an_image', async ({ page }) => {
  const file = { site: 'banner', origin: 'http://localhost:3400', slots: [
    { key: 'hero', selector: '#hero', kind: 'copy', copy: { fields: [{ name: 'headline', max: 40 }, { name: 'banner', type: 'image', required: true }] },
      variants: [{ key: 'own', baseline: true }] },
    { key: 'promo', selector: '#promo', variants: [{ key: 'own', baseline: true }] },
  ] };
  const tmp = path.join(mkdtempSync(path.join(tmpdir(), 'prb-s11-')), 'site.json');
  writeFileSync(tmp, JSON.stringify(file));
  execFileSync(path.resolve(import.meta.dirname, '../../target/debug/pratibimba'), ['apply', tmp], {
    env: { ...process.env, PRB_DB: E2E_DB, RUST_LOG: 'warn' }, stdio: 'pipe',
  });
  await signIn(page);
  await page.goto('/app/drafts/?site=banner');
  const shelf = page.getByTestId('asset-shelf');
  // A file that only says it is an image is refused, with the reason.
  await shelf.getByTestId('asset-input').setInputFiles({ name: 'fake.png', mimeType: 'image/png', buffer: Buffer.from('<html>hi</html>') });
  await expect(shelf.getByRole('alert')).toContainText("judged by the file's bytes");
  await shelf.getByTestId('asset-input').setInputFiles({ name: 'autumn.png', mimeType: 'image/png', buffer: PNG_1x1 });
  await expect(shelf.getByTestId('asset-list').locator('li')).toHaveCount(1);
  await expect(shelf.getByTestId('asset-list').locator('li')).toContainText('autumn.png');
  await expect(shelf.getByTestId('asset-list').locator('li')).toContainText('1×1');
  const id = (await api('/sites/banner/assets')).assets[0].id as string;
  expect(await shelf.locator('img').evaluate((i: HTMLImageElement) => i.naturalWidth)).toBe(1);

  // A copy draft: text and an image picked from the uploads, with alt text; no HTML written.
  await page.getByPlaceholder('new_variant').fill('autumn');
  const inputs = page.getByTestId('copy-inputs');
  await inputs.getByLabel('headline').fill('Autumn roast');
  await inputs.getByLabel('banner image').selectOption({ label: 'autumn.png' });
  await inputs.getByLabel('banner alt text').fill('A sack of autumn beans');
  await page.getByRole('button', { name: 'Save as draft' }).click();
  await expect(page.getByTestId('draft-saved')).toContainText('autumn');
  await page.goto('/app/approvals/?site=banner');
  const card = page.getByTestId('card-autumn');
  const img = card.getByTestId('copy-fields').getByTestId('field-image').locator('img');
  await expect(img).toHaveAttribute('src', `/assets/banner/${id}`);
  await expect(img).toHaveAttribute('alt', 'A sack of autumn beans');
  await expect.poll(() => img.evaluate((i: HTMLImageElement) => i.naturalWidth)).toBe(1);
  await expect(card.locator('iframe')).toHaveCount(0);
  await expect(card).not.toContainText('{');

  // A swap draft that shows the image through the site's /_prb door previews it in the sandbox.
  await page.goto('/app/drafts/?site=banner');
  await page.getByRole('combobox').filter({ hasText: 'hero' }).first().selectOption('promo');
  await page.getByPlaceholder('new_variant').fill('wide');
  await page.getByPlaceholder('<h1>…</h1>').fill(`<div style="display:flex;gap:8px"><img src="/_prb/assets/banner/${id}" alt="Beans" width="40" height="40"><h1>Autumn</h1></div>`);
  await page.getByRole('button', { name: 'Save as draft' }).click();
  await expect(page.getByTestId('draft-saved')).toContainText('wide');
  await page.goto('/app/approvals/?site=banner');
  const wide = page.getByTestId('card-wide');
  // The champion is the page's own (no frame); the draft's frame runs nothing.
  await expect(wide.locator('iframe')).toHaveCount(1);
  await expect(wide.locator('iframe')).toHaveAttribute('sandbox', '');
  await expect.poll(() => wide.frameLocator('iframe').locator('img').evaluate((i: HTMLImageElement) => i.complete && i.naturalWidth)).toBe(1);
});

// ---- redesign: stages, tooltips, help ----

test('stage_rail_walks_build_to_observe', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/install/?site=shop');
  const rail = page.getByRole('navigation', { name: 'Stages' });
  for (const [stage, url] of [['Build', /\/install\//], ['Approve', /\/approvals\//], ['Run', /\/site\//], ['Learn', /\/results\//], ['Observe', /\/audit\//]] as const) {
    await rail.getByRole('link', { name: new RegExp(stage) }).click();
    await expect(page).toHaveURL(url);
    await expect(rail.getByRole('link', { name: new RegExp(stage) })).toHaveAttribute('aria-current', 'step');
  }
  // Each stage page offers the next stage at its foot.
  await page.goto('/app/install/?site=shop');
  await page.getByRole('link', { name: /Next.*Approve/ }).click();
  await expect(page).toHaveURL(/\/approvals\//);
});

test('tooltips_open_on_keyboard_focus_and_help_on_click', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/site/?id=shop');
  const learn = page.getByRole('navigation', { name: 'Stages' }).getByRole('link', { name: /Learn/ });
  await learn.focus();
  await expect(page.getByRole('tooltip')).toHaveText('What the outcomes taught the model');
  await expect(learn).toHaveAttribute('aria-describedby', /.+/);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('tooltip')).toHaveCount(0);
  const help = page.getByRole('button', { name: 'Privacy' });
  await help.click();
  await expect(help).toHaveAttribute('aria-expanded', 'true');
  await expect(page.getByRole('dialog', { name: 'Privacy' })).toContainText('session-scoped');
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog', { name: 'Privacy' })).toHaveCount(0);
  await expect(help).toBeFocused();
});

// ---- B4: personas, rendered approval cards, history settings (a site of their own) ----

let dhaba = false;
function applyDhaba() {
  if (dhaba) return;
  const file = { site: 'dhaba', origin: 'http://localhost:3400', privacy_mode: 'full', slots: [
    { key: 'hero_copy', selector: '#hero', kind: 'copy', copy: { fields: [{ name: 'title', max: 40, required: true }] },
      variants: [
        { key: 'own', baseline: true },
        { key: 'your_usual', requires: ['visitor.top_item'], copy: { title: 'Your usual {visitor.top_item.name | truncate: 14} is ready' } },
      ] },
  ] };
  const tmp = path.join(mkdtempSync(path.join(tmpdir(), 'prb-b4-')), 'site.json');
  writeFileSync(tmp, JSON.stringify(file));
  execFileSync(path.resolve(import.meta.dirname, '../../target/debug/pratibimba'), ['apply', tmp], { env: { ...process.env, PRB_DB: E2E_DB, RUST_LOG: 'warn' }, stdio: 'pipe' });
  dhaba = true;
}

// Mutation: send no persona to the page -> the hero keeps its fallback, red.
test('preview persona shows filled hero', async ({ page }) => {
  applyDhaba();
  await signIn(page);
  await page.goto('/app/preview/?site=dhaba');
  const site = page.frameLocator('iframe[title="Your site, in preview"]');
  const title = site.locator('#hero [data-prb-field=title]');
  await expect(title).toHaveText('Home-style food, cooked today');
  await page.getByTestId('persona-usual_rajma').click();
  await expect(page.getByTestId('persona-usual_rajma')).toHaveAttribute('aria-pressed', 'true');
  // The page has answered for this visitor once the design no longer falls through.
  await expect(page.getByTestId('variant-your_usual').getByTestId('falls-through')).toHaveCount(0);
  await page.getByTestId('variant-your_usual').click();
  await expect(title).toHaveText('Your usual rajma-chawal is ready');
  // Back to a first visit: the design needs a top item this visitor lacks, so the page keeps its own.
  await page.getByRole('button', { name: 'First visit' }).click();
  await expect(page.getByTestId('variant-your_usual').getByTestId('falls-through')).toBeVisible();
  await page.getByTestId('variant-your_usual').click();
  await expect(title).toHaveText('Home-style food, cooked today');
});

// Mutation: open the source by default -> the source shows first, red.
test('approval card renders a template for each persona', async ({ page }) => {
  applyDhaba();
  const r = await apiPost(`${CORE}/api/sites/dhaba/slots/hero_copy/variants`, { key: 'usual_again', html: JSON.stringify({ title: 'Your usual {visitor.top_item.name | truncate: 14}, again?' }), requires: ['visitor.top_item'] });
  expect(r.status()).toBe(201);
  await signIn(page);
  await page.goto('/app/approvals/?site=dhaba');
  const card = page.getByTestId('card-usual_again');
  const renders = card.getByTestId('renders');
  await expect(renders.locator('[data-testid^=render-]')).toHaveCount(4);
  await expect(renders.locator('dd').first()).toHaveText('Your usual masala-dosa, again?');
  await expect(card.getByTestId('render-usual_rajma')).toContainText('Your usual rajma-chawal, again?');
  await expect(card.getByTestId('render-first_visit')).toContainText('Falls to own');
  await expect(card.getByTestId('template-source')).toHaveCount(0);
  await expect(card).not.toContainText('{visitor');
  await card.getByRole('button', { name: 'Source' }).click();
  await expect(card.getByTestId('template-source')).toContainText('{visitor.top_item.name | truncate: 14}, again?');
});

// Mutation: post the default look-back whatever was typed -> the saved value stays 180, red.
test('history settings save changes the value', async ({ page }) => {
  applyDhaba();
  await signIn(page);
  await page.goto('/app/drafts/?site=dhaba');
  const panel = page.getByTestId('history-settings');
  const days = panel.getByLabel('Look back (days)');
  await expect(days).toHaveValue('180');
  await expect(panel).toContainText('1–730, default 180');
  // Out of bounds: Save stays off.
  await days.fill('731');
  await expect(panel.getByRole('button', { name: 'Save' })).toBeDisabled();
  await days.fill('90');
  await panel.getByRole('radio', { name: 'A session' }).click();
  await panel.getByLabel('Session gap (minutes)').fill('45');
  await panel.getByRole('button', { name: 'Save' }).click();
  await expect(panel.getByRole('status')).toHaveText('Saved');
  const got = await api('/sites/dhaba/history-settings');
  expect([got.top_item_days, got.visit, got.session_gap_minutes]).toEqual([90, 'session', 45]);
  await page.reload();
  await expect(page.getByTestId('history-settings').getByLabel('Look back (days)')).toHaveValue('90');
  // Restore defaults, then save them.
  await page.getByTestId('history-settings').getByRole('button', { name: 'Restore defaults' }).click();
  await expect(page.getByTestId('history-settings').getByLabel('Look back (days)')).toHaveValue('180');
  await page.getByTestId('history-settings').getByRole('button', { name: 'Save' }).click();
  await expect(page.getByTestId('history-settings').getByRole('status')).toHaveText('Saved');
  const back = await api('/sites/dhaba/history-settings');
  expect([back.top_item_days, back.visit]).toEqual([180, 'day']);
});

// ---- B8: the advisor ----

let cafe = false;
/** Site `cafe`: a strip slot with two live versions and one pending pause card. */
function applyCafe() {
  if (cafe) return;
  const file = { site: 'cafe', origin: 'http://localhost:3400', privacy_mode: 'full', slots: [
    { key: 'kitchen_strip', selector: '#strip', metric: 'purchase', variants: [
      { key: 'own', baseline: true },
      { key: 'function_orders', html: '<p>We cater functions</p>' },
      { key: 'fresh_daily', html: '<p>Cooked fresh daily</p>' },
    ] },
  ] };
  const tmp = path.join(mkdtempSync(path.join(tmpdir(), 'prb-b8-')), 'site.json');
  writeFileSync(tmp, JSON.stringify(file));
  execFileSync(path.resolve(import.meta.dirname, '../../target/debug/pratibimba'), ['apply', tmp], { env: { ...process.env, PRB_DB: E2E_DB, RUST_LOG: 'warn' }, stdio: 'pipe' });
  const evidence = {
    claim: "kitchen_strip: all 2 versions are within ±5 orders per 100 of the page's own over 8,147 visits. It makes no difference.",
    rows: 8147, visits_per_week: 1900, margin: 5, p: 0.004, state: 'confident',
    arms: [{ key: 'function_orders', uplift: 1.44, lo: -1.22, hi: 4.11, n: 2700 }, { key: 'fresh_daily', uplift: -1.58, lo: -4.77, hi: 1.61, n: 2650 }],
    held_out: [{ key: 'function_orders', uplift: -1.09, lo: -5, hi: 3, n: 900 }, { key: 'fresh_daily', uplift: 0.72, lo: -3, hi: 4, n: 880 }],
  };
  execFileSync('psql', [E2E_DB, '-v', 'ON_ERROR_STOP=1', '-qc',
    `INSERT INTO advisor_cards (site_id, slot_id, kind, state, evidence) SELECT 'cafe', id, 'pause_slot', 'confident', '${JSON.stringify(evidence).replace(/'/g, "''")}' FROM slots WHERE site_id = 'cafe' ON CONFLICT DO NOTHING`], { stdio: 'pipe' });
  cafe = true;
}

// Mutation: the panel posts an empty body (the change never reaches Core) -> red.
test('advisor settings save changes the value', async ({ page }) => {
  applyCafe();
  await signIn(page);
  await page.goto('/app/approvals/?site=cafe');
  const panel = page.getByTestId('advisor-settings');
  const margin = panel.getByLabel('Margin (per 100)', { exact: true });
  await expect(margin).toHaveValue('0.3');
  await expect(panel).toContainText('needs ~77k visits per version');
  // Out of bounds: Save stays off.
  await margin.fill('80');
  await expect(panel.getByRole('button', { name: 'Save' })).toBeDisabled();
  await margin.fill('3');
  await panel.getByLabel('Quiet after reject (days)', { exact: true }).fill('10');
  await panel.getByRole('button', { name: 'Save' }).click();
  await expect(panel.getByRole('status')).toHaveText('Saved');
  const got = await api('/sites/cafe/advisor-settings');
  expect([got.margin, got.quiet_days, got.min_rows]).toEqual([3, 10, 300]);
  await page.reload();
  await expect(page.getByTestId('advisor-settings').getByLabel('Margin (per 100)', { exact: true })).toHaveValue('3');
  await page.getByTestId('advisor-settings').getByRole('button', { name: 'Restore defaults' }).click();
  await page.getByTestId('advisor-settings').getByRole('button', { name: 'Save' }).click();
  await expect(page.getByTestId('advisor-settings').getByRole('status')).toHaveText('Saved');
  const back = await api('/sites/cafe/advisor-settings');
  expect([back.margin, back.quiet_days]).toEqual([0.3, 30]);
});

// Mutation: Approve pause posts to /reject -> red (no version paused).
test('advisor card approve pauses the versions and Results tracks it', async ({ page }) => {
  applyCafe();
  await signIn(page);
  await page.evaluate(() => localStorage.setItem('prb-approver', 'Asha')).catch(() => {});
  await page.goto('/app/approvals/?site=cafe');
  await page.evaluate(() => { localStorage.setItem('prb-approver', 'Asha'); window.dispatchEvent(new Event('prb-actor')); });
  const card = page.getByTestId('advisor-card-kitchen_strip');
  await expect(card.getByTestId('advisor-claim')).toHaveText('Kitchen strip makes no clear difference: its 2 versions get about as many orders as the page’s own.');
  await expect(card).toContainText('About 1,900 visits a week');
  await card.getByRole('button', { name: 'Details' }).click();
  await expect(card.getByTestId('advisor-evidence')).toContainText('+1.4 (−1.2 to +4.1)');
  await card.getByRole('button', { name: 'Approve pause' }).click();
  await expect(card).toBeHidden();
  const slot = await api('/sites/cafe/slots/kitchen_strip');
  const status = Object.fromEntries(slot.variants.map((v: { key: string; status: string }) => [v.key, v.status]));
  expect(status).toEqual({ own: 'approved', function_orders: 'paused', fresh_daily: 'paused' });
  await page.goto('/app/results/?site=cafe');
  await expect(page.getByTestId('result-kitchen_strip').getByTestId('advisor-line')).toContainText('Advisor suggestion: paused');
});

// Time travel: a sandbox copy of the site, one visitor skipped forward in it.
test('time_travel_skips_a_visitor_forward_in_the_sandbox', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/time-travel/?site=shop');
  await expect(page.getByTestId('tt-empty')).toContainText('No sandbox yet');
  await page.getByRole('button', { name: 'Start a sandbox' }).click();
  await expect(page.getByTestId('tt-clock')).toBeVisible();
  const before = new Date((await api('/sites/shop/sim')).clock).getTime();
  await page.getByLabel('Visitor').fill('meera');
  await page.getByLabel('Days').fill('7');
  await page.getByRole('button', { name: 'Skip forward' }).click();
  const result = page.getByTestId('tt-result');
  await expect(result).toContainText('meera');
  for (const slot of ['hero', 'banner']) await expect(result.getByTestId(`tt-slot-${slot}`).getByTestId('tt-variant')).not.toBeEmpty();
  const after = new Date((await api('/sites/shop/sim')).clock).getTime();
  expect(Math.round((after - before) / 86_400_000)).toBe(7);
  // Start over wipes the sandbox clock back to now and clears the answer.
  await page.getByRole('button', { name: 'Start over' }).click();
  await expect(result).toBeHidden();
});

test('time_travel_is_reached_from_the_stage_rail', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/results/?site=shop');
  await page.getByRole('link', { name: 'Time travel' }).first().click();
  await expect(page).toHaveURL(/\/app\/time-travel\/\?site=shop/);
  await expect(page.getByRole('heading', { name: 'Time travel' })).toBeVisible();
});

// The slot workbench's agent sidebar: pack status, a brief that becomes a prompt, a slot-scoped tail.
test('slot_agent_panel_shows_pack_status_prompt_and_only_this_slots_calls', async ({ page }) => {
  await signIn(page);
  const c = await pwRequest.newContext({ baseURL: CORE, extraHTTPHeaders: { authorization: `Bearer ${TOKEN}` } });
  const { token } = await (await c.post('/api/sites/shop/mcp-tokens', { data: { name: 'e2e-slot-panel' } })).json();
  await c.dispose();
  const current = (await api('/sites/shop/mcp-calls')).pack_version;
  await mcpCall(token, 'list_variants', { agent: 'Claude Code', justification: 'slot panel e2e', pack: current, slot: 'hero' });
  await mcpCall(token, 'list_variants', { agent: 'Cursor', justification: 'slot panel e2e, old pack', pack: '0.0.1', slot: 'banner' });
  await page.goto('/app/slot/?site=shop&slot=hero');
  const panel = page.getByTestId('slot-agent');
  await expect(panel.getByTestId('copy-slot-endpoint')).toHaveText(`${CORE}/mcp`);
  await expect(panel.getByTestId('pack-Claude-Code')).toContainText('pack installed');
  await expect(panel.getByTestId('pack-Cursor')).toContainText('pack out of date');
  await expect(panel.getByTestId('pack-Generic-MCP')).toContainText('not installed');
  // The tail is this slot's calls (what the API says), newest first, and not the banner one.
  const tail = panel.getByTestId('slot-mcp-tail');
  const scoped = (await api('/sites/shop/mcp-calls?slot=hero&limit=6')).calls;
  const banner = (await api('/sites/shop/mcp-calls')).calls.find((x: any) => x.justification === 'slot panel e2e, old pack');
  expect(scoped.some((x: any) => x.id === banner.id)).toBe(false);
  await expect(tail.getByTestId('slot-mcp-call')).toHaveCount(scoped.length);
  await expect(tail.getByTestId('slot-mcp-call').first()).toContainText('list_variants from Claude Code');
  await expect(tail.locator(`a[href*="id=${banner.id}"]`)).toHaveCount(0);
  // A new call on the slot arrives without a reload.
  await mcpCall(token, 'get_results', { agent: 'Claude Code', justification: 'slot panel e2e, live', pack: current, slot: 'hero' });
  await expect(tail.getByTestId('slot-mcp-call').first()).toContainText('get_results from Claude Code', { timeout: 10_000 });
  // The brief becomes a prompt with the slot in it; nothing is sent.
  await panel.getByLabel('Brief').fill('make it warmer for returning buyers');
  const prompt = panel.getByTestId('copy-prompt');
  await expect(prompt).toContainText('"hero"');
  await expect(prompt).toContainText('Brief: make it warmer for returning buyers');
  // A token is minted on request and shown once, copy-ready.
  await panel.getByRole('button', { name: 'Mint a token for this site' }).click();
  await expect(panel.getByTestId('copy-slot-token')).toContainText('prb_mcp_');
});
