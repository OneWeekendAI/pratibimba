# @pratibimba/agent

The Pratibimba instruction pack. It teaches Claude Code, Cursor or any MCP client to read experiment results and submit variants as drafts, which a human still has to approve before they are served.

```sh
npm i -g @pratibimba/agent    # or run it with npx, below
```

```sh
# in the project the SDK is installed in:
npx @pratibimba/agent scan       # proposes candidate slots (JSX/TSX, HTML, Vue, Svelte, Astro)
npx @pratibimba/agent install    # drops the pack into .claude / .cursor / .pratibimba, wires MCP config
npx @pratibimba/agent install --url https://core.example.com
```

Then mint a per-site MCP token in the Pratibimba dashboard. Re-running is safe: it never overwrites a file you edited. See the [main README](https://github.com/OneWeekendAI/pratibimba#readme).

MIT licensed.
