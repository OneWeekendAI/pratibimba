import { test } from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';
import { BEACON_JS } from '../src/beacon.ts';

/** A page as the edge sends it, with the beacon tag; returns the events it posts. */
function load(decisions: { slot: string; id: string }[]) {
  const posted: any[] = [];
  const cfg = JSON.stringify({ site: 'shop', subject: 'v1', decisions }).replace(/'/g, '&#39;');
  const dom = new JSDOM(
    `<!doctype html><html><body><nav><a id="nav">Nav</a></nav><!-- prb:slot="hero" --><h1><a id="in">Gifts</a></h1><!-- /prb:slot --><p><a id="out">After</a></p>
     <script data-prb='${cfg}'>${BEACON_JS}</script></body></html>`,
    {
      runScripts: 'dangerously',
      url: 'https://shop.test/',
      beforeParse(w: any) {
        w.fetch = (url: string, init: any) => {
          posted.push({ url, ...JSON.parse(init.body) });
          return Promise.resolve();
        };
        w.crypto.randomUUID ??= () => 'id';
      },
    },
  );
  return { dom, posted };
}

const names = (posted: any[]) => posted.map((p) => p.events[0].name);

test('beacon_reports_exposure_for_each_decision', async () => {
  const { posted } = load([{ slot: 'hero', id: 'd-1' }]);
  await new Promise((r) => setTimeout(r, 20));
  assert.deepEqual(names(posted), ['exposure']);
  assert.equal(posted[0].events[0].decision_id, 'd-1');
  assert.equal(posted[0].url, '/_prb/e');
});

test('beacon_reports_a_click_between_the_markers', async () => {
  const { dom, posted } = load([{ slot: 'hero', id: 'd-1' }]);
  await new Promise((r) => setTimeout(r, 20));
  dom.window.document.getElementById('in')!.click();
  assert.deepEqual(names(posted), ['exposure', 'click']);
});

test('beacon_ignores_clicks_outside_the_slot', async () => {
  const { dom, posted } = load([{ slot: 'hero', id: 'd-1' }]);
  await new Promise((r) => setTimeout(r, 20));
  dom.window.document.getElementById('out')!.click();
  dom.window.document.getElementById('nav')!.click();
  assert.deepEqual(names(posted), ['exposure']);
});

test('beacon_skips_a_slot_missing_from_the_page', async () => {
  const { posted } = load([{ slot: 'promo', id: 'd-9' }]);
  await new Promise((r) => setTimeout(r, 20));
  assert.deepEqual(posted, []);
});
