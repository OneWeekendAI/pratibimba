// G3: the marker rewriter serves a sorted slot's decided order, as the client and next SDKs do:
// the `data-prb-item` children move into the order, gated (unplaced) items follow in page order.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';
import { rewriter, reorderItems } from '../src/markers.ts';
import { BEACON_JS } from '../src/beacon.ts';
import { createEdge } from '../src/index.ts';

const card = (k: string) => `<div data-prb-item="${k}"><img src="/${k}.jpg"><a href="/p/${k}">${k.toUpperCase()}</a></div>`;
const GRID = ['a', 'b', 'c'].map(card).join('\n  ');
const keysIn = (html: string) => [...html.matchAll(/data-prb-item="([^"]+)"/g)].map((m) => m[1]);

test('items_take_the_decided_order', () => {
  const r = reorderItems(GRID, ['c', 'a', 'b']);
  assert.equal(r.html, [card('c'), card('a'), card('b')].join('\n  '), 'whitespace between cards stays put');
  assert.deepEqual(r.keys, ['c', 'a', 'b']);
});

test('items_the_order_leaves_out_follow_in_page_order', () => {
  const r = reorderItems(GRID, ['c', 'zz']);
  assert.deepEqual(keysIn(r.html), ['c', 'a', 'b']);
  assert.deepEqual(r.keys, ['c'], 'only the ordered items are reported as rendered in order');
});

test('items_inside_a_wrapper_are_reordered_and_their_own_markup_kept', () => {
  const region = `<ul class="grid" data-x='a>b'>\n<li data-prb-item=a><p>A<br>a</p></li><li data-prb-item='b' class="x"><div data-prb-item="inner">B</div></li>\n</ul>`;
  const r = reorderItems(region, ['b', 'a']);
  assert.equal(r.html, `<ul class="grid" data-x='a>b'>\n<li data-prb-item='b' class="x"><div data-prb-item="inner">B</div></li><li data-prb-item=a><p>A<br>a</p></li>\n</ul>`);
  assert.deepEqual(r.keys, ['b', 'a']);
});

test('a_script_or_comment_holding_markup_does_not_confuse_the_scan', () => {
  const region = `<div data-prb-item="a">A<script>if (x < 1) document.write("</div>")</script></div><!-- <div> --><div data-prb-item="b">B</div>`;
  assert.deepEqual(keysIn(reorderItems(region, ['b', 'a']).html), ['b', 'a']);
});

test('markup_it_cannot_balance_is_served_in_page_order', () => {
  // An implied end tag (</li> left out) is legal HTML; the rewriter does not guess and leaves it.
  const region = '<ul><li data-prb-item="a">A<li data-prb-item="b">B</ul>';
  assert.deepEqual(reorderItems(region, ['b', 'a']), { html: region, keys: [] });
  // A card left open at the region's end.
  const open = '<div data-prb-item="a">A</div><div data-prb-item="b">B';
  assert.deepEqual(reorderItems(open, ['b', 'a']), { html: open, keys: [] });
});

test('misnested_tags_are_served_in_page_order', () => {
  const region = '<div data-prb-item="a"><b>A</div></b><div data-prb-item="b">B</div>';
  assert.deepEqual(reorderItems(region, ['b', 'a']), { html: region, keys: [] });
});

test('a_duplicate_item_key_places_its_first_card_and_the_second_follows', () => {
  const region = [card('a'), card('b'), card('a')].join('');
  assert.deepEqual(keysIn(reorderItems(region, ['a', 'b']).html), ['a', 'b', 'a']);
});

async function run(chunks: string[], order: string[] | undefined, seen: string[][] = []) {
  const stream = new ReadableStream<string>({
    start(c) {
      for (const ch of chunks) c.enqueue(ch);
      c.close();
    },
  });
  return new Response(
    stream.pipeThrough(rewriter(async () => ({ html: null, order, rendered: (k) => seen.push(k) }))).pipeThrough(new TextEncoderStream()),
  ).text();
}

const PAGE = `<h1>Shop</h1><!-- prb:slot="grid" -->\n  ${GRID}\n<!-- /prb:slot --><footer>f</footer>`;

for (const size of [1, 3, 7]) {
  test(`a_sorted_region_split_into_chunks_of_${size}_is_reordered`, async () => {
    const seen: string[][] = [];
    const out = await run(PAGE.match(new RegExp(`[\\s\\S]{1,${size}}`, 'g'))!, ['b', 'c', 'a'], seen);
    assert.equal(out, PAGE.replace(GRID, [card('b'), card('c'), card('a')].join('\n  ')));
    assert.deepEqual(seen, [['b', 'c', 'a']]);
  });
}

test('a_region_with_no_order_keeps_the_page_order', async () => {
  assert.equal(await run([PAGE], undefined), PAGE);
});

const sortedDecision = () =>
  Response.json({ decision_id: 'd-g', slot: 'grid', variant: 'c', payload: null, order: ['c', 'a', 'b'], explanation: {} });

test('the_edge_serves_the_order_and_the_beacon_carries_the_rendered_items', async () => {
  const body = `<html><body>${PAGE}</body></html>`;
  const origin = async () => new Response(body, { headers: { 'content-type': 'text/html' } });
  const fetch = (async () => sortedDecision()) as unknown as typeof globalThis.fetch;
  const res = await createEdge({ core: 'http://core:8080', site: 'shop', origin, fetch }).handleRequest(new Request('https://shop.test/'));
  const text = await res.text();
  assert.deepEqual(keysIn(text), ['c', 'a', 'b']);
  const cfg = JSON.parse(text.match(/data-prb='([^']*)'/)![1].replace(/&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&'));
  assert.deepEqual(cfg.decisions, [{ slot: 'grid', id: 'd-g', items: ['c', 'a', 'b'] }]);
});

/** The page as the edge serves it, with the beacon; returns what it posts. */
function load(items: string[] | undefined) {
  const posted: any[] = [];
  const cfg = JSON.stringify({ site: 'shop', subject: 'v1', decisions: [{ slot: 'grid', id: 'd-g', ...(items ? { items } : {}) }] });
  const dom = new JSDOM(`<!doctype html><html><body><!-- prb:slot="grid" -->${[card('c'), card('a'), card('b')].join('')}<!-- /prb:slot --><script data-prb='${cfg}'>${BEACON_JS}</script></body></html>`, {
    runScripts: 'dangerously',
    url: 'https://shop.test/',
    beforeParse(w: any) {
      w.fetch = (_: string, init: any) => (posted.push(JSON.parse(init.body).events[0]), Promise.resolve());
      w.crypto.randomUUID ??= () => 'id';
    },
  });
  return { dom, posted };
}

test('the_beacon_sends_the_rendered_order_and_names_the_clicked_card', async () => {
  const { dom, posted } = load(['c', 'a', 'b']);
  await new Promise((r) => setTimeout(r, 20));
  dom.window.document.querySelector<HTMLAnchorElement>('a[href="/p/a"]')!.click();
  assert.deepEqual(posted.map((e) => [e.name, e.items]), [
    ['exposure', [{ id: 'c' }, { id: 'a' }, { id: 'b' }]],
    ['click', [{ id: 'a' }]],
  ]);
});

test('a_swap_slot_beacon_sends_no_items', async () => {
  const { dom, posted } = load(undefined);
  await new Promise((r) => setTimeout(r, 20));
  dom.window.document.querySelector<HTMLAnchorElement>('a[href="/p/a"]')!.click();
  assert.deepEqual(posted.map((e) => [e.name, e.items]), [['exposure', undefined], ['click', undefined]]);
});
