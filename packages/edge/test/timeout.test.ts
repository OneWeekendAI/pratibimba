// A page whose slot gave up on Core must not throw away the connection it was using.
//
// Aborting an in-flight fetch destroys its keep-alive socket, so the next page pays a fresh
// TCP+TLS handshake, misses its deadline too, and the pool never warms (measured live with
// @pratibimba/next: 15 timed-out renders opened 22 connections). A real HTTP server and the
// runtime's own fetch, because only a real socket can show whether it was kept.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer, type Server } from 'node:http';
import type { AddressInfo, Socket } from 'node:net';
import { createEdge } from '../src/index.ts';

const PAGE = '<html><body><!-- prb:slot="hero" --><h1>Espresso</h1><!-- /prb:slot --></body></html>';
const ANSWER = JSON.stringify({ decision_id: 'd-1', slot: 'hero', variant: 'gift', payload: '<h1>Gifts</h1>', explanation: {} });

/** Core that answers after `delayMs` (never, for `null`), counting the connections it accepts. */
async function slowCore(delayMs: number | null) {
  const sockets: Socket[] = [];
  const server: Server = createServer((req, res) => {
    req.resume();
    req.on('end', () => {
      if (delayMs === null) return;
      setTimeout(() => res.writeHead(200, { 'content-type': 'application/json' }).end(ANSWER), delayMs);
    });
  });
  server.on('connection', (s) => sockets.push(s));
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  return {
    url: `http://127.0.0.1:${(server.address() as AddressInfo).port}`,
    sockets,
    close: () => {
      for (const s of sockets) s.destroy();
      return new Promise<void>((r) => server.close(() => r()));
    },
  };
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const origin = async () => new Response(PAGE, { headers: { 'content-type': 'text/html; charset=utf-8' } });
const page = (core: string, extra = {}) =>
  createEdge({ core, site: 'shop', origin, ...extra }).handleRequest(new Request('https://shop.test/')).then((r) => r.text());

test('edge_slot_that_timed_out_leaves_its_connection_in_the_pool', async () => {
  const core = await slowCore(120);
  try {
    for (let i = 0; i < 4; i++) {
      assert.match(await page(core.url, { timeoutMs: 30 }), /Espresso/);
      await sleep(250); // the late answer lands, and the socket goes back to the pool
    }
    assert.equal(core.sockets.length, 1, `4 timed-out pages opened ${core.sockets.length} connections`);
  } finally {
    await core.close();
  }
});

test('edge_slot_keeps_its_baseline_at_the_deadline_not_when_core_answers', async () => {
  const core = await slowCore(400);
  try {
    const t0 = performance.now();
    assert.match(await page(core.url, { timeoutMs: 40 }), /Espresso/);
    const took = performance.now() - t0;
    assert.ok(took < 200, `the page waited ${Math.round(took)} ms for a 40 ms deadline`);
  } finally {
    await core.close();
  }
});

test('edge_decide_core_never_answers_is_cut_off_at_the_hard_limit', async () => {
  const core = await slowCore(null);
  try {
    assert.match(await page(core.url, { timeoutMs: 20, hardTimeoutMs: 150 }), /Espresso/);
    await sleep(400);
    assert.ok(core.sockets.length >= 1);
    assert.ok(core.sockets.every((s) => s.destroyed), 'a hung request kept its socket open past the hard limit');
  } finally {
    await core.close();
  }
});

test('edge_slot_inside_its_deadline_still_gets_the_variant', async () => {
  const core = await slowCore(10);
  try {
    assert.match(await page(core.url, { timeoutMs: 500 }), /Gifts/);
  } finally {
    await core.close();
  }
});
