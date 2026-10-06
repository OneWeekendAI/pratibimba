'use client';
// Bring-your-own-agent: the card that connects a coding agent over MCP, and the live tail of
// what agents have called. Both read /api/sites/:site/mcp-calls; nothing here talks to an LLM.
import clsx from 'clsx';
import Link from 'next/link';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { Button, Card, Chip, SectionTitle, inputClass, type ChipTone } from '@/components/ui';
import { api, ApiError, type McpCall, type McpTail } from '@/lib/api';
import { ago } from '@/lib/format';

/** The instruction pack's installer. The token is read from the environment, never written to a file. */
export const packCommand = (url: string) => `npx @pratibimba/agent install --url ${url}`;
const OUTCOME: Record<McpCall['outcome'], ChipTone> = { ok: 'live', error: 'warn', refused: 'paused' };

export function useMcp(site: string) {
  return useQuery({ queryKey: ['mcp', site], queryFn: () => api.get<McpTail>(`/sites/${site}/mcp-calls?limit=25`), refetchInterval: 3000 });
}

function CopyBlock({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="relative">
      <pre className="rounded-md bg-chrome px-3 py-2.5 pr-20 font-mono whitespace-pre-wrap break-all text-[12px] leading-relaxed text-chrome-ink" data-testid={`copy-${label}`}>{text}</pre>
      <button
        type="button"
        aria-label={`Copy ${label}`}
        className="absolute top-1.5 right-1.5 rounded border border-chrome-2 bg-chrome-2 px-2 py-0.5 text-[11px] text-chrome-ink hover:text-white"
        onClick={async () => {
          try {
            await navigator.clipboard.writeText(text);
            setCopied(true);
            setTimeout(() => setCopied(false), 1500);
          } catch {}
        }}
      >
        {copied ? 'Copied' : 'Copy'}
      </button>
    </div>
  );
}

const AGENTS = ['Claude Code', 'Cursor', 'Generic MCP'] as const;
type AgentKind = (typeof AGENTS)[number];

function config(kind: AgentKind, url: string, token: string) {
  if (kind === 'Claude Code') return `claude mcp add --transport http pratibimba ${url} \\\n  --header "Authorization: Bearer ${token}"`;
  const servers = { mcpServers: { pratibimba: { url, headers: { Authorization: `Bearer ${token}` } } } };
  if (kind === 'Cursor') return `// .cursor/mcp.json\n${JSON.stringify(servers, null, 2)}`;
  return `POST ${url}   (MCP over streamable HTTP, JSON-RPC 2.0)\nAuthorization: Bearer ${token}`;
}

