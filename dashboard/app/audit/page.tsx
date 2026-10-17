'use client';
// One MCP tool call's audit record: who called what, for whom, why, with which arguments, and
// what it wrote. Every tools/call leaves one, refused or not.
import Link from 'next/link';
import { useQuery } from '@tanstack/react-query';
import { Suspense, type ReactNode } from 'react';
import { McpTailCard } from '@/components/agent';
import { Shell } from '@/components/shell';
import { Card, Chip, Empty, Loading, SectionTitle, type ChipTone } from '@/components/ui';
import { api, type McpCallDetail, type SiteRow } from '@/lib/api';
import { when } from '@/lib/format';
import { useParam } from '@/lib/page';

const OUTCOME: Record<McpCallDetail['outcome'], ChipTone> = { ok: 'live', error: 'warn', refused: 'paused' };

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[160px_1fr] gap-4 px-4 py-2.5 text-[13px]">
      <dt className="text-muted">{label}</dt>
      <dd className="min-w-0 break-words">{children}</dd>
    </div>
  );
}

function Record({ c }: { c: McpCallDetail }) {
  return (
    <div className="max-w-4xl space-y-6">
      <Card>
        <dl className="divide-y divide-line" data-testid="audit-record">
          <Row label="Agent"><b className="font-medium">{c.agent || 'unnamed agent'}</b>{c.on_behalf_of && <span className="text-muted"> on behalf of {c.on_behalf_of}</span>}</Row>
          <Row label="Tool"><span className="font-mono">{c.tool}</span></Row>
          <Row label="Outcome"><Chip tone={OUTCOME[c.outcome]}>{c.outcome}</Chip>{c.error && <span className="ml-2 text-pause-ink">{c.error}</span>}</Row>
          <Row label="Justification">{c.justification ? <span data-testid="audit-justification">“{c.justification}”</span> : <span className="text-muted">none given</span>}</Row>
          <Row label="Token"><span className="font-mono">{c.token}</span></Row>
          <Row label="JSON-RPC id"><span className="font-mono">{JSON.stringify(c.call_id)}</span></Row>
          <Row label="At">{when(c.at)}</Row>
        </dl>
      </Card>
      {c.variants.length > 0 && (
        <section>
          <SectionTitle>What it wrote</SectionTitle>
          <Card className="divide-y divide-line text-[13px]">
            {c.variants.map((v) => (
              <div key={v.id} className="flex items-center gap-2 px-4 py-2.5">
                <span className="font-mono">{v.key}</span>
                <span className="text-muted">on slot {v.slot}</span>
                <Chip tone={v.status === 'draft' ? 'warn' : v.status === 'approved' ? 'live' : 'off'}>{v.status}</Chip>
                {v.status === 'draft' && <Link className="ml-auto text-xs text-accent underline" href={`/approvals/?site=${c.site}`}>Review in Approvals</Link>}
              </div>
            ))}
          </Card>
        </section>
      )}
      <section>
        <SectionTitle hint="exactly as the agent sent them">Arguments</SectionTitle>
        <pre className="overflow-x-auto rounded-lg border border-line bg-surface p-4 font-mono text-[12px] leading-relaxed whitespace-pre-wrap" data-testid="audit-arguments">{JSON.stringify(c.arguments, null, 2)}</pre>
      </section>
    </div>
  );
}

/** /audit with no id: the site's newest tool calls, each linking to its record. */
function AuditLog() {
  const param = useParam('site');
  const sites = useQuery({ queryKey: ['sites'], queryFn: () => api.get<SiteRow[]>('/sites'), enabled: !param });
  const site = param ?? (sites.data?.length === 1 ? sites.data[0].id : null);
  const title = <><div className="text-xs text-muted">{site ?? 'Audit'}</div><h1 className="font-serif text-3xl">Audit log</h1></>;
  return (
    <Shell site={site} title={title}>
      {site ? (
        <div className="max-w-4xl"><McpTailCard site={site} /></div>
      ) : sites.isLoading ? (
        <Loading />
      ) : !sites.data?.length ? (
        <Empty>No sites yet, so no tool calls.</Empty>
      ) : (
        <Card className="max-w-xl divide-y divide-line text-[13px]">
          {sites.data.map((s) => (
            <Link key={s.id} href={`/audit/?site=${s.id}`} className="block px-4 py-2.5 hover:bg-surface-2/60">
              <span className="font-mono">{s.id}</span> <span className="text-muted">tool calls</span>
            </Link>
          ))}
        </Card>
      )}
    </Shell>
  );
}

function Page() {
  const id = useParam('id');
  const q = useQuery({ queryKey: ['mcp-call', id], queryFn: () => api.get<McpCallDetail>(`/mcp-calls/${id}`), enabled: !!id });
  if (!id) return <AuditLog />;
  return (
    <Shell site={q.data?.site ?? null} title={<><div className="text-xs text-muted">{q.data?.site ?? 'Audit'} · MCP audit log</div><h1 className="font-serif text-3xl">Tool call #{id}</h1></>}>
      {q.isError ? <Empty>That tool call was not found.</Empty> : q.data ? <Record c={q.data} /> : <Loading />}
    </Shell>
  );
}

export default function AuditPage() {
  return (
    <Suspense>
      <Page />
    </Suspense>
  );
}
