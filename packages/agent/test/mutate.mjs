// Proves the suite can fail: each mutation breaks one guarded behaviour in a copy of the
// package, and the suite must go red on it. A mutation whose pattern is not found is itself a
// failure, so this script cannot pass by mutating nothing.
//
//   node test/mutate.mjs

import { cp, mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const pkg = join(dirname(fileURLToPath(import.meta.url)), '..');
const S = 'src/install.mjs';
const C = 'bin/cli.mjs';
const G = 'pack/guide.md';

const MUTANTS = [
  ['an edited pack file is overwritten', S, "if (!force) return { action: 'kept'", "if (false) return { action: 'kept'"],
  ['an identical pack file is not recognised', S, "if (old === content) return { action: 'unchanged' };", ''],
  ['other MCP servers are dropped', S, 'doc.mcpServers = { ...servers, [SERVER_NAME]: entry };', 'doc.mcpServers = { [SERVER_NAME]: entry };'],
  ['other top-level config keys are dropped', S, 'doc.mcpServers = { ...servers, [SERVER_NAME]: entry };', 'doc = { mcpServers: { ...servers, [SERVER_NAME]: entry } };'],
  ['a different pratibimba entry is overwritten', S, 'if (current !== undefined && !force)', 'if (false)'],
  ['an unparsable config is overwritten', S, "} catch (e) {\n      return { action: 'error'", "} catch (e) {\n      doc = {}; if (0) return { action: 'error'"],
  ['the config entry is compared by reference', S, 'isDeepStrictEqual(current, entry)', 'current === entry'],
  ['Claude Code ignores --token-env', S, "claude: { type: 'http', url, headers: { Authorization: `Bearer \\${${tokenEnv}}` } }", "claude: { type: 'http', url, headers: { Authorization: 'Bearer ${PRATIBIMBA_MCP_TOKEN}' } }"],
  ['Cursor gets the ${VAR} form it does not expand', S, 'Bearer \\${env:${tokenEnv}}', 'Bearer \\${${tokenEnv}}'],
  ['the generic snippet drops its type', S, "generic: { type: 'http', url,", 'generic: { url,'],
  ['a base URL is not given /mcp', S, 'path.endsWith(\'/mcp\') ? path : `${path}/mcp`', 'path'],
  ['the query string is kept', S, "u.search = '';", ''],
  ['credentials in the URL are accepted', S, "if (u.username || u.password) throw", 'if (false) throw'],
  ['the instructions ignore --token-env', S, 'TOKEN_ENV: tokenEnv', "TOKEN_ENV: 'PRATIBIMBA_MCP_TOKEN'"],
  ['the instructions ignore --url', S, 'MCP_URL: url', "MCP_URL: 'http://localhost:8080/mcp'"],
  ['the skill is misnamed', S, "'name: pratibimba'", "'name: prb'"],
  ['the CLI exits 0 after an error', C, "process.exit(report.some((r) => r.action === 'error') ? 1 : 0);", 'process.exit(0);'],
  ['the CLI ignores PRATIBIMBA_MCP_URL', C, 'values.url ?? process.env.PRATIBIMBA_MCP_URL ?? DEFAULT_URL', 'values.url ?? DEFAULT_URL'],
  ['the pack no longer says a person approves', G, 'A person approves. You do not, and you cannot.', 'A person usually approves.'],
  ['the pack drops the site-file rule', G, '- **Never add variants to the site file**', '- **Prefer not to edit the site file**'],
  ['the pack no longer reads before proposing', G, '### 1. Read before you write', '### 1. Optionally, look around'],
  ['the pack lets a new slot go live at once', G, 'reaches no visitor until a\n  person approves that baseline', 'goes live at once, before a\n  person looks at that baseline'],
  ['the pack lets update_slot edit live slots', G, '**`update_slot` changes only a draft slot.**', '**`update_slot` changes any slot.**'],
  ['the pack says retire_variant retires', G, '**`retire_variant` only proposes.**', '**`retire_variant` retires the variant.**'],
  ['the pack says install_plugin installs', G, '**`install_plugin` only requests.** Nothing is installed until a person confirms', '**`install_plugin` installs the plugin.** It is on at once'],
  ['the pack lets agents request de-anonymisers freely', G, 'Do not request one\n  unless the user asked for that plugin by name', 'Request one\n  whenever it helps'],
  ['the pack drops the keeps-serving rule', G, 'The variant **keeps serving** until a person confirms.', 'The variant stops serving at once.'],
  ['the pack drops the justification rule', G, '## Every call needs `agent` and `justification`', '## Calls'],
  ['the pack stops grounding in the repo', G, '- **Use only true facts.**', '- **Be persuasive.**'],
];

let survived = 0;
for (const [name, file, from, to] of MUTANTS) {
  const dir = await mkdtemp(join(tmpdir(), 'prb-mutant-'));
  for (const d of ['src', 'bin', 'pack', 'test', 'package.json']) await cp(join(pkg, d), join(dir, d), { recursive: true });
  const path = join(dir, file);
  const text = await readFile(path, 'utf8');
  if (!text.includes(from)) {
    console.log(`MISSING  ${name}: pattern not found in ${file}`);
    survived++;
    continue;
  }
  await writeFile(path, text.replace(from, to));
  const r = spawnSync(process.execPath, ['--test', 'test/install.test.mjs'], { cwd: dir, encoding: 'utf8' });
  const failed = (r.stdout.match(/^# fail (\d+)/m) ?? r.stdout.match(/ℹ fail (\d+)/))?.[1];
  if (r.status === 0) {
    console.log(`SURVIVED ${name}`);
    survived++;
  } else {
    console.log(`killed   ${name} (${failed ?? '?'} failing)`);
  }
  await rm(dir, { recursive: true });
}
console.log(`\n${MUTANTS.length - survived}/${MUTANTS.length} mutants killed`);
process.exit(survived ? 1 : 0);
