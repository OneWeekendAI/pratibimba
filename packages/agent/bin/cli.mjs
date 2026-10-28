#!/usr/bin/env node
// npx @pratibimba/agent install [--url <core or /mcp URL>] [--token-env NAME] [--dir <repo>] [--force]
// npx @pratibimba/agent scan [--dir <repo>] [--json] [--limit N] [--write]

import { parseArgs } from 'node:util';
import { resolve } from 'node:path';
import { install, DEFAULT_TOKEN_ENV, DEFAULT_URL } from '../src/install.mjs';
import { scan, formatText, writeDrafts } from '../src/scan.mjs';

const USAGE = `usage: npx @pratibimba/agent install [options]

Drops the Pratibimba instruction pack into a repo: two Claude Code skills (pratibimba, for
working with slots, and pratibimba-integrate, for adding Pratibimba to an existing site) and
.mcp.json, the matching Cursor rules and .cursor/mcp.json, and generic instructions plus an
MCP snippet in .pratibimba/.
Safe to re-run: it never overwrites a file you edited and only ever touches the
"pratibimba" entry of an MCP config.

  --url <url>        Core's URL or its /mcp URL (default: $PRATIBIMBA_MCP_URL, else ${DEFAULT_URL})
  --token-env <NAME> the environment variable holding the prb_mcp_ token (default ${DEFAULT_TOKEN_ENV})
  --dir <path>       the repo to install into (default: the current directory)
  --force            replace pack files you edited and an existing "pratibimba" MCP entry

usage: npx @pratibimba/agent scan [options]

Lists places in the repo that could become slots (heroes, calls to action, product lists,
promo lines, returning-visitor spots), best first, with file:line, kind, audience, goal and why.
JSX/TSX and plain HTML. Writes nothing unless --write.

  --dir <path>       the repo to scan (default: the current directory)
  --json             print JSON instead of text
  --limit <n>        show at most n candidates
  --write            add the candidates to .pratibimba/slot-drafts.json as drafts (never site.json)`;

if (process.argv[2] === 'scan') {
  let opts;
  try {
    opts = parseArgs({
      args: process.argv.slice(3),
      options: {
        dir: { type: 'string' }, json: { type: 'boolean', default: false },
        limit: { type: 'string' }, write: { type: 'boolean', default: false },
      },
    }).values;
    if (opts.limit !== undefined && !/^[1-9]\d*$/.test(opts.limit)) throw new Error('--limit takes a positive whole number');
  } catch (e) {
    console.error(`${e.message}\n\n${USAGE}`);
    process.exit(2);
  }
  try {
    const dir = resolve(opts.dir ?? '.');
    const result = await scan({ dir, limit: opts.limit && Number(opts.limit) });
    console.log(opts.json ? JSON.stringify(result, null, 2) : formatText(result));
    if (opts.write) {
      const w = await writeDrafts({ dir, candidates: result.candidates });
      (opts.json ? console.error : console.log)(`\nwrote ${w.path}: ${w.added} added, ${w.kept} already there. None is live until a person approves it.`);
    }
  } catch (e) {
    console.error(`pratibimba agent scan: ${e.message}`);
    process.exit(1);
  }
  process.exit(0);
}

let parsed;
try {
  parsed = parseArgs({
    allowPositionals: true,
    options: {
      url: { type: 'string' },
      'token-env': { type: 'string' },
      dir: { type: 'string' },
      force: { type: 'boolean', default: false },
      help: { type: 'boolean', short: 'h', default: false },
    },
  });
} catch (e) {
  console.error(`${e.message}\n\n${USAGE}`);
  process.exit(2);
}
const { values, positionals } = parsed;
if (values.help || positionals[0] !== 'install' || positionals.length !== 1) {
  console.log(USAGE);
  process.exit(values.help ? 0 : 2);
}

const url = values.url ?? process.env.PRATIBIMBA_MCP_URL ?? DEFAULT_URL;
const tokenEnv = values['token-env'] ?? DEFAULT_TOKEN_ENV;
let report;
try {
  report = await install({ dir: resolve(values.dir ?? '.'), url, tokenEnv, force: values.force });
} catch (e) {
  console.error(`pratibimba agent install: ${e.message}`);
  process.exit(2);
}
for (const r of report) console.log(`${r.action.padEnd(9)} ${r.path}${r.detail ? `  (${r.detail})` : ''}`);
if (!values.url && !process.env.PRATIBIMBA_MCP_URL) console.log(`\nNo --url given, so the configs point at ${DEFAULT_URL}.`);
console.log(`
Next:
  1. Make a token on the Core host:  pratibimba mcp-token <site> "Claude Code"
  2. Export it where your agent runs: export ${tokenEnv}=prb_mcp_...   (never commit it)
  3. Restart your agent. Claude Code asks you to approve the project's .mcp.json server once.
  4. Commit the pack files, so everyone's agent works the same way.`);
process.exit(report.some((r) => r.action === 'error') ? 1 : 0);
