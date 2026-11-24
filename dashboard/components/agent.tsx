'use client';
// Bring-your-own-agent: the card that connects a coding agent over MCP, and the live tail of
// what agents have called. Both read /api/sites/:site/mcp-calls; nothing here talks to an LLM.
import clsx from 'clsx';
import Link from 'next/link';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { Bot, Check, ChevronDown, Copy, KeyRound, Plug, Radio, Terminal } from 'lucide-react';
import { Button, Card, Chip, Help, SectionTitle, inputClass, type ChipTone } from '@/components/ui';
import { api, ApiError, type McpCall, type McpTail } from '@/lib/api';
import { ago } from '@/lib/format';

/** The instruction pack's installer. The token is read from the environment, never written to a file. */
export const packCommand = (url: string) => `npx @pratibimba/agent install --url ${url}`;
const OUTCOME: Record<McpCall['outcome'], ChipTone> = { ok: 'live', error: 'warn', refused: 'paused' };

export function useMcp(site: string, slot?: string) {
  return useQuery({
    queryKey: ['mcp', site, slot ?? null],
    queryFn: () => api.get<McpTail>(`/sites/${site}/mcp-calls?limit=${slot ? 6 : 25}${slot ? `&slot=${encodeURIComponent(slot)}` : ''}`),
    refetchInterval: 3000,
  });
}

