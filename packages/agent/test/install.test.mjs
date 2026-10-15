import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, mkdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import { install, mcpUrl } from '../src/install.mjs';

const run = promisify(execFile);
const CLI = join(dirname(fileURLToPath(import.meta.url)), '..', 'bin', 'cli.mjs');
const URL_ = 'https://core.example.com/mcp';
const PACK = ['.claude/skills/pratibimba/SKILL.md', '.cursor/rules/pratibimba.mdc', '.pratibimba/agent-instructions.md'];
const CONFIGS = ['.mcp.json', '.cursor/mcp.json', '.pratibimba/mcp.json'];
const ALL = [...PACK, ...CONFIGS];

async function repo() {
  return mkdtemp(join(tmpdir(), 'prb-pack-'));
}
const read = (dir, p) => readFile(join(dir, p), 'utf8');
const json = async (dir, p) => JSON.parse(await read(dir, p));
async function seed(dir, p, text) {
  await mkdir(dirname(join(dir, p)), { recursive: true });
  await writeFile(join(dir, p), text);
}
const actions = (report) => Object.fromEntries(report.map((r) => [r.path, r.action]));

// ---- a fresh install writes all six files ----

for (const p of ALL) {
  test(`fresh install creates ${p}`, async () => {
    const dir = await repo();
    const report = await install({ dir, url: URL_ });
    assert.equal(actions(report)[p], 'created');
    assert.ok((await read(dir, p)).length > 0);
  });
}

// ---- idempotence: a second run changes no byte ----

for (const p of ALL) {
  test(`a second install leaves ${p} unchanged, byte for byte`, async () => {
    const dir = await repo();
    await install({ dir, url: URL_ });
    const first = await read(dir, p);
    const report = await install({ dir, url: URL_ });
    assert.equal(actions(report)[p], 'unchanged');
    assert.equal(await read(dir, p), first);
  });
}

// ---- no clobbering ----

for (const p of PACK) {
  test(`an edited ${p} is kept, and --force replaces it`, async () => {
    const dir = await repo();
    await install({ dir, url: URL_ });
    const edited = `${await read(dir, p)}\n## Our house rule\nAlways propose two variants.\n`;
    await writeFile(join(dir, p), edited);
    assert.equal(actions(await install({ dir, url: URL_ }))[p], 'kept');
    assert.equal(await read(dir, p), edited);
    assert.equal(actions(await install({ dir, url: URL_, force: true }))[p], 'replaced');
    assert.doesNotMatch(await read(dir, p), /Our house rule/);
  });
}

for (const p of CONFIGS) {
  test(`${p}: other servers and top-level keys survive the install`, async () => {
    const dir = await repo();
    const other = { command: 'npx', args: ['-y', 'some-other-server'], env: { X: '1' } };
    await seed(dir, p, JSON.stringify({ mcpServers: { other }, inputs: [{ id: 'k' }] }, null, 4));
    assert.equal(actions(await install({ dir, url: URL_ }))[p], 'added');
    const doc = await json(dir, p);
    assert.deepEqual(doc.mcpServers.other, other);
    assert.deepEqual(doc.inputs, [{ id: 'k' }]);
    assert.equal(doc.mcpServers.pratibimba.url, URL_);
  });

  test(`${p}: an existing, different pratibimba entry is kept unless --force`, async () => {
    const dir = await repo();
    const mine = { url: 'https://staging.example.com/mcp', headers: { Authorization: 'Bearer ${MY_TOKEN}' } };
    const text = JSON.stringify({ mcpServers: { pratibimba: mine, other: { url: 'x' } } }, null, 2);
    await seed(dir, p, text);
    assert.equal(actions(await install({ dir, url: URL_ }))[p], 'kept');
    assert.equal(await read(dir, p), text);
    assert.equal(actions(await install({ dir, url: URL_, force: true }))[p], 'replaced');
    const doc = await json(dir, p);
    assert.equal(doc.mcpServers.pratibimba.url, URL_);
    assert.deepEqual(doc.mcpServers.other, { url: 'x' });
  });

  test(`${p}: a file that is not JSON is reported and left untouched`, async () => {
    const dir = await repo();
    const text = '{ "mcpServers": { // a comment, so not JSON\n } }';
    await seed(dir, p, text);
    const r = (await install({ dir, url: URL_ })).find((x) => x.path === p);
    assert.equal(r.action, 'error');
    assert.equal(await read(dir, p), text);
  });
}

