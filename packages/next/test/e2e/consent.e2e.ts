// End to end through @pratibimba/next and a real Core on a consent_aware site: the visitor's
// consent cookie reaches Core as `ctx.consent`, so Core files the decision and the server-side
// goal under the visitor's own id; without it, under a daily pseudonym.
//
//   cargo build && (cd packages/next && npm run e2e)

import { test, after, before } from 'node:test';
import assert from 'node:assert/strict';
import { NextRequest } from 'next/server.js';
import { createPratibimba } from '../../src/index.ts';
import { heroSite, startCore, type Core } from './core.ts';

let core: Core;
before(async () => {
  core = await startCore('prb_next_consent_e2e', 18094, [heroSite('consent')]);
});
after(() => core?.stop());

async function visit(cookie: string | undefined) {
  let forwarded = new Headers();
  const scope = {};
  const prb = createPratibimba({ core: core.url, site: 'consent', timeoutMs: 2000, headers: async () => forwarded, requestScope: () => scope });
  const res = prb.proxy(new NextRequest('https://shop.test/', { headers: { 'user-agent': 'Mozilla/5.0', ...(cookie ? { cookie } : {}) } }));
  res.headers.forEach((v, k) => {
    if (k.startsWith('x-middleware-request-')) forwarded.set(k.slice('x-middleware-request-'.length), v);
  });
  const d = await prb.decide('hero');
  assert.ok(d, 'no decision');
  await prb.track('purchase', { slot: 'hero', decisionId: d.decision_id });
  const subject = forwarded.get('x-prb-subject')!;
  return {
    subject,
    decision: core.sql(`SELECT subject FROM decisions WHERE id = '${d.decision_id}'`),
    event: core.sql(`SELECT subject FROM events WHERE decision_id = '${d.decision_id}' AND name = 'purchase'`),
  };
}

test('next_consented_visitor_is_stored_under_their_own_id', async () => {
  const v = await visit('_prb_consent=granted');
  assert.deepEqual([v.decision, v.event], [v.subject, v.subject]);
});

test('next_visitor_without_consent_is_stored_as_a_pseudonym', async () => {
  const v = await visit(undefined);
  assert.ok(v.decision.startsWith('p1_') && v.event === v.decision, JSON.stringify(v));
});