function CopyBlock({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="relative">
      <pre className="rounded-md border border-line bg-paper px-3 py-2.5 pr-11 font-mono text-[12.5px] leading-relaxed whitespace-pre-wrap text-ink [overflow-wrap:anywhere]" data-testid={`copy-${label}`}>{text}</pre>
      <button
        type="button"
        aria-label={`Copy ${label}`}
        title={copied ? 'Copied' : 'Copy'}
        className="absolute top-1.5 right-1.5 grid size-7 place-items-center rounded-md text-muted transition-colors hover:bg-surface-2 hover:text-ink"
        onClick={async () => {
          try {
            await navigator.clipboard.writeText(text);
            setCopied(true);
            setTimeout(() => setCopied(false), 1500);
          } catch {}
        }}
      >
        {copied ? <Check className="size-3.5" aria-hidden /> : <Copy className="size-3.5" aria-hidden />}
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

function Step({ n, title, help, children }: { n: number; icon: typeof Bot; title: string; help?: React.ReactNode; children: React.ReactNode }) {
  return (
    <div>
      <div className="mb-2 flex min-h-6 items-center gap-2 text-[13px] font-medium text-ink">
        <span className="grid size-5 place-items-center rounded-full border border-line-strong text-[11px] font-medium text-ink-2 tabular">{n}</span>
        {title}
        {help && <Help title={title}>{help}</Help>}
      </div>
      {children}
    </div>
    );
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
    <Card className={clsx(bare ? 'border-0 bg-transparent' : 'p-5 sm:p-6')} data-testid="connect-agent">
      {!bare && <SectionTitle icon={Bot} help="Your agent writes drafts over MCP. A person approves them in Review. There is no approve tool.">Connect your coding agent</SectionTitle>}
      <div className="mb-6 flex flex-wrap items-center gap-1.5" data-testid="agent-status">
        <Chip tone={live.length ? 'live' : 'off'} icon={KeyRound}>{live.length} token{live.length === 1 ? '' : 's'}</Chip>
        <Chip tone={used ? 'live' : 'warn'}>{used ? `token last used ${ago(used)}` : 'no tool call yet'}</Chip>
        <Chip tone={firstCall ? 'live' : 'off'} icon={Plug}>{firstCall ? 'agent connected' : 'waiting for the first call'}</Chip>
      </div>
      <div className={clsx("grid gap-x-8 gap-y-6", !bare && "lg:grid-cols-2")}>
        <div className="space-y-6">
          <Step n={1} icon={Radio} title="Endpoint">
            <CopyBlock label="endpoint" text={url} />
          </Step>
          <Step n={2} icon={KeyRound} title={`Token for ${site}`} help="Shown once; only its hash is stored. Mint it from a shell on the server, or here.">
            <CopyBlock label="mint" text={`pratibimba mcp-token ${site} <name>`} />
            <form
              className="mt-2 flex flex-wrap items-center gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                mint.mutate();
              }}
            >
              <input className={`${inputClass} w-44`} value={name} onChange={(e) => setName(e.target.value)} placeholder="ritwik-laptop" aria-label="Token name" />
              <Button type="submit" variant="ghost" icon={KeyRound} disabled={!name.trim() || mint.isPending}>Or mint one here</Button>
            </form>
            {error && <p role="alert" className="mt-1 text-xs text-pause-ink">{error}</p>}
            {token && <p className="mt-2 text-xs text-warn-ink" data-testid="minted">Copy it now; it will not be shown again.</p>}
          </Step>
        </div>
        <div className="space-y-6">
          <Step n={3} icon={Bot} title="Add it to your agent" help={<>Every tool call must carry <code className="font-mono">agent</code> and a <code className="font-mono">justification</code>. Each one is recorded in Audit.</>}>
            <div role="tablist" aria-label="Agent" className="mb-2 inline-flex flex-wrap rounded-md bg-surface-2 p-0.5">
              {AGENTS.map((a) => (
                <button
                  key={a}
                  role="tab"
                  aria-selected={kind === a}
                  onClick={() => setKind(a)}
                  className={clsx('h-7 rounded-[5px] px-2.5 text-[12.5px] transition-colors duration-150', kind === a ? 'bg-surface font-medium text-ink shadow-[0_0_0_1px_var(--line),0_1px_2px_rgb(0_0_0/0.06)]' : 'text-muted hover:text-ink')}
                >
                  {a}
                </button>
              ))}
            </div>
            <CopyBlock label="config" text={config(kind, url, token ?? '<token>')} />
          </Step>
          <Step n={4} icon={Terminal} title="Install the instruction pack" help={<>Adds the prompt and skill files that teach your agent to submit variants as drafts. The token is read from <code className="font-mono">PRATIBIMBA_MCP_TOKEN</code>, never written to a file.</>}>
            <CopyBlock label="pack" text={`export PRATIBIMBA_MCP_TOKEN=${token ?? '<token>'}\n${packCommand(origin)}`} />
          </Step>
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
      <SectionTitle icon={Terminal} help="The newest tool calls agents made on this site. It refreshes every 3 seconds; open one for its full record." hint={<span className="inline-flex items-center gap-1.5"><span className="size-1.5 rounded-full bg-good" aria-hidden />refreshing</span>}>Agent tool calls</SectionTitle>
      <Card className="divide-y divide-line text-[13px]" data-testid="mcp-tail">
        {calls.length === 0 && <div className="px-4 py-10 text-center text-muted">No tool calls yet.</div>}
        {calls.map((c) => (
          <Link key={c.id} href={`/audit/?id=${c.id}`} className="grid grid-cols-[1fr_auto] items-center gap-x-4 px-4 py-3 transition-colors first:rounded-t-lg last:rounded-b-lg hover:bg-surface-2/60 sm:grid-cols-[84px_1fr_auto]" data-testid="mcp-call">
            <span className="text-[12px] text-muted tabular max-sm:hidden">{ago(c.at)}</span>
            <span className="min-w-0">
              <span className="font-mono text-[12.5px] font-medium">{c.tool}</span>
              <span className="text-muted"> by </span>
              <span className="font-medium">{c.agent || 'unnamed agent'}</span>
              <span className="mt-0.5 block truncate text-[12.5px] text-muted">{c.outcome === 'refused' ? c.error : (c.justification ?? '')}</span>
            </span>
            <Chip tone={OUTCOME[c.outcome]}>{c.outcome}</Chip>
          </Link>
        ))}
      </Card>
    </section>
  );
}

const PACK_LABEL: Record<McpTail['packs'][number]['status'], [ChipTone, string]> = {
  installed: ['live', 'pack installed'],
  out_of_date: ['warn', 'pack out of date'],
  not_installed: ['off', 'not installed'],
};