/** "Connect your coding agent": the endpoint, a token, a config block per agent, the pack. */
export function ConnectAgentCard({ site, bare }: { site: string; bare?: boolean }) {
  const qc = useQueryClient();
  const mcp = useMcp(site);
  const [origin, setOrigin] = useState('');
  const [kind, setKind] = useState<AgentKind>('Claude Code');
  const [name, setName] = useState('');
  const [token, setToken] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => setOrigin(location.origin), []);
  const mint = useMutation({
    mutationFn: () => api.post<{ token: string }>(`/sites/${site}/mcp-tokens`, { name: name.trim() }),
    onSuccess: (r) => {
      setToken(r.token);
      setError(null);
      qc.invalidateQueries({ queryKey: ['mcp', site] });
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'failed'),
  });
  const url = `${origin}${mcp.data?.endpoint ?? '/mcp'}`;
  const live = (mcp.data?.tokens ?? []).filter((t) => !t.revoked_at);
  const used = live.map((t) => t.last_used).filter((t): t is string => !!t).sort().at(-1);
  const firstCall = (mcp.data?.calls ?? []).some((c) => c.outcome !== 'refused');
  return (
    <Card className={clsx(bare ? 'border-0 bg-transparent shadow-none' : 'p-5')} data-testid="connect-agent">
      {!bare && <SectionTitle hint="your agent writes drafts; a person approves them here">Connect your coding agent</SectionTitle>}
      <div className="mb-4 flex flex-wrap items-center gap-2 text-xs" data-testid="agent-status">
        <Chip tone={live.length ? 'live' : 'off'}>{live.length} token{live.length === 1 ? '' : 's'}</Chip>
        <Chip tone={used ? 'live' : 'warn'}>{used ? `token last used ${ago(used)}` : 'no tool call yet'}</Chip>
        <Chip tone={firstCall ? 'live' : 'off'}>{firstCall ? 'agent connected' : 'waiting for the first call'}</Chip>
      </div>
      <div className="grid gap-5 lg:grid-cols-2">
        <div className="space-y-3">
          <div>
            <div className="mb-1 text-xs font-medium text-ink-2">1. The MCP endpoint</div>
            <CopyBlock label="endpoint" text={url} />
          </div>
          <div>
            <div className="mb-1 text-xs font-medium text-ink-2">2. A token scoped to <span className="font-mono">{site}</span></div>
            <p className="mb-2 text-xs text-muted">Shown once; only its hash is stored. From a shell on the server:</p>
            <CopyBlock label="mint" text={`pratibimba mcp-token ${site} <name>`} />
            <form
              className="mt-2 flex items-center gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                mint.mutate();
              }}
            >
              <input className={`${inputClass} w-48`} value={name} onChange={(e) => setName(e.target.value)} placeholder="ritwik-laptop" aria-label="Token name" />
              <Button type="submit" variant="ghost" disabled={!name.trim() || mint.isPending}>Or mint one here</Button>
            </form>
            {error && <p role="alert" className="mt-1 text-xs text-pause-ink">{error}</p>}
            {token && <p className="mt-2 text-xs text-warn-ink" data-testid="minted">Copy it now; it will not be shown again. It is filled into the config below.</p>}
          </div>
          <div>
            <div className="mb-1 text-xs font-medium text-ink-2">4. Install the instruction pack in your repo</div>
            <p className="mb-2 text-xs text-muted">Drops the prompt and skill files that teach your agent to read Pratibimba&apos;s state and submit variants as drafts. The token is read from <code className="font-mono">PRATIBIMBA_MCP_TOKEN</code> and never written to a file.</p>
            <CopyBlock label="pack" text={`export PRATIBIMBA_MCP_TOKEN=${token ?? '<token>'}\n${packCommand(origin)}`} />
          </div>
        </div>
        <div>
          <div className="mb-1 text-xs font-medium text-ink-2">3. Add it to your agent</div>
          <div role="tablist" aria-label="Agent" className="mb-2 flex gap-1">
            {AGENTS.map((a) => (
              <button
                key={a}
                role="tab"
                aria-selected={kind === a}
                onClick={() => setKind(a)}
                className={clsx('rounded-md px-2.5 py-1 text-xs', kind === a ? 'bg-accent text-accent-ink' : 'border border-line-strong text-ink-2 hover:bg-surface-2')}
              >
                {a}
              </button>
            ))}
          </div>
          <CopyBlock label="config" text={config(kind, url, token ?? '<token>')} />
          <p className="mt-2 text-xs text-muted">Every tool call must carry <code className="font-mono">agent</code> and a <code className="font-mono">justification</code>; each is recorded in the audit log below. There is no approve tool.</p>
        </div>
      </div>
    </Card>
  );
}

/** The newest MCP tool calls on this site, refreshed every few seconds. */
export function McpTailCard({ site }: { site: string }) {
  const mcp = useMcp(site);
  const calls = mcp.data?.calls ?? [];
  return (
    <section>
      <SectionTitle hint={<span className="inline-flex items-center gap-2"><span className="size-1.5 animate-pulse rounded-full motion-reduce:animate-none bg-[color:var(--good)]" aria-hidden />refreshes every 3 s</span>}>Agent tool calls</SectionTitle>
      <Card className="divide-y divide-line text-[13px]" data-testid="mcp-tail">
        {calls.length === 0 && <div className="px-4 py-6 text-center text-muted">No tool calls yet. They appear here as your agent makes them.</div>}
        {calls.map((c) => (
          <Link key={c.id} href={`/audit/?id=${c.id}`} className="grid grid-cols-[72px_1fr_auto] items-baseline gap-x-3 px-4 py-2.5 hover:bg-surface-2/60" data-testid="mcp-call">
            <span className="text-xs text-muted tabular">{ago(c.at)}</span>
            <span className="min-w-0">
              <span className="font-mono font-medium">{c.tool}</span>
              <span className="text-muted"> by </span>
              <span className="font-medium">{c.agent || 'unnamed agent'}</span>
              <span className="mt-0.5 block truncate text-xs text-ink-2">{c.outcome === 'refused' ? c.error : (c.justification ?? '')}</span>
            </span>
            <Chip tone={OUTCOME[c.outcome]}>{c.outcome}</Chip>
          </Link>
        ))}
      </Card>
    </section>
  );
}
