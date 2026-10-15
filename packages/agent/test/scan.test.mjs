import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, mkdir, readdir, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import { scan, writeDrafts, DRAFTS_PATH } from '../src/scan.mjs';

const run = promisify(execFile);
const here = dirname(fileURLToPath(import.meta.url));
const CLI = join(here, '..', 'bin', 'cli.mjs');
const EXAMPLES = process.env.PRB_EXAMPLES ?? join(here, '..', '..', '..', 'examples');
const GOLDEN = join(here, 'golden');
const UPDATE = process.env.UPDATE_GOLDEN === '1';

async function fixture(files) {
  const dir = await mkdtemp(join(tmpdir(), 'prb-scan-'));
  for (const [p, text] of Object.entries(files)) {
    await mkdir(dirname(join(dir, p)), { recursive: true });
    await writeFile(join(dir, p), text);
  }
  return dir;
}
const at = (r, file, line) => r.candidates.find((c) => c.file === file && c.line === line);

// ---- golden outputs on the real examples ----

for (const ex of ['shop', 'next', 'plain']) {
  test(`examples/${ex} matches its golden scan`, async () => {
    const got = await scan({ dir: join(EXAMPLES, ex) });
    const path = join(GOLDEN, `${ex}.json`);
    if (UPDATE) await writeFile(path, `${JSON.stringify(got, null, 2)}\n`);
    assert.deepEqual(got, JSON.parse(await readFile(path, 'utf8')));
  });
}

// Hand-written claims about the examples, so the goldens are not the only judge.
test('examples/shop: every existing slot is found, and nothing inside one is offered', async () => {
  const r = await scan({ dir: join(EXAMPLES, 'shop') });
  assert.deepEqual(r.existing.map((e) => `${e.key}:${e.kind}`).sort(),
    ['comeback:swap', 'grid:sorted', 'hero:swap', 'hero_copy:copy', 'promo:swap']);
  assert.ok(!r.candidates.some((c) => c.file === 'app/page.tsx'), 'the home page is fully slotted');
  assert.ok(!r.candidates.some((c) => c.file.startsWith('app/components/')), 'the hero variants render inside a slot');
  const buy = at(r, 'app/p/[slug]/page.tsx', 33);
  assert.equal(buy?.kind, 'copy');
  assert.equal(buy.fields[0].baseline, 'Buy now');
  assert.ok(!r.candidates.some((c) => /Added to your cart/.test(c.excerpt)), 'a confirmation note is not a returning-visitor spot');
});

test('examples/next: the hero component is covered because it renders inside <prb.Slot>', async () => {
  const r = await scan({ dir: join(EXAMPLES, 'next') });
  assert.deepEqual(r.existing.map((e) => e.key), ['hero', 'promo']);
  assert.deepEqual(r.candidates, [], 'nav, the slotted hero and the slotted promo leave nothing');
});

test('examples/plain: a site.json selector marks the HTML hero as an existing slot', async () => {
  const r = await scan({ dir: join(EXAMPLES, 'plain') });
  assert.deepEqual(r.existing, [{ key: 'hero', kind: 'swap', file: 'index.html', line: 13 }]);
  assert.deepEqual(r.candidates, []);
});

// ---- each kind on an unslotted repo ----

const PAGE = `import { products } from '@/lib/products';
export default function Home() {
  return (
    <>
      <nav>
        <a className="cta" href="/sign-up">Sign up</a>
        <ul>{links.map((l) => <li key={l}>{l}</li>)}</ul>
      </nav>
      <section className="hero">
        <h1>Hand-poured candles</h1>
        <p>Soy wax, cotton wicks.</p>
        <a className="cta" href="#shop">Shop candles</a>
      </section>
      <p className="promo">Free shipping over ₹999 this week.</p>
      <div className="products">
        {products.map((p) => (
          <a key={p.slug} href={'/p/' + p.slug}><h2>{p.name}</h2><span>₹{p.price}</span></a>
        ))}
      </div>
      <aside>Welcome back! Pick up where you left off.</aside>
      <button onClick={() => go()}>Get started</button>
      <a className="cta" href={url}>{label}</a>
    </>
  );
}
`;
const HTML = `<!doctype html>
<html><body>
  <header id="top"><h1>Candle Co.</h1><a href="#shop">Shop now</a></header>
  <div class="shelf">
    <div class="card">A</div>
    <div class="card">B</div>
    <div class="card">C</div>
  </div>
  <!-- prb:slot="note" -->
  <p class="banner">20% off today only</p>
</body></html>
`;

test('a React page: hero, grid, promo, returning and CTA are each found, ranked, with file:line', async () => {
  const dir = await fixture({ 'app/page.tsx': PAGE });
  const r = await scan({ dir });
  const kinds = r.candidates.map((c) => [c.line, c.kind, c.key]);
  assert.deepEqual(kinds, [
    [9, 'copy', 'hero'],
    [15, 'sorted', 'products'],
    [20, 'swap', 'returning'],
    [14, 'swap', 'promo'],
    [21, 'copy', 'get_started_cta'],
  ]);
  const hero = r.candidates[0];
  assert.deepEqual(hero.fields.map((f) => [f.name, f.baseline]),
    [['headline', 'Hand-poured candles'], ['subhead', 'Soy wax, cotton wicks.'], ['cta_label', 'Shop candles']]);
  assert.equal(r.candidates[1].goal, 'revenue', 'a priced list is judged on revenue');
  assert.match(r.candidates[2].audience, /returning/);
  assert.deepEqual(r.candidates.map((c) => c.rank), [1, 2, 3, 4, 5]);
  assert.ok(!r.candidates.some((c) => c.line === 6 || c.line === 7), 'nothing inside <nav> is offered');
  assert.ok(!r.candidates.some((c) => c.line === 22), 'a link with dynamic words is not a copy slot');
});

