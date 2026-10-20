import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, mkdir, readdir, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import { scan, writeDrafts, createSlotArgs, DRAFTS_PATH } from '../src/scan.mjs';

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

const FIXTURES = join(here, 'fixtures');
for (const [ex, root] of [['shop', EXAMPLES], ['next', EXAMPLES], ['plain', EXAMPLES],
  ['vue', FIXTURES], ['svelte', FIXTURES], ['astro', FIXTURES], ['jsx-lt', FIXTURES]]) {
  test(`${root === EXAMPLES ? 'examples' : 'fixtures'}/${ex} matches its golden scan`, async () => {
    const got = await scan({ dir: join(root, ex) });
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

test('examples/plain: site.json selectors mark the HTML hero and the image banner as existing slots, with their declared kinds', async () => {
  const r = await scan({ dir: join(EXAMPLES, 'plain') });
  assert.deepEqual(r.existing, [{ key: 'hero', kind: 'swap', file: 'index.html', line: 13 }, { key: 'banner', kind: 'copy', file: 'index.html', line: 18 }]);
  assert.deepEqual(r.candidates, []);
});

// ---- Vue, Svelte and Astro: markup only, each framework's loop, the script-tag marks ----

const brief = (r) => r.candidates.map((c) => [c.line, c.kind, c.key]);

test('fixtures/vue: the template is scanned, <script> and <style> are not, v-for is a sorted list', async () => {
  const r = await scan({ dir: join(FIXTURES, 'vue') });
  assert.deepEqual(r.existing, [{ key: 'comeback', kind: 'swap', file: 'src/pages/Home.vue', line: 21 }]);
  assert.deepEqual(brief(r), [[11, 'copy', 'hero'], [18, 'sorted', 'products'], [24, 'swap', 'returning'], [16, 'swap', 'promo'], [23, 'copy', 'book_a_call_cta']]);
  assert.ok(!r.candidates.some((c) => c.file === 'src/components/Welcome.vue'), 'a component rendered inside a slot is covered by its file name');
  assert.ok(!r.candidates.some((c) => c.line === 22), 'mustache words are dynamic');
  assert.ok(!r.candidates.some((c) => c.line < 9 || c.line > 25), 'nothing from <script> or <style>');
  const [hero, list] = r.candidates;
  assert.deepEqual(hero.fields.map((f) => f.baseline), ['Hand-poured candles', 'Soy wax, cotton wicks.', 'Shop candles']);
  assert.equal(list.goal, 'revenue');
  for (const c of r.candidates) {
    assert.match(c.mark, new RegExp(`data-prb-slot="${c.key}"`), c.key);
    assert.doesNotMatch(c.mark, /prb\.(Slot|Copy|Sorted)/, 'there is no Vue component to wrap with');
    assert.match(c.mark, /script tag/);
  }
  assert.match(hero.mark, /data-prb-field="<name>".*headline, subhead, cta_label/);
  assert.match(list.mark, /data-prb-item=/);
});

test('fixtures/svelte: {#each} is a sorted list, a prb:slot comment is existing, `<` in braces is not a tag', async () => {
  const r = await scan({ dir: join(FIXTURES, 'svelte') });
  assert.deepEqual(r.existing, [{ key: 'welcome', kind: 'swap', file: 'src/routes/+page.svelte', line: 25 }]);
  assert.deepEqual(brief(r), [[10, 'copy', 'hero'], [19, 'sorted', 'products'], [16, 'swap', 'promo'], [27, 'copy', 'get_started_cta']]);
  assert.ok(!r.candidates.some((c) => c.line === 28), 'a heading with dynamic words is not a copy slot');
  assert.match(r.candidates[1].why, /with prices/);
});

test('fixtures/astro: frontmatter and <script> are skipped, JSX inside expressions is scanned, site.json selectors count', async () => {
  const r = await scan({ dir: join(FIXTURES, 'astro') });
  assert.deepEqual(r.existing, [{ key: 'offer_box', kind: 'swap', file: 'src/pages/index.astro', line: 17 }]);
  assert.deepEqual(brief(r), [[8, 'copy', 'hero'], [14, 'sorted', 'products'], [18, 'sorted', 'list'], [13, 'swap', 'promo']]);
});

test('fixtures/jsx-lt: comparisons, generics, comments, strings and regexes do not make tags', async () => {
  const r = await scan({ dir: join(FIXTURES, 'jsx-lt') });
  assert.deepEqual(brief(r), [[14, 'copy', 'hero'], [17, 'sorted', 'products'], [15, 'swap', 'returning'], [16, 'swap', 'promo'], [20, 'copy', 'get_started_cta']]);
  assert.ok(!r.candidates.some((c) => c.line < 9), 'nothing from a comment, a template string or a regex');
});

test('a JSX list mapped without keys is not offered as sorted (no item ids to learn)', async () => {
  const dir = await fixture({ 'app/page.tsx': `export default function P({ xs }) {
  return <ul className="products">{xs.map((x) => <li>{x}</li>)}</ul>;
}
` });
  assert.deepEqual((await scan({ dir })).candidates, []);
});

test('plain HTML: markup inside <script> and <style> is not scanned', async () => {
  const dir = await fixture({ 'index.html': `<html><body>
<script>const s = '<button class="cta">Buy now</button>'; if (a<b) go();</script>
<style>.x::before { content: '<a class="cta">Shop now</a>'; }</style>
<a class="cta" href="/join">Join us</a>
</body></html>
` });
  const r = await scan({ dir });
  assert.deepEqual(r.candidates.map((c) => [c.line, c.key]), [[4, 'join_us_cta']]);
  assert.match(r.candidates[0].mark, /data-prb-field="<name>".*cta_label/, 'a copy mark names its fields');
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

test('a long hero headline gets a copy max the server accepts (500 at most)', async () => {
  const long = 'Hand-poured candles '.repeat(20).trim();
  const dir = await fixture({ 'app/page.tsx': PAGE.replace('<h1>Hand-poured candles</h1>', `<h1>${long}</h1>`) });
  const hero = (await scan({ dir })).candidates.find((c) => c.kind === 'copy' && c.fields.some((f) => f.name === 'headline'));
  assert.ok(long.length * 1.5 > 500);
  assert.equal(hero.fields.find((f) => f.name === 'headline').max, 500);
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

test('--write gives each draft the exact create_slot arguments for its kind', async () => {
  const dir = await fixture({ 'app/page.tsx': PAGE });
  await run(process.execPath, [CLI, 'scan', '--dir', dir, '--write']);
  const { drafts, note } = JSON.parse(await readFile(join(dir, DRAFTS_PATH), 'utf8'));
  assert.deepEqual(new Set(drafts.map((d) => d.kind)), new Set(['swap', 'copy', 'sorted']));
  for (const d of drafts) {
    const a = d.create_slot;
    assert.deepEqual([a.key, a.selector, a.metric, a.kind], [d.key, d.selector, d.metric, d.kind], d.key);
    if (d.kind === 'copy') {
      // The server's copy spec refuses unknown keys, so the baseline text must not travel.
      assert.deepEqual(a.copy, { fields: d.fields.map(({ name, max, required }) => ({ name, max, required })) });
      assert.ok(a.copy.fields.every((f) => f.max >= 1 && f.max <= 500));
    } else assert.equal(a.copy, undefined, d.key);
    if (d.kind === 'sorted') {
      assert.equal(a.items, undefined, 'scan cannot know the item keys; it must not invent them');
      assert.match(d.needs, /items/);
    } else {
      assert.equal(a.items, undefined, d.key);
      assert.equal(d.needs, undefined, d.key);
    }
  }
  assert.match(note, /every kind through create_slot/);
  assert.doesNotMatch(note, /site owner/);
});

test('--write refuses a drafts file it cannot parse and leaves it alone', async () => {
  const dir = await fixture({ 'app/page.tsx': PAGE, [DRAFTS_PATH]: '{oops' });
  await assert.rejects(run(process.execPath, [CLI, 'scan', '--dir', dir, '--write']));
  assert.equal(await readFile(join(dir, DRAFTS_PATH), 'utf8'), '{oops');
});

// ---- S11: images ----

const IMG_PAGE = `export default function Home() {
  return (
    <main>
      <section className="hero">
        <img src="/hero.jpg" alt="Candles on a shelf" />
        <h1>Hand-poured candles</h1>
        <a href="/shop" className="btn">Shop candles</a>
      </section>
      <div className="banner">
        <Image src="/sale.webp" alt="Autumn sale" width={1200} height={300} />
      </div>
      <picture>
        <source srcSet="/wide.avif" type="image/avif" />
        <img src="/wide.jpg" alt="The workshop" />
      </picture>
      <figure className="team"><img src="/team.jpg" alt="Our team" /><figcaption>Our team</figcaption></figure>
    </main>
  );
}
`;

test('a hero with an image gets an image field beside its words', async () => {
  const dir = await fixture({ 'app/page.tsx': IMG_PAGE });
  const hero = (await scan({ dir })).candidates.find((c) => c.key === 'hero');
  assert.equal(hero.kind, 'copy');
  assert.deepEqual(hero.fields.map((f) => [f.name, f.type ?? 'text']), [['headline', 'text'], ['cta_label', 'text'], ['hero_image', 'image']]);
  const img = hero.fields.find((f) => f.name === 'hero_image');
  assert.deepEqual(img.baseline, { src: '/hero.jpg', alt: 'Candles on a shelf' });
  assert.equal(img.required, true);
  assert.match(hero.why, /image/);
  assert.match(hero.mark, /prb\.Image/);
});

test('a banner that is only an image, and a picture, are image slots; a captioned figure is not', async () => {
  const dir = await fixture({ 'app/page.tsx': IMG_PAGE });
  const r = await scan({ dir });
  const banner = r.candidates.find((c) => c.line === 9);
  assert.equal(banner?.kind, 'copy', JSON.stringify(r.candidates.map((c) => [c.line, c.key])));
  assert.equal(banner.key, 'banner');
  assert.deepEqual(banner.fields, [{ name: 'image', type: 'image', required: true, baseline: { src: '/sale.webp', alt: 'Autumn sale' } }]);
  assert.match(banner.why, /without writing HTML/);
  const pic = r.candidates.find((c) => c.line === 12);
  assert.equal(pic?.kind, 'copy');
  assert.equal(pic.fields[0].baseline.src, '/wide.jpg');
  assert.ok(!r.candidates.some((c) => c.line === 16 && c.fields?.some((f) => f.type === 'image')), 'a figure with a caption is not an image-only banner');
  // Nothing inside the hero is offered again.
  assert.ok(!r.candidates.some((c) => c.line === 5));
});

test('an image slot in plain HTML says to mark the img', async () => {
  const dir = await fixture({ 'index.html': '<html><body><div class="promo-art"><img src="/a.png" alt="A"></div></body></html>' });
  const [c] = (await scan({ dir })).candidates;
  assert.equal(c.kind, 'copy');
  assert.match(c.mark, /data-prb-field="image"/);
});

test('a dynamic image source still makes an image field, with no baseline src', async () => {
  const dir = await fixture({ 'app/page.tsx': IMG_PAGE.replace('src="/hero.jpg"', 'src={heroUrl}') });
  const hero = (await scan({ dir })).candidates.find((c) => c.key === 'hero');
  assert.deepEqual(hero.fields.find((f) => f.name === 'hero_image').baseline, { src: null, alt: 'Candles on a shelf' });
});

test('create_slot arguments give an image field its type and no max', async () => {
  const dir = await fixture({ 'app/page.tsx': IMG_PAGE });
  const hero = (await scan({ dir })).candidates.find((c) => c.key === 'hero');
  const args = createSlotArgs(hero);
  assert.deepEqual(args.copy.fields.find((f) => f.name === 'hero_image'), { name: 'hero_image', type: 'image', required: true });
  assert.ok(args.copy.fields.find((f) => f.name === 'headline').max > 0);
});

test('a banner with words beside its image stays a swap slot (layout and words change together)', async () => {
  const dir = await fixture({ 'index.html': '<html><body><div class="banner"><img src="/a.png" alt="A"><p>Autumn sale ends Sunday</p></div></body></html>' });
  const [c] = (await scan({ dir })).candidates;
  assert.equal(c.kind, 'swap');
  assert.equal(c.fields, undefined);
});