test('the CLI exits 1 when a config could not be merged, and still installs the rest', async () => {
  const dir = await repo();
  await seed(dir, '.mcp.json', 'not json');
  const err = await run('node', [CLI, 'install', '--dir', dir, '--url', URL_]).catch((e) => e);
  assert.equal(err.code, 1);
  assert.match(err.stdout, /error\s+\.mcp\.json/);
  assert.match(await read(dir, '.cursor/mcp.json'), /pratibimba/);
  await rm(dir, { recursive: true });
});

// ---- the configs point at the right URL and token variable ----

test('.mcp.json (Claude Code) is an http server with a ${VAR} bearer header', async () => {
  const dir = await repo();
  await install({ dir, url: URL_ });
  assert.deepEqual((await json(dir, '.mcp.json')).mcpServers.pratibimba, {
    type: 'http',
    url: URL_,
    headers: { Authorization: 'Bearer ${PRATIBIMBA_MCP_TOKEN}' },
  });
});

test('.cursor/mcp.json uses Cursor\'s ${env:VAR} form', async () => {
  const dir = await repo();
  await install({ dir, url: URL_ });
  assert.deepEqual((await json(dir, '.cursor/mcp.json')).mcpServers.pratibimba, {
    url: URL_,
    headers: { Authorization: 'Bearer ${env:PRATIBIMBA_MCP_TOKEN}' },
  });
});

test('.pratibimba/mcp.json (generic) names the URL and the variable', async () => {
  const dir = await repo();
  await install({ dir, url: URL_ });
  assert.deepEqual((await json(dir, '.pratibimba/mcp.json')).mcpServers.pratibimba, {
    type: 'http',
    url: URL_,
    headers: { Authorization: 'Bearer ${PRATIBIMBA_MCP_TOKEN}' },
  });
});

test('--token-env reaches every config and every instruction file', async () => {
  const dir = await repo();
  await install({ dir, url: URL_, tokenEnv: 'SHOP_PRB_TOKEN' });
  assert.equal((await json(dir, '.mcp.json')).mcpServers.pratibimba.headers.Authorization, 'Bearer ${SHOP_PRB_TOKEN}');
  assert.equal((await json(dir, '.cursor/mcp.json')).mcpServers.pratibimba.headers.Authorization, 'Bearer ${env:SHOP_PRB_TOKEN}');
  assert.equal((await json(dir, '.pratibimba/mcp.json')).mcpServers.pratibimba.headers.Authorization, 'Bearer ${SHOP_PRB_TOKEN}');
  for (const p of PACK) {
    const text = await read(dir, p);
    assert.match(text, /`SHOP_PRB_TOKEN`/, p);
    assert.doesNotMatch(text, /PRATIBIMBA_MCP_TOKEN|\{\{[A-Z_]+\}\}/, p);
  }
});

test('the instruction files name the configured URL', async () => {
  const dir = await repo();
  await install({ dir, url: 'https://prb.shop.example/mcp' });
  for (const p of PACK) assert.match(await read(dir, p), /`https:\/\/prb\.shop\.example\/mcp`/, p);
});

test('the CLI reads PRATIBIMBA_MCP_URL when --url is absent', async () => {
  const dir = await repo();
  await run('node', [CLI, 'install', '--dir', dir], { env: { ...process.env, PRATIBIMBA_MCP_URL: 'http://127.0.0.1:9999' } });
  assert.equal((await json(dir, '.mcp.json')).mcpServers.pratibimba.url, 'http://127.0.0.1:9999/mcp');
});

