// The real check: install the pack into a copy of examples/next, then connect the official MCP
// SDK client using only what the generated configs say, list the slots and propose a draft
// against a locally running Core.
//
//   PRB_BIN=../../target/debug/pratibimba DATABASE_URL=postgres://…/scratch_db node test/e2e.mjs
//
// DATABASE_URL must be a database this script may write to (it applies examples/next/site.json).

import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';
import assert from 'node:assert/strict';
import { cp, mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:net';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
const BIN = resolve(process.env.PRB_BIN ?? join(root, 'target', 'debug', 'pratibimba'));
const DB = process.env.DATABASE_URL ?? process.env.PRB_DB;
if (!DB) throw new Error('set DATABASE_URL to a scratch database');
const CLI = join(root, 'packages', 'agent', 'bin', 'cli.mjs');
const TOKEN_ENV = 'PRATIBIMBA_MCP_TOKEN';
const env = { ...process.env, PRB_DB: DB, RUST_LOG: 'warn' };
const step = (s) => console.log(`- ${s}`);

const port = await new Promise((ok) => {
  const s = createServer().listen(0, '127.0.0.1', () => {
    const p = s.address().port;
    s.close(() => ok(p));
  });
});

// 1. A temp Next.js app: examples/next without its build output and dependencies.
const app = await mkdtemp(join(tmpdir(), 'prb-next-'));
await cp(join(root, 'examples', 'next'), app, {
  recursive: true,
  filter: (src) => !/\/(node_modules|\.next|test-results)(\/|$)/.test(src.slice(root.length)),
});
step(`copied examples/next to ${app}`);

// 2. Install the pack the way a developer would.
const out = execFileSync('node', [CLI, 'install', '--url', `http://127.0.0.1:${port}`], { cwd: app, encoding: 'utf8' });
assert.match(out, /created\s+\.mcp\.json/);
assert.match(out, /created\s+\.claude\/skills\/pratibimba\/SKILL\.md/);
const again = execFileSync('node', [CLI, 'install', '--url', `http://127.0.0.1:${port}`], { cwd: app, encoding: 'utf8' });
assert.equal(again.match(/^unchanged /gm)?.length, 6, again);
step('installed the pack, and a second install changed nothing');

// The slot the pack tells the agent to search for is there to be found.
assert.match(await readFile(join(app, 'app', 'page.tsx'), 'utf8'), /<prb\.Slot id="hero"/);

// 3. Core: apply the example's site file, mint a token, serve.
execFileSync(BIN, ['apply', join(app, 'site.json')], { env, stdio: ['ignore', 'ignore', 'inherit'] });
const token = execFileSync(BIN, ['mcp-token', 'shop', 'Claude Code'], { env, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
assert.match(token, /^prb_mcp_/);
const core = spawn(BIN, ['serve'], { env: { ...env, PRB_BIND: `127.0.0.1:${port}` }, stdio: ['ignore', 'ignore', 'inherit'] });
for (let i = 0; ; i++) {
  if (i > 100) throw new Error('Core did not come up');
  if (await fetch(`http://127.0.0.1:${port}/health`).then((r) => r.ok, () => false)) break;
  await new Promise((r) => setTimeout(r, 100));
}
step(`Core serving on ${port}`);

// The agent's environment holds the token; the configs only name the variable.
const agentEnv = { [TOKEN_ENV]: token };
function expand(s) {
  return s.replace(/\$\{(?:env:)?([A-Z_][A-Z0-9_]*)\}/g, (_, k) => {
    if (!(k in agentEnv)) throw new Error(`config references unset ${k}`);
    return agentEnv[k];
  });
}
async function connect(configPath) {
  const entry = JSON.parse(await readFile(join(app, configPath), 'utf8')).mcpServers.pratibimba;
  const headers = Object.fromEntries(Object.entries(entry.headers).map(([k, v]) => [k, expand(v)]));
  const client = new Client({ name: `e2e via ${configPath}`, version: '0.0.0' });
  await client.connect(new StreamableHTTPClientTransport(new URL(entry.url), { requestInit: { headers } }));
  return client;
}
const who = { agent: 'Claude Code', justification: 'End-to-end check of the instruction pack.' };

try {
  // 4. Without the token the server refuses: the header in the config is what gets us in.
  const bare = new Client({ name: 'no token', version: '0.0.0' });
  await assert.rejects(bare.connect(new StreamableHTTPClientTransport(new URL(`http://127.0.0.1:${port}/mcp`))));
  step('no token: refused');

  // 5. Claude Code's config: list slots, read first, then propose.
  const c = await connect('.mcp.json');
  const tools = (await c.listTools()).tools.map((t) => t.name).sort();
  assert.deepEqual(tools, ['create_audience', 'create_slot', 'explain_decision', 'fast_forward_visitor', 'get_results', 'get_site', 'install_plugin', 'list_audiences',
    'list_decisions', 'list_experiments', 'list_plugins', 'list_sites', 'list_slots', 'list_variants', 'propose_variant', 'retire_variant', 'simulate', 'update_slot']);
  const slots = (await c.callTool({ name: 'list_slots', arguments: { ...who, justification: 'Finding the hero slot.' } })).structuredContent.slots;
  assert.deepEqual(slots.map((s) => s.key).sort(), ['hero', 'promo']);
  step(`.mcp.json: ${tools.length} tools; slots ${slots.map((s) => `${s.key} (champion ${s.champion})`).join(', ')}`);
  await c.callTool({ name: 'get_results', arguments: { ...who, slot: 'hero', justification: 'Reading hero results before drafting.' } });
  const before = (await c.callTool({ name: 'list_variants', arguments: { ...who, slot: 'hero', justification: 'Checking what hero has tried.' } })).structuredContent;

  // As the pack says: a real sibling component, and the same markup as the html argument.
  const key = `returning-reorder-${process.pid}`;
  const html = '<section class="hero"><h1>Your usual roast, ready again</h1><p>Small batches from Chikmagalur, shipped the day they rest.</p><a class="cta" href="#shop">Reorder espresso</a></section>';
  await writeFile(join(app, 'app', 'components', 'ReorderHero.tsx'),
    `export function ReorderHero() {\n  return (\n    <section className="hero">\n      <h1>Your usual roast, ready again</h1>\n      <p>Small batches from Chikmagalur, shipped the day they rest.</p>\n      <a className="cta" href="#shop">Reorder espresso</a>\n    </section>\n  );\n}\n`);
  const proposed = await c.callTool({
    name: 'propose_variant',
    arguments: { ...who, slot: 'hero', key, html,
      justification: 'Hypothesis: returning visitors act on a reorder prompt more than on the first-visit pitch.',
      note: 'Changes the headline and CTA to a reorder. Component: app/components/ReorderHero.tsx. No results yet, so this is a guess.' },
  });
  assert.equal(proposed.isError, false, JSON.stringify(proposed.content));
  assert.equal(proposed.structuredContent.status, 'draft');
  assert.ok(proposed.structuredContent.checks.every((x) => x.status !== 'fail'), JSON.stringify(proposed.structuredContent.checks));
  step(`proposed ${key}: status ${proposed.structuredContent.status}; checks ${proposed.structuredContent.checks.map((x) => `${x.name}=${x.status}`).join(' ')}`);

  // 6. Cursor's config reaches the same server and sees the draft, still a draft.
  const cur = await connect('.cursor/mcp.json');
  const after = (await cur.callTool({ name: 'list_variants', arguments: { agent: 'Cursor', slot: 'hero', justification: 'Confirming the draft landed.' } })).structuredContent;
  const draft = after.variants.find((v) => v.key === key);
  assert.equal(draft.status, 'draft');
  assert.equal(draft.author, 'agent');
  assert.equal(after.variants.length, before.variants.length + 1);
  step('.cursor/mcp.json: sees returning-reorder as a draft by an agent');

  // 7. The generic snippet works too.
  const gen = await connect('.pratibimba/mcp.json');
  const site = (await gen.callTool({ name: 'get_site', arguments: { agent: 'generic', justification: 'Generic snippet check.' } })).structuredContent;
  assert.equal(site.id, 'shop');
  step('.pratibimba/mcp.json: get_site answers for shop');

  // The manifest still serves only approved variants.
  const manifest = await (await fetch(`http://127.0.0.1:${port}/manifest/shop`)).text();
  assert.ok(!manifest.includes(key));
  step('the public manifest does not carry the draft');
  for (const x of [c, cur, gen]) await x.close();
  console.log('\nok: pack installed, three configs connected, one draft proposed');
} finally {
  core.kill('SIGTERM');
  await rm(app, { recursive: true, force: true });
}
