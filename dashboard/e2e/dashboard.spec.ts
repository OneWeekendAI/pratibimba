// The dashboard, driven in Chromium against real Core and a seeded history. One test per
// behaviour; the gate tests run last because they change data.
import { test, expect, request as pwRequest, type Page } from '@playwright/test';

const CORE = 'http://127.0.0.1:18083';
const TOKEN = 'dash-admin-token';

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

test('results_headline_matches_the_api', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/results/?site=shop');
  const r = await api('/sites/shop/results');
  await expect(page.getByTestId('headline')).toContainText(pp(r.results.lift.estimate));
  await expect(page.getByTestId('headline')).toContainText(`${pp(r.results.lift.lo)} to ${pp(r.results.lift.hi)}`);
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
    const stripe = await page.getByTestId(`variant-${key}`).locator('div.h-1').first().evaluate((e) => (e as HTMLElement).style.background);
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
  await page.goto('/app/site/?id=shop');
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
  await page.goto('/app/site/?id=shop');
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
  await page.goto('/app/site/?id=shop');
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
    await banner.getByLabel('Your name for the kill switch').fill('Ritwik');
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

test('sidebar_runs_the_full_height_of_a_long_page', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/site/?id=shop');
  await expect(page.getByTestId('mcp-tail')).toBeVisible();
  const total = await page.evaluate(() => document.documentElement.scrollHeight);
  expect(total).toBeGreaterThan(1200);
  const box = (await page.getByTestId('sidebar').boundingBox())!;
  expect(box.y + box.height).toBeGreaterThanOrEqual(total - 1);
  await page.evaluate(() => scrollTo(0, document.body.scrollHeight));
  await expect(page.getByRole('button', { name: 'Sign out' })).toBeInViewport();
});

test('approval_card_diffs_against_the_champion', async ({ page }) => {
  await signIn(page);
  await page.goto('/app/approvals/?site=shop');
  const card = page.getByTestId('card-gift_ribbon');
  await expect(card).toContainText('Current champion: gift');
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
  await page.getByLabel('Reviewer name').fill('Ritwik');
  const card = page.getByTestId('card-popup_offer');
  await expect(card.getByTestId('checks')).toContainText('technical · fail');
  await expect(card.getByRole('button', { name: /^Approve/ })).toBeDisabled();
});

test('approve_takes_one_card_and_leaves_the_other', async ({ page }) => {
  await apiPost('/api/sites/shop/slots/banner/variants', { key: 'spring_beans', html: '<p>Spring harvest is in.</p>' });
  await signIn(page);
  await page.goto('/app/approvals/?site=shop');
  await page.getByLabel('Reviewer name').fill('Ritwik');
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
  await page.getByLabel('Reviewer name').fill('Ritwik');
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
  await page.goto('/app/approvals/?site=shop&slot=hero');
  await page.getByLabel('Key').fill('autumn_hero');
  await page.getByLabel('HTML').fill('<h1>Autumn roasts are here</h1>');
  await page.getByRole('button', { name: 'Save as draft' }).click();
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
