// EC3: the write buffer. With QUEUE_WRITES=on, /e and the edge's decision records go onto the
// events queue instead of to Core; the consumer sends them to Core's /e/batch and acks only on a
// 2xx. With it off, or for anything the queue cannot vouch for, the direct path is unchanged.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { RETRY_DELAYS_S, queueable } from '../src/index.js';
import { snapshot, harness, TOKEN, NOON } from './harness.mjs';

const uuid = (n) => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
const events = (extra = {}) => ({ site: 'shop', subject: 'v-1', events: [{ id: uuid(1), name: 'exposure', slot: 'hero', decision_id: uuid(9) }, { id: uuid(2), name: 'click' }], tz_offset: 330, consent: true, ...extra });

test('queue on: /e is queued with the time the edge received it, answered 202, and Core is not called', async () => {
  const h = harness({ queueWrites: 'on' });
  const body = events();
  const r = await h.call(body, '/e');
  assert.equal(r.status, 202);
  assert.deepEqual(r.body, { queued: 2 });
  assert.equal(r.headers.get('x-prb-queued'), 'yes');
  assert.equal(h.core.length, 0, 'Core was called');
  assert.deepEqual(h.queued, [{ kind: 'events', received_at: new Date(NOON).toISOString(), request: body }]);
});

test('queue off: /e goes to Core and nothing is queued (the direct path)', async () => {
  const h = harness({ queueWrites: 'off' });
  const r = await h.call(events(), '/e');
  assert.equal(r.status, 202);
  assert.equal(h.core.filter((c) => c.path === '/e').length, 1);
  assert.equal(h.queued.length, 0);
});

test('what the queue cannot vouch for goes straight to Core', async () => {
  const cases = {
    pick: events({ events: [{ id: uuid(3), name: 'pick', slot: 'hero', picks: ['a'] }] }),
    rescore: events({ events: [{ id: uuid(3), name: 'rescore', slot: 'hero' }] }),
    'unknown field': events({ email: 'a@b.c' }),
    'unknown event field': events({ events: [{ id: uuid(3), name: 'click', email: 'x' }] }),
    'bad id': events({ events: [{ id: 'not-a-uuid', name: 'click' }] }),
    'bad name': events({ events: [{ id: uuid(3), name: 'Click!' }] }),
    'no events': events({ events: [] }),
    'bad subject': events({ subject: 'has space' }),
    'bad tz': events({ tz_offset: 9999 }),
  };
  for (const [what, body] of Object.entries(cases)) {
    assert.equal(queueable(body), false, what);
    const h = harness({ queueWrites: 'on' });
    const r = await h.call(body, '/e');
    assert.equal(h.queued.length, 0, `${what} was queued`);
    assert.equal(h.core.filter((c) => c.path === '/e').length, 1, `${what} did not reach Core`);
    assert.match(r.headers.get('x-prb-queued'), /^no/);
  }
  assert.equal(queueable(events()), true);
});

test('a queue that refuses the message: the event goes to Core instead of being lost', async () => {
  const h = harness({ queueWrites: 'on', queueFails: true });
  const r = await h.call(events(), '/e');
  assert.equal(r.status, 202);
  assert.equal(h.core.filter((c) => c.path === '/e').length, 1);
});

test('CORS on a queued answer: only for an origin the site allows', async () => {
  const h = harness({ queueWrites: 'on' });
  h.env.SITE_ORIGINS = 'https://shop.example, https://b.example';
  const ok = await h.call(events(), '/e', { origin: 'https://shop.example' });
  assert.equal(ok.headers.get('access-control-allow-origin'), 'https://shop.example');
  const no = await h.call(events(), '/e', { origin: 'https://evil.example' });
  assert.equal(no.headers.get('access-control-allow-origin'), null);
});

test('queue on: an edge decision record is queued, not posted to /edge/decisions', async () => {
  const h = harness({ kv: snapshot(), beaconSnap: snapshot(), queueWrites: 'on' });
  const r = await h.call({ site: 'shop', slot: 'hero', subject: 'v-1', ctx: { consent: true } });
  assert.equal(r.headers.get('x-prb-decided-by'), 'edge');
  assert.equal(h.ingests().length, 0, 'the record went to /edge/decisions');
  assert.equal(h.queued.length, 1);
  const [m] = h.queued;
  assert.equal(m.kind, 'decisions');
  assert.equal(m.request.site, 'shop');
  assert.equal(m.request.subject, 'v-1');
  assert.equal(m.request.consent, true);
  assert.equal(m.request.snapshot_version, 1);
  assert.equal(m.request.decisions[0].id, r.body.decision_id);
});

test('queue on but the queue refuses: the record is posted to /edge/decisions', async () => {
  const h = harness({ kv: snapshot(), beaconSnap: snapshot(), queueWrites: 'on', queueFails: true });
  await h.call({ site: 'shop', slot: 'hero', subject: 'v-1' });
  assert.equal(h.ingests().length, 1);
});

/** A delivered batch, recording what the consumer did with it. */
function delivery(bodies, attempts = 1) {
  const did = { ack: 0, retry: [] };
  return {
    did,
    batch: {
      messages: bodies.map((body, i) => ({ id: `m${i}`, body, attempts, timestamp: new Date(NOON - 2000) })),
      ackAll: () => { did.ack++; },
      retryAll: (o) => { did.retry.push(o); },
    },
  };
}

test('consumer: one /e/batch with every message, acked on a 2xx', async () => {
  let seen = null;
  const h = harness({ coreBatch: (body, headers) => { seen = { body, auth: headers.get('authorization') }; return Response.json({ accepted: [uuid(1)], duplicate: [uuid(2)], refused: [{ id: uuid(3), reason: 'erased' }] }); } });
  const bodies = [{ kind: 'events', received_at: 'x', request: events() }, { kind: 'decisions', request: { site: 'shop' } }];
  const { batch, did } = delivery(bodies);
  await h.worker.queue(batch, h.env);
  assert.deepEqual(seen.body, { items: bodies });
  assert.equal(seen.auth, `Bearer ${TOKEN}`);
  assert.deepEqual(did, { ack: 1, retry: [] }, 'a refusal is Core\'s final word: acked, not retried');
});

test('consumer: anything but a 2xx retries the whole batch with backoff and acks nothing', async () => {
  for (const [status, attempts, delay] of [[500, 1, RETRY_DELAYS_S[0]], [503, 2, RETRY_DELAYS_S[1]], [401, 9, RETRY_DELAYS_S.at(-1)], [400, 3, RETRY_DELAYS_S[2]], [302, 1, RETRY_DELAYS_S[0]]]) {
    const h = harness({ coreBatch: () => new Response('no', { status }) });
    const { batch, did } = delivery([{ kind: 'events', request: events() }], attempts);
    await h.worker.queue(batch, h.env);
    assert.deepEqual(did, { ack: 0, retry: [{ delaySeconds: delay }] }, `status ${status} attempt ${attempts}`);
  }
  const h = harness({ coreBatch: () => { throw new Error('connection reset'); } });
  const { batch, did } = delivery([{ kind: 'events', request: events() }]);
  await h.worker.queue(batch, h.env);
  assert.deepEqual(did, { ack: 0, retry: [{ delaySeconds: RETRY_DELAYS_S[0] }] });
});

test('the retry schedule fits inside Core\'s 30 min orphan hold', () => {
  const retries = 8; // max_retries in wrangler.jsonc
  let total = 0;
  for (let a = 1; a <= retries; a++) total += RETRY_DELAYS_S[Math.min(a, RETRY_DELAYS_S.length) - 1];
  assert.ok(total < 1800, `${total} s`);
});
