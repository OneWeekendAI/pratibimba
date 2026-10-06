// End to end through @pratibimba/next and a real Core: the proxy names the visitor's zone from
// the platform's geolocation header (or the beacon's cookie), the slot sends it with its
// server-side decide, and the live worker learns from the local-time features Core derives.
// A planted "night visitors prefer gift" cohort is learned; with no zone it stays at a coin.
//
//   cargo build && (cd packages/next && npm run e2e)

import { test, after, before } from 'node:test';
import assert from 'node:assert/strict';
import { NextRequest } from 'next/server.js';
import { createPratibimba } from '../../src/index.ts';
import { MEASURE, SETTLE_EVERY, VISITORS, heroSite, rng, settled, startCore, truth, zonesNow, type Core } from './core.ts';

let core: Core;

before(async () => {
  core = await startCore('prb_next_time_e2e', 18092, [heroSite('night-one'), heroSite('night-cookie'), heroSite('learn'), heroSite('control')]);
});
after(() => core?.stop());

/**
 * One request as Next.js runs it: the proxy, then a server render whose `<Slot>` decides with
 * the headers the proxy forwarded, then the beacon's events through the `/_prb` route.
 */
async function visit(site: string, headers: Record<string, string>, click: (variant: string) => boolean) {
  let forwarded = new Headers();
  const scope = {};
  const prb = createPratibimba({ core: core.url, site, timeoutMs: 2000, headers: async () => forwarded, requestScope: () => scope });
  const res = prb.proxy(new NextRequest('https://shop.test/', { headers: { 'user-agent': 'Mozilla/5.0', ...headers } }));
  // Next hands the render the request headers the proxy overrode, under this prefix.
  forwarded = new Headers();
  res.headers.forEach((v, k) => {
    if (k.startsWith('x-middleware-request-')) forwarded.set(k.slice('x-middleware-request-'.length), v);
  });
  const d = await prb.decide('hero');
  assert.ok(d, 'no decision');
  const events = [{ id: crypto.randomUUID(), name: 'exposure', slot: 'hero', decision_id: d.decision_id }];
  if (click(d.variant)) events.push({ id: crypto.randomUUID(), name: 'click', slot: 'hero', decision_id: d.decision_id });
  const subject = forwarded.get('x-prb-subject');
  const e = await prb.route.POST(
    new Request('https://shop.test/_prb/e', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ site, subject, events }) }),
  );
  assert.equal(e.status, 202, await e.text());
  return d;
}

test('next_decides_with_the_night_visitors_local_clock', async () => {
  const { night } = zonesNow();
  const d = await visit('night-one', { 'x-vercel-ip-timezone': night[0] }, () => false);
  assert.equal(d.explanation.policy, 'lints_time', `zone ${night[0]}`);
  assert.equal(d.explanation.features.is_night_local, 1, `zone ${night[0]}`);
});

test('next_decides_with_the_cookie_zone_when_no_geo_header', async () => {
  const { night } = zonesNow();
  const d = await visit('night-cookie', { cookie: `_prb_tz=${encodeURIComponent(night[0])}` }, () => false);
  assert.equal(d.explanation.features.is_night_local, 1, `zone ${night[0]}`);
});

async function cohort(site: string, sendZone: boolean, seed: number): Promise<number> {
  const { night: nightZones, day: dayZones } = zonesNow();
  const r = rng(seed);
  let matched = 0;
  for (let i = 0; i < VISITORS; i++) {
    const night = r() < 0.5;
    const zones = night ? nightZones : dayZones;
    const zone = zones[Math.floor(r() * zones.length)];
    const u = r();
    const d = await visit(site, sendZone ? { 'x-vercel-ip-timezone': zone } : {}, (v) => u < truth(v, night));
    if (i >= VISITORS - MEASURE) matched += Number((d.variant === 'gift') === night);
    if ((i + 1) % SETTLE_EVERY === 0) await settled(core, site);
  }
  const m = matched / MEASURE;
  console.log(`next cohort ${site} zone=${sendZone}: shown what their hour prefers ${m.toFixed(2)}`);
  return m;
}

test('next_cohort_learns_that_night_visitors_prefer_gift', { timeout: 300_000 }, async () => {
  const m = await cohort('learn', true, 21);
  assert.ok(m >= 0.8, `shown what their hour prefers: ${m.toFixed(2)}`);
});

test('next_cohort_without_a_zone_stays_at_a_coin', { timeout: 300_000 }, async () => {
  const m = await cohort('control', false, 21);
  assert.ok(m >= 0.3 && m <= 0.65, `shown what their hour prefers: ${m.toFixed(2)}`);
});
