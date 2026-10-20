// S10: one sorted region, three renderers. The browser SDK (packages/client/sdk.js), the edge
// marker rewriter (packages/edge/src/markers.ts) and <prb.Sorted> (src/slot.ts) must lay out the
// same children for the same decided order: the ordered cards fill the cards' places, a gated card
// follows them in page order, and everything that is not a card (text between cards, a heading,
// an ad tile) keeps its place. Each renderer's output is read back as one list of children and
// the three lists are compared.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';
import { createElement, Fragment, type ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { arrange } from '../src/slot.ts';
import { reorderItems } from '../../edge/src/markers.ts';
import { load, ok, sleep } from '../../client/test/harness.mjs';

// Text before, between and after the cards; a heading and an ad tile among them; `c` is gated
// out (not in the order); the order names a key the page lacks and repeats one.
const REGION =
  'Lead text <h2>Our picks</h2>\n  <article data-prb-item="a">Card A</article> between a and b ' +
  '<article data-prb-item="b">Card B</article><aside class="ad">Ad tile</aside>' +
  '<article data-prb-item="c">Card C</article>\n  after c <article data-prb-item="d">Card D</article> tail';
const ORDER = ['d', 'zz', 'b', 'd', 'a'];
const EXPECTED = [
  'text:Lead text ', 'el:<h2>Our picks</h2>', 'text:\n  ', 'item:d', 'text: between a and b ',
  'item:b', 'el:<aside class="ad">Ad tile</aside>', 'item:a', 'text:\n  after c ', 'item:c', 'text: tail',
];

const { document: doc, Node } = new JSDOM('').window;

/** A node list as one token per child: text verbatim, a card by its key, any other element whole. */
function tokens(nodes: ArrayLike<any>): string[] {
  const out: string[] = [];
  for (const n of Array.from(nodes)) {
    if (n.nodeType === Node.TEXT_NODE || n.nodeType === 3) {
      // A renderer may split one run of text into adjacent nodes; the page sees one.
      if (out.length && out[out.length - 1].startsWith('text:')) out[out.length - 1] += n.data;
      else out.push('text:' + n.data);
    } else if (n.nodeType === 1 && n.hasAttribute('data-prb-item')) out.push('item:' + n.getAttribute('data-prb-item'));
    else if (n.nodeType === 1) out.push('el:' + n.outerHTML);
  }
  return out;
}
const parse = (html: string) => {
  const t = doc.createElement('template');
  t.innerHTML = html;
  return t.content.childNodes;
};

function edge(): string[] {
  return tokens(parse(reorderItems(REGION, ORDER).html));
}

async function client(): Promise<string[]> {
  const manifest = {
    site: 'demo',
    slots: [{ key: 'grid', selector: '#grid', metric: 'click', ttl_secs: 60, kind: 'sorted', sorted: { visible: 2 },
              variants: ['a', 'b', 'c', 'd'].map((k) => ({ key: k, baseline: k === 'a' })) }],
  };
  const decision = { decision_id: 'd-1', slot: 'grid', variant: 'd', payload: null, order: ORDER, explanation: {} };
  const p = load({ manifest, body: `<div id="grid">${REGION}</div>`, decide: () => ok(decision) });
  await sleep(20);
  const grid = p.w.document.getElementById('grid')!;
  assert.equal(grid.getAttribute('data-prb-done'), '', 'the client applied the decision');
  return tokens(grid.childNodes);
}

function next(): string[] {
  // The same region as React children: a card is a keyed child, everything else unkeyed.
  const children: ReactNode[] = Array.from(parse(REGION)).map((n: any) =>
    n.nodeType === 3
      ? n.data
      : n.hasAttribute('data-prb-item')
        ? createElement(n.localName, { key: n.getAttribute('data-prb-item') }, n.textContent)
        : createElement(n.localName, n.className ? { className: n.className } : null, n.textContent),
  );
  const { nodes } = arrange(createElement(Fragment, null, ...children).props.children, ORDER);
  // <prb.Sorted> wraps each card in a display:contents element carrying data-prb-item: that
  // wrapper is the card's place in the output.
  return tokens(parse(renderToStaticMarkup(createElement(Fragment, null, ...nodes))));
}

test('the fixture region reads back in page order', () => {
  // The tokens see the cards: the expected layout is the page's with the cards renamed.
  const page = ['a', 'b', 'c', 'd'];
  let n = 0;
  assert.deepEqual(tokens(parse(REGION)), EXPECTED.map((t) => (t.startsWith('item:') ? 'item:' + page[n++] : t)));
});

test('edge_rewriter_lays_out_the_sorted_region_as_expected', () => {
  assert.deepEqual(edge(), EXPECTED);
});

test('client_sdk_lays_out_the_sorted_region_as_expected', async () => {
  assert.deepEqual(await client(), EXPECTED);
});

test('next_sorted_lays_out_the_sorted_region_as_expected', () => {
  assert.deepEqual(next(), EXPECTED);
});

test('all_three_renderers_agree_on_the_sorted_region', async () => {
  const e = edge();
  assert.deepEqual(await client(), e, 'client vs edge');
  assert.deepEqual(next(), e, 'next vs edge');
});