test('a base URL gets /mcp; an /mcp URL is kept; query and trailing slashes go', () => {
  assert.equal(mcpUrl('https://core.example.com'), 'https://core.example.com/mcp');
  assert.equal(mcpUrl('https://core.example.com/'), 'https://core.example.com/mcp');
  assert.equal(mcpUrl('https://core.example.com/mcp/'), 'https://core.example.com/mcp');
  assert.equal(mcpUrl('https://example.com/prb/mcp?x=1'), 'https://example.com/prb/mcp');
  assert.equal(mcpUrl('http://localhost:8080/prb'), 'http://localhost:8080/prb/mcp');
});

test('bad URLs and variable names are refused before any file is written', async () => {
  const dir = await repo();
  await assert.rejects(install({ dir, url: 'ftp://core.example.com' }), /http or https/);
  await assert.rejects(install({ dir, url: 'https://u:prb_mcp_x@core.example.com' }), /not in the URL/);
  await assert.rejects(install({ dir, url: URL_, tokenEnv: 'prb-token' }), /environment variable/);
  await assert.rejects(readFile(join(dir, '.mcp.json')), { code: 'ENOENT' });
});

// ---- the prose: the four things the pack must teach, in every agent's copy ----

const RULES = {
  'read results and history before proposing': /### 1\. Read before you write[\s\S]*`get_results`[\s\S]*`list_variants`[\s\S]*`explain_decision`[\s\S]*### 4\. Propose/,
  'ground variants in the repo': /Use what the repo already uses[\s\S]*Use only true facts/,
  'a justification on every call': /Every call needs `agent` and `justification`/,
  'never claim a draft is live': /## Never claim a draft is live[\s\S]*A person approves\. You do not, and you cannot\./,
  'never add variants through the site file': /Never add variants to the site file/,
  'a new slot is a draft until a person activates it': /`create_slot` makes a draft slot\.[\s\S]*reaches no visitor until a\s+person approves that baseline/,
  'update_slot changes only a draft slot': /`update_slot` changes only a draft slot\./,
  'retire_variant only proposes and the variant keeps serving': /`retire_variant` only proposes\.[\s\S]*\*\*keeps serving\*\* until a person confirms/,
  'install_plugin only requests': /\*\*`install_plugin` only requests\.\*\* Nothing is installed until a person confirms/,
  'history audiences fail closed for unidentified visitors': /`days_since_last_purchase: \{gte: 30\}`[\s\S]*History is known only for identified visitors[\s\S]*Everyone else fails closed/,
  'run scan first, and its drafts are not slots': /### 0\. Run `scan` first[\s\S]*\*\*scan points, you judge\.\*\*[\s\S]*\*\*A draft in that file is not a slot and reaches no visitor\.\*\*[\s\S]*### 1\. Read before you write/,
  'every scan draft goes through create_slot, kind and all': /Every draft carries the exact `create_slot` arguments for its kind[\s\S]*a `sorted` draft first needs its `items`/,
  'create_slot takes copy fields and sorted items': /`create_slot` makes a draft slot\.[\s\S]*`copy` \(give `copy:[\s\S]*`sorted` \(give `items`/,
  'a de-anonymiser only when asked by name': /`deanonymiser`[\s\S]*Do not request one\s+unless the user asked for that plugin by name/,
};
for (const p of PACK) {
  for (const [name, re] of Object.entries(RULES)) {
    test(`${p} teaches: ${name}`, async () => {
      const dir = await repo();
      await install({ dir, url: URL_ });
      assert.match(await read(dir, p), re);
    });
  }
}

test('the Claude Code skill and the Cursor rule carry the front matter their tools need', async () => {
  const dir = await repo();
  await install({ dir, url: URL_ });
  assert.match(await read(dir, PACK[0]), /^---\nname: pratibimba\ndescription: Use when .+\n---\n/);
  assert.match(await read(dir, PACK[1]), /^---\ndescription: .+\nglobs:\nalwaysApply: false\n---\n/);
});