/** The prompt the developer pastes into their own agent. Built here; nothing sends it anywhere. */
export function briefPrompt({ site, slot, champion, selector, lift, brief }: { site: string; slot: string; champion: string | null; selector: string; lift: string; brief: string }) {
  return [
    `Write a new variant for the Pratibimba slot "${slot}" on site "${site}", using the pratibimba MCP tools.`,
    `Slot: ${slot} (${selector}). Current champion: ${champion ?? 'none yet'}. Results so far: ${lift}.`,
    'Read get_site first for the brand rules and facts, and get_results for this slot. Submit a draft; do not expect to approve it.',
    'Call get_brand for the site\'s voice and style rules (null means none is declared).',,
    brief.trim() && `Brief: ${brief.trim()}`,
  ].filter(Boolean).join('\n');
}

/**
 * The slot workbench's agent sidebar: endpoint, a token, one status line per agent, a brief that
 * becomes a prompt to copy (it is never sent to a model by us), and the calls made on this slot.
 */
export function SlotAgentPanel({ site, slot, champion, selector, lift }: { site: string; slot: string; champion: string | null; selector: string; lift: string }) {
  const qc = useQueryClient();
  const mcp = useMcp(site, slot);
  const [origin, setOrigin] = useState('');
  const [token, setToken] = useState<string | null>(null);
  const [brief, setBrief] = useState('');
  const [error, setError] = useState<string | null>(null);
  useEffect(() => setOrigin(location.origin), []);
  const mint = useMutation({
    mutationFn: () => api.post<{ token: string }>(`/sites/${site}/mcp-tokens`, { name: `${slot}-agent` }),
    onSuccess: (r) => {
      setToken(r.token);
      setError(null);
      qc.invalidateQueries({ queryKey: ['mcp', site] });
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'failed'),
  });
  const calls = mcp.data?.calls ?? [];
  return (
    <details className="group p-4" data-testid="slot-agent">
      <summary className="flex cursor-pointer list-none items-center justify-between gap-2 rounded-sm text-[13px] font-medium text-ink-2 hover:text-ink [&::-webkit-details-marker]:hidden">
        Have your coding agent write a variant
        <ChevronDown className="size-4 text-muted transition-transform group-open:rotate-180" aria-hidden />
      </summary>
      <div className="mt-3 space-y-3">
      <p className="text-xs text-muted">We do not run your agent or send this brief to a model. Paste the prompt into your own agent; it writes drafts over MCP, and a person approves them in Review.</p>
      <CopyBlock label="slot-endpoint" text={`${origin}${mcp.data?.endpoint ?? '/mcp'}`} />
      {token ? (
        <>
          <CopyBlock label="slot-token" text={token} />
          <p className="text-xs text-warn-ink">Copy it now; it will not be shown again.</p>
        </>
      ) : (
        <Button variant="ghost" icon={KeyRound} disabled={mint.isPending} onClick={() => mint.mutate()}>Mint a token for this site</Button>
      )}
      {error && <p role="alert" className="text-xs text-pause-ink">{error}</p>}
      <ul className="space-y-1" data-testid="pack-status">
        {(mcp.data?.packs ?? []).map((p) => {
          const [tone, label] = PACK_LABEL[p.status];
          return (
            <li key={p.agent} className="flex items-center justify-between gap-2 text-[12.5px]" data-testid={`pack-${p.agent.replace(/\s+/g, '-')}`}>
              <span>{p.agent}</span>
              <Chip tone={tone}>{label}</Chip>
            </li>
          );
        })}
      </ul>
      <textarea className={`${inputClass} h-20 w-full py-2`} value={brief} onChange={(e) => setBrief(e.target.value)} placeholder="What should the variant do?" aria-label="Brief" />
      <CopyBlock label="prompt" text={briefPrompt({ site, slot, champion, selector, lift, brief })} />
      <div className="space-y-1.5 text-[12.5px]" data-testid="slot-mcp-tail">
        {calls.length === 0 && <p className="text-muted">No agent calls on this slot yet.</p>}
        {calls.map((c) => (
          <Link key={c.id} href={`/audit/?id=${c.id}`} className="block truncate text-ink-2 hover:text-ink" data-testid="slot-mcp-call">
            <span className="font-mono">{c.tool}</span> from {c.agent || 'unnamed agent'} <span className="text-muted">{ago(c.at)}</span>
          </Link>
        ))}
      </div>
      </div>
    </details>
  );
}
