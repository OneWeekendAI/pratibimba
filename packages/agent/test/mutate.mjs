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
const SC = 'src/scan.mjs';

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
  ['the pack says history reaches every visitor', G, 'History is known only for identified visitors', 'History is known for every visitor'],
  ['the pack stops grounding in the repo', G, '- **Use only true facts.**', '- **Be persuasive.**'],
  ['the pack drops the scan-first step', G, '### 0. Run `scan` first', '### 0. Optionally scan'],
  ['the pack says scan drafts are live', G, '**A draft in that file is not a slot and reaches no visitor.**', '**A draft in that file is live at once.**'],
  ['the pack still sends copy and sorted drafts to the owner', G, 'Every draft carries the exact `create_slot` arguments for its kind', 'A swap draft goes through `create_slot`; copy and sorted go to the owner'],
  ['scan --write drops the create_slot arguments', SC, 'create_slot: createSlotArgs(c), ', ''],
  ['scan --write sends the copy baseline text to the server', SC, ".map(({ name, max, required, type }) => (type === 'image' ? { name, type, required } : { name, max, required }))", ''],
  ['scan --write drops the copy fields', SC, "if (c.kind === 'copy') args.copy =", 'if (false) args.copy ='],
  ['scan --write sends no kind', SC, 'metric: c.metric, kind: c.kind };', 'metric: c.metric };'],
  ['scan --write does not ask for sorted items', SC, "c.kind === 'sorted' ? 'items:", "false ? 'items:"],
  ['scan --write keeps the owner note', SC, 'every kind through create_slot', 'copy and sorted slots through the site owner, swap'],
  ['scan proposes a copy max over 500', SC, 'max: Math.min(500, Math.max(40,', 'max: Math.min(5000, Math.max(40,'],
  // scan: each heuristic and each safety rule
  ['scan offers elements inside an existing slot', SC, 'if (inside(el, f.covered) || inside(el, chosen)) continue;', 'if (inside(el, chosen)) continue;'],
  ['scan ignores components rendered in a slot', SC, 'if (exported.some((n) => coveredNames.has(n)))', 'if (false)'],
  ['scan offers containers holding a slot', SC, 'if (f.covered.some(([a]) => a > el.start && a < el.end)) continue;', ''],
  ['scan offers nav links', SC, "a.name === 'nav' || ", ''],
  ['scan ignores site.json selectors', SC, "site.find((s) => matches(s.selector, el)) : undefined", 'undefined : undefined'],
  ['scan forgets a site.json slot\'s declared kind', SC, "['copy', 'sorted'].includes(declared) ? declared : 'swap'", "'swap'"],
  ['scan ignores prb:slot comments', SC, 'c.before === el.start', 'false'],
  ['scan makes a dynamic hero a copy slot', SC, 'const copy = fields.headline !== undefined;', 'const copy = true;'],
  ['scan misses mapped lists', SC, "if ((f.fmt !== 'jsx' || /key=/.test(body))", 'if (false'],
  ['scan misses HTML card grids', SC, 'if (n >= 3 &&', 'if (n >= 30 &&'],
  ['scan misses returning-visitor spots', SC, '(leaf && RETURNING_TEXT.test(lit))', 'false'],
  ['scan calls a page wrapper a promo', SC, '(leaf && PROMO_TEXT.test(lit)', '(PROMO_TEXT.test(lit)'],
  ['scan offers CTAs with dynamic words', SC, 'if (isAction(el, f) && literalOnly(el, f))', 'if (isAction(el, f))'],
  ['scan ranks by file order, not by kind', SC, 'b.score - a.score || ', ''],
  ['scan reuses an existing key', SC, 'while (taken.has(k))', 'while (false)'],
  ['scan walks node_modules', SC, "if (!SKIP_DIRS.has(e.name))", 'if (true)'],
  ['--write overwrites edited drafts', SC, 'if (have.has(c.key)) continue;', ''],
  ['--write replaces an unparsable drafts file', SC, "if (e.code !== 'ENOENT') throw", 'if (false) throw'],
  // G4: Vue, Svelte, Astro, and a tokenizer that knows JS from markup
  ['scan skips .vue/.svelte/.astro files', SC, 'html?|vue|svelte|astro)$/', 'html?)$/'],
  ['scan reads <script> and <style> bodies as markup', SC, 'if (RAW.test(t.name) && !t.self)', 'if (false)'],
  ['scan reads Astro frontmatter as markup', SC, 'const fm = o.frontmatter &&', 'const fm = false &&'],
  ['scan reads tags inside JS strings', SC, `if (c === '"' || c === "'") { jsStr(c);`, 'if (false) { jsStr(c);'],
  ['scan reads tags inside template literals', SC, "if (c === '`') { template();", 'if (false) { template();'],
  ['scan reads tags inside regex literals', SC, "if (c === '/' && !value && regex())", 'if (false && regex())'],
  ['scan reads tags inside // comments', SC, "if (c === '/' && src[i + 1] === '/') {", 'if (false) {'],
  ['scan reads tags inside /* */ comments', SC, "if (c === '/' && src[i + 1] === '*') {", 'if (false) {'],
  ['scan reads a < after a value as a tag', SC, "if (c === '<' && !value &&", "if (c === '<' &&"],
  ['scan keeps a JSX misreading', SC, 'if (element()) return true;', 'element(); return true;'],
  ['scan reads JSX {expressions} as text', SC, "if (c === '{') { i++; js('}'); if (i >= n) return false; i++; continue; }", ''],
  ['scan reads Vue {{ }} as text', SC, "o.mustache && src.startsWith('{{', i)", "false && src.startsWith('{{', i)"],
  ['scan reads a Svelte {/block} keyword as JS', SC, "if (/[#:/@]/.test(src[i] ?? ''))", 'if (false)'],
  ['scan names no Vue/Svelte/Astro component by its file', SC, "if (f.fmt !== 'jsx' && f.fmt !== 'html') exported.push(", 'if (false) exported.push('],
  ['scan misses Vue v-for lists', SC, 'vue: /\\sv-for\\s*=/, ', ''],
  ['scan misses Svelte {#each} lists', SC, 'svelte: /\\{#each\\b/ ', ''],
  ['scan misses Astro .map lists', SC, 'astro: /\\.map\\(/, ', ''],
  ['scan offers unkeyed JSX lists', SC, "(f.fmt !== 'jsx' || /key=/.test(body))", 'true'],
  ['scan ignores prb:slot comments outside HTML', SC, 'const commentKey = f.markup ?', "const commentKey = f.fmt === 'html' ?"],
  ['scan ignores site.json selectors outside HTML', SC, 'const siteSlot = f.markup ?', "const siteSlot = f.fmt === 'html' ?"],
  ['scan misses static card lists outside HTML', SC, 'if (f.markup && isContainer) {', "if (f.fmt === 'html' && isContainer) {"],
  ['scan tells Vue/Svelte/Astro to use a component', SC, 'if (!/\\.[jt]sx$/.test(file)) {', 'if (/\\.html?$/.test(file)) {'],
  ['scan hides that Vue/Svelte/Astro use the script tag', SC, "if (!/\\.html?$/.test(file)) m += '; this is the script tag", "if (false) m += '; this is the script tag"],
  ['scan gives a copy mark no fields', SC, 'if (words.length) m +=', 'if (false) m +='],
  ['scan gives an image field no mark', SC, "filter((x) => x.type === 'image') : []) m +=", "filter((x) => false) : []) m +="],
  ['scan offers no image banner', SC, "fields: [imageField('image', imgs[0])],", 'fields: [],'],
  ['the pack does not teach upload_asset', G, 'call `upload_asset`', 'call `propose_copy`'],
  ['scan gives a sorted mark no item keys', SC, "if (c.kind === 'sorted') m +=", 'if (false) m +='],
  ['the pack says Vue/Svelte/Astro have a component', G, 'For Vue, Svelte and Astro there is **no SDK component**', 'For Vue, Svelte and Astro use the SDK component'],
  ['scan writes without --write', C, 'if (opts.write) {', 'if (true) {'],
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
  const r = spawnSync(process.execPath, ['--test', 'test/install.test.mjs', 'test/scan.test.mjs'], { cwd: dir, encoding: 'utf8', env: { ...process.env, PRB_EXAMPLES: join(pkg, '..', '..', 'examples') } });
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