test('a hero with dynamic words is a swap slot, not a copy slot', async () => {
  const dir = await fixture({ 'app/page.tsx': PAGE.replace('<h1>Hand-poured candles</h1>', '<h1>{title}</h1>') });
  const r = await scan({ dir });
  assert.equal(r.candidates[0].kind, 'swap');
  assert.equal(r.candidates[0].fields, undefined);
});

test('plain HTML: a section with h1 + action is a hero, repeated cards are sorted, a prb:slot comment is existing', async () => {
  const dir = await fixture({ 'index.html': HTML });
  const r = await scan({ dir });
  assert.deepEqual(r.existing, [{ key: 'note', kind: 'swap', file: 'index.html', line: 10 }]);
  assert.deepEqual(r.candidates.map((c) => [c.line, c.kind, c.selector]), [
    [3, 'copy', '#top'],
    [4, 'sorted', '[data-prb-slot=list]'],
  ]);
  assert.match(r.candidates[1].mark, /data-prb-slot="list"/);
});

test('an offer inside a wrapper is offered as the line itself, not the wrapper', async () => {
  const dir = await fixture({ 'app/about/page.tsx': `export default function About() {
  return (
    <section>
      <h2>Our story</h2>
      <p>Free shipping on every order.</p>
    </section>
  );
}
` });
  const r = await scan({ dir });
  assert.deepEqual(r.candidates.map((c) => [c.line, c.key, c.excerpt]), [[5, 'promo', '<p>Free shipping on every order.</p>']]);
});

test('a candidate key never collides with an existing slot key', async () => {
  const dir = await fixture({ 'app/page.tsx': PAGE, 'site.json': JSON.stringify({ slots: [{ key: 'hero', selector: '#nope' }] }) });
  const r = await scan({ dir });
  assert.equal(r.candidates[0].key, 'hero-2');
});

test('build output, node_modules and tests are not scanned', async () => {
  const dir = await fixture({
    'node_modules/x/index.html': HTML, '.next/server/page.html': HTML, 'e2e/a.tsx': PAGE, 'app/page.test.tsx': PAGE,
  });
  assert.deepEqual((await scan({ dir })).candidates, []);
});

// ---- writing: nothing without --write; drafts only with it ----

async function snapshot(dir) {
  const out = {};
  const go = async (d) => {
    for (const e of await readdir(d, { withFileTypes: true })) {
      const p = join(d, e.name);
      if (e.isDirectory()) await go(p);
      else out[p] = [(await stat(p)).mtimeMs, await readFile(p, 'utf8')];
    }
  };
  await go(dir);
  return out;
}

test('the CLI scan without --write changes no file', async () => {
  const dir = await fixture({ 'app/page.tsx': PAGE });
  const before = await snapshot(dir);
  const { stdout } = await run(process.execPath, [CLI, 'scan', '--dir', dir]);
  assert.match(stdout, /app\/page\.tsx:9 {2}copy {2}key hero/);
  assert.deepEqual(await snapshot(dir), before);
});

test('the CLI --json prints the scan as JSON', async () => {
  const dir = await fixture({ 'app/page.tsx': PAGE });
  const { stdout } = await run(process.execPath, [CLI, 'scan', '--dir', dir, '--json']);
  assert.deepEqual(JSON.parse(stdout), await scan({ dir }));
});

test('--write adds drafts to .pratibimba/slot-drafts.json, never to site.json, and keeps edited entries', async () => {
  const site = JSON.stringify({ slots: [] });
  const dir = await fixture({ 'app/page.tsx': PAGE, 'site.json': site });
  await run(process.execPath, [CLI, 'scan', '--dir', dir, '--write']);
  const doc = JSON.parse(await readFile(join(dir, DRAFTS_PATH), 'utf8'));
  assert.equal(doc.drafts.length, 5);
  assert.ok(doc.drafts.every((d) => d.status === 'draft'));
  assert.match(doc.note, /None of these is live/);
  assert.equal(await readFile(join(dir, 'site.json'), 'utf8'), site);

  doc.drafts[0].audience = 'edited by a person';
  await writeFile(join(dir, DRAFTS_PATH), JSON.stringify(doc));
  const res = await writeDrafts({ dir, candidates: (await scan({ dir })).candidates });
  assert.deepEqual([res.added, res.kept], [0, 5]);
  const again = JSON.parse(await readFile(join(dir, DRAFTS_PATH), 'utf8'));
  assert.equal(again.drafts[0].audience, 'edited by a person');
  assert.equal(again.drafts.length, 5);
});

test('--write refuses a drafts file it cannot parse and leaves it alone', async () => {
  const dir = await fixture({ 'app/page.tsx': PAGE, [DRAFTS_PATH]: '{oops' });
  await assert.rejects(run(process.execPath, [CLI, 'scan', '--dir', dir, '--write']));
  assert.equal(await readFile(join(dir, DRAFTS_PATH), 'utf8'), '{oops');
});
