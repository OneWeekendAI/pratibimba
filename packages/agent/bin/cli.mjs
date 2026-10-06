#!/usr/bin/env node
// npx @pratibimba/agent install [--url <core or /mcp URL>] [--token-env NAME] [--dir <repo>] [--force]

import { parseArgs } from 'node:util';
import { resolve } from 'node:path';
import { install, DEFAULT_TOKEN_ENV, DEFAULT_URL } from '../src/install.mjs';

const USAGE = `usage: npx @pratibimba/agent install [options]

Drops the Pratibimba instruction pack into a repo: a Claude Code skill and .mcp.json, a
Cursor rule and .cursor/mcp.json, and generic instructions plus an MCP snippet in .pratibimba/.
Safe to re-run: it never overwrites a file you edited and only ever touches the
"pratibimba" entry of an MCP config.

  --url <url>        Core's URL or its /mcp URL (default: $PRATIBIMBA_MCP_URL, else ${DEFAULT_URL})
  --token-env <NAME> the environment variable holding the prb_mcp_ token (default ${DEFAULT_TOKEN_ENV})
  --dir <path>       the repo to install into (default: the current directory)
  --force            replace pack files you edited and an existing "pratibimba" MCP entry`;

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
