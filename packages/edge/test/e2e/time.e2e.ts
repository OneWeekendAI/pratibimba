// End to end through @pratibimba/edge and a real Core: the edge names the visitor's zone from
// the platform's geolocation header, Core turns it into local-time features, and the live
// worker learns from them. A planted "night visitors prefer gift" cohort is learned; the same
// cohort with no zone header stays at a coin flip.
//
//   cargo build && (cd packages/edge && npm run e2e)

import { test, after, before } from 'node:test';
import assert from 'node:assert/strict';
import { createEdge } from '../../src/index.ts';
import { MEASURE, SETTLE_EVERY, VISITORS, heroSite, rng, settled, startCore, truth, zonesNow, type Core } from './core.ts';

const PAGE = '<html><body><!-- prb:slot="hero" --><h1>Espresso</h1><!-- /prb:slot --></body></html>';
let core: Core;

before(async () => {
  core = await startCore('prb_edge_time_e2e', 18091, [heroSite('night-one'), heroSite('learn'), heroSite('control')]);
});
after(() => core?.stop());

function edgeFor(site: string) {
  return createEdge({ core: core.url, site, origin: async () => new Response(PAGE, { headers: { 'content-type': 'text/html' } }) });
}

/** The decisions the edge wrote into its beacon tag, as the browser would read them. */
function beaconConfig(html: string): { subject: string; decisions: { slot: string; id: string }[] } {
  const m = /data-prb='([^']*)'/.exec(html);
  assert.ok(m, `no beacon tag in ${html}`);
  return JSON.parse(m[1].replace(/&#39;/g, "'").replace(/&lt;/g, '<').replace(/&amp;/g, '&'));
}

/** One page view; then the beacon's exposure and, maybe, a click, posted through the edge. */
async function visit(site: string, edge: ReturnType<typeof createEdge>, headers: Record<string, string>, click: (variant: string) => boolean) {
  const res = await edge.handleRequest(new Request('https://shop.test/', { headers }));
  const html = await res.text();
  const variant = html.includes('Gift sets') ? 'gift' : 'baseline';
  const cfg = beaconConfig(html);
  const d = cfg.decisions[0];
  const events = [{ id: crypto.randomUUID(), name: 'exposure', slot: d.slot, decision_id: d.id }];
  if (click(variant)) events.push({ id: crypto.randomUUID(), name: 'click', slot: d.slot, decision_id: d.id });
  const e = await edge.handleRequest(
    new Request('https://shop.test/_prb/e', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ site, subject: cfg.subject, events }) }),
  );
  assert.equal(e.status, 202, await e.text());
  return { variant, decision: d.id };
}

test('edge_decides_with_the_night_visitors_local_clock', async () => {
  const { night } = zonesNow();
  const { decision } = await visit('night-one', edgeFor('night-one'), { 'x-vercel-ip-timezone': night[0] }, () => false);
  const row = core.sql(`SELECT policy || ' ' || ((features->>'is_night_local')::float) FROM decisions WHERE id = '${decision}'`);
  assert.equal(row, 'lints_time 1', `zone ${night[0]}`);
});

async function cohort(site: string, sendZone: boolean, seed: number): Promise<number> {
  const edge = edgeFor(site);
  const { night: nightZones, day: dayZones } = zonesNow();
  const r = rng(seed);
  let matched = 0;
  for (let i = 0; i < VISITORS; i++) {
    const night = r() < 0.5;
    const zones = night ? nightZones : dayZones;
    const zone = zones[Math.floor(r() * zones.length)];
    const u = r();
    const { variant } = await visit(site, edge, sendZone ? { 'x-vercel-ip-timezone': zone } : {}, (v) => u < truth(v, night));
    if (i >= VISITORS - MEASURE) matched += Number((variant === 'gift') === night);
    if ((i + 1) % SETTLE_EVERY === 0) await settled(core, site);
  }
  const m = matched / MEASURE;
  console.log(`edge cohort ${site} zone=${sendZone}: shown what their hour prefers ${m.toFixed(2)}`);
  return m;
}

test('edge_cohort_learns_that_night_visitors_prefer_gift', { timeout: 300_000 }, async () => {
  const m = await cohort('learn', true, 21);
  assert.ok(m >= 0.8, `shown what their hour prefers: ${m.toFixed(2)}`);
});

test('edge_cohort_without_a_zone_stays_at_a_coin', { timeout: 300_000 }, async () => {
  const m = await cohort('control', false, 21);
  assert.ok(m >= 0.3 && m <= 0.65, `shown what their hour prefers: ${m.toFixed(2)}`);
});
