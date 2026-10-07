// End to end through @pratibimba/edge and a real Core on a consent_aware site: the visitor's
// consent cookie reaches Core as `ctx.consent`, so Core files the decision under the visitor's
// own id; without it, under a daily pseudonym. The beacon's exposure carries the same answer
// (as beacon.js sends it), so the event lands under the same subject as its decision.
//
//   cargo build && (cd packages/edge && npm run e2e)

import { test, after, before } from 'node:test';
import assert from 'node:assert/strict';
import { createEdge } from '../../src/index.ts';
import { heroSite, startCore, type Core } from './core.ts';

const PAGE = '<html><body><!-- prb:slot="hero" --><h1>Espresso</h1><!-- /prb:slot --></body></html>';
let core: Core;
before(async () => {
  core = await startCore('prb_edge_consent_e2e', 18095, [heroSite('consent')]);
});
after(() => core?.stop());

async function visit(cookie: string | undefined) {
  const edge = createEdge({ core: core.url, site: 'consent', timeoutMs: 2000, origin: async () => new Response(PAGE, { headers: { 'content-type': 'text/html' } }) });
  const html = await (await edge.handleRequest(new Request('https://shop.test/', { headers: cookie ? { cookie } : {} }))).text();
  const m = /data-prb='([^']*)'/.exec(html);
  assert.ok(m, html);
  const cfg = JSON.parse(m[1].replace(/&#39;/g, "'").replace(/&lt;/g, '<').replace(/&amp;/g, '&'));
  const d = cfg.decisions[0];
  const body = { site: 'consent', subject: cfg.subject, events: [{ id: crypto.randomUUID(), name: 'exposure', slot: d.slot, decision_id: d.id }], consent: cfg.consent === true };
  const e = await edge.handleRequest(new Request('https://shop.test/_prb/e', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) }));
  assert.equal(e.status, 202, await e.text());
  return {
    subject: cfg.subject as string,
    decision: core.sql(`SELECT subject FROM decisions WHERE id = '${d.id}'`),
    event: core.sql(`SELECT subject FROM events WHERE decision_id = '${d.id}' AND name = 'exposure'`),
  };
}

test('edge_consented_visitor_is_stored_under_their_own_id', async () => {
  const v = await visit('_prb_consent=granted');
  assert.deepEqual([v.decision, v.event], [v.subject, v.subject]);
});

test('edge_visitor_without_consent_is_stored_as_a_pseudonym', async () => {
  const v = await visit(undefined);
  assert.ok(v.decision.startsWith('p1_') && v.event === v.decision, JSON.stringify(v));
});
