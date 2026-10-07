// A render that gives up on Core must not throw away the connection it was using.
//
// Aborting an in-flight fetch destroys its keep-alive socket, so the next render pays a fresh
// TCP+TLS handshake, misses its deadline too, and the pool never warms (measured live: 15
// timed-out renders opened 22 connections). These tests use a real HTTP server and Node's own
// fetch, because only a real socket can show whether it was kept.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer, type Server } from 'node:http';
import type { AddressInfo, Socket } from 'node:net';
import { decideBatch } from '../src/core.ts';

const ANSWER = JSON.stringify({ decisions: [{ decision_id: 'd-1', slot: 'hero', variant: 'gift', payload: null, explanation: {} }] });
const CTX = { returning: false, mobile: false, from_ad: false };

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
  const url = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  return {
    url,
    sockets,
    close: () => {
      for (const s of sockets) s.destroy();
      return new Promise<void>((r) => server.close(() => r()));
    },
  };
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

test('a_timed_out_decide_leaves_its_connection_in_the_pool', async () => {
  const core = await slowCore(120);
  try {
    for (let i = 0; i < 4; i++) {
      await assert.rejects(decideBatch({ core: core.url, site: 'shop', timeoutMs: 30 }, `v${i}`, CTX, ['hero']));
      await sleep(250); // the late answer lands, and the socket goes back to the pool
    }
    assert.equal(core.sockets.length, 1, `4 timed-out decides opened ${core.sockets.length} connections`);
  } finally {
    await core.close();
  }
});

test('a_timed_out_decide_returns_at_its_deadline_not_when_core_answers', async () => {
  const core = await slowCore(400);
  try {
    const t0 = performance.now();
    await assert.rejects(decideBatch({ core: core.url, site: 'shop', timeoutMs: 40 }, 'v', CTX, ['hero']), /timed out after 40 ms/);
    const took = performance.now() - t0;
    assert.ok(took < 200, `the render waited ${Math.round(took)} ms for a 40 ms deadline`);
  } finally {
    await core.close();
  }
});

test('a_decide_core_never_answers_is_cut_off_at_the_hard_limit', async () => {
  const core = await slowCore(null);
  try {
    await assert.rejects(decideBatch({ core: core.url, site: 'shop', timeoutMs: 20, hardTimeoutMs: 150 }, 'v', CTX, ['hero']));
    await sleep(400);
    assert.ok(core.sockets.length >= 1);
    assert.ok(core.sockets.every((s) => s.destroyed), 'a hung request kept its socket open past the hard limit');
  } finally {
    await core.close();
  }
});

test('a_decide_inside_its_deadline_still_returns_the_decisions', async () => {
  const core = await slowCore(10);
  try {
    const ds = await decideBatch({ core: core.url, site: 'shop', timeoutMs: 500 }, 'v', CTX, ['hero']);
    assert.equal((ds[0] as { variant: string }).variant, 'gift');
  } finally {
    await core.close();
  }
});

