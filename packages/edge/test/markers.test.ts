import { test } from 'node:test';
import assert from 'node:assert/strict';
import { rewriter, beforeBodyEnd } from '../src/markers.ts';

async function run(chunks: string[], decide: (slot: string) => Promise<{ html: string | null }>) {
  const asked: string[] = [];
  const out = await new Response(
    new ReadableStream<string>({
      start(c) {
        for (const ch of chunks) c.enqueue(ch);
        c.close();
      },
    })
      .pipeThrough(rewriter(async (s) => (asked.push(s), decide(s))))
      .pipeThrough(new TextEncoderStream()),
  ).text();
  return { out, asked };
}

const PAGE = '<nav>Shop</nav><!-- prb:slot="hero" --><h1>Espresso</h1><!-- /prb:slot --><footer>f</footer>';

test('marker_region_is_replaced', async () => {
  const { out } = await run([PAGE], async () => ({ html: '<h1>Gifts</h1>' }));
  assert.equal(out, '<nav>Shop</nav><!-- prb:slot="hero" --><h1>Gifts</h1><!-- /prb:slot --><footer>f</footer>');
});

// One test per chunk size, down to one character per chunk, so a failure names the size.
for (const size of [1, 2, 3, 5, 7, 13]) {
  test(`marker_split_across_chunks_of_${size}`, async () => {
    const chunks = PAGE.match(new RegExp(`[\\s\\S]{1,${size}}`, 'g'))!;
    const { out, asked } = await run(chunks, async () => ({ html: '<h1>Gifts</h1>' }));
    assert.equal(out, PAGE.replace('<h1>Espresso</h1>', '<h1>Gifts</h1>'));
    assert.deepEqual(asked, ['hero']);
  });
}

test('two_slots_are_both_replaced', async () => {
  const page = PAGE + '<!-- prb:slot="promo" --><p>Free shipping</p><!-- /prb:slot -->';
  const { out, asked } = await run([page], async (s) => ({ html: `<p>${s}!</p>` }));
  assert.deepEqual(asked, ['hero', 'promo']);
  assert.match(out, /<!-- prb:slot="hero" --><p>hero!<\/p>/);
  assert.match(out, /<!-- prb:slot="promo" --><p>promo!<\/p>/);
});

test('baseline_decision_keeps_the_region', async () => {
  const { out } = await run([PAGE], async () => ({ html: null }));
  assert.equal(out, PAGE);
});

test('failed_decision_keeps_the_region', async () => {
  const { out } = await run([PAGE], async () => {
    throw new Error('core down');
  });
  assert.equal(out, PAGE);
});

test('text_without_markers_streams_untouched', async () => {
  const html = '<p>a < b and 2<3</p>' + 'x'.repeat(500);
  const { out, asked } = await run([html.slice(0, 7), html.slice(7)], async () => ({ html: 'X' }));
  assert.equal(out, html);
  assert.equal(asked.length, 0);
});

test('unterminated_region_is_returned_untouched', async () => {
  const html = '<!-- prb:slot="hero" --><h1>Espresso</h1>';
  const { out } = await run([html], async () => ({ html: 'X' }));
  assert.equal(out, html);
});

test('tag_goes_before_body_end', async () => {
  const out = await new Response(
    new ReadableStream<string>({
      start(c) {
        c.enqueue('<html><body><p>x</p></bo');
        c.enqueue('dy></html>');
        c.close();
      },
    })
      .pipeThrough(beforeBodyEnd(() => '<script></script>'))
      .pipeThrough(new TextEncoderStream()),
  ).text();
  assert.equal(out, '<html><body><p>x</p><script></script></body></html>');
});

test('text_streams_before_the_page_ends', async () => {
  // The head and nav must reach the browser while the origin is still sending the rest.
  let ctl!: ReadableStreamDefaultController<string>;
  const reader = new ReadableStream<string>({ start: (c) => void (ctl = c) })
    .pipeThrough(rewriter(async () => ({ html: 'X' })))
    .getReader();
  ctl.enqueue('<head><title>t</title></head><nav>Shop</nav>' + 'x'.repeat(300));
  const first = await Promise.race([reader.read(), new Promise<'timeout'>((r) => setTimeout(() => r('timeout'), 300))]);
  assert.notEqual(first, 'timeout', 'nothing was sent until the page ended');
  assert.match((first as ReadableStreamReadResult<string>).value!, /^<head>/);
  ctl.close();
});
