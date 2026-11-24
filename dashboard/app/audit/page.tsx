'use client';
// One MCP tool call's audit record: who called what, for whom, why, with which arguments, and
// what it wrote. Every tools/call leaves one, refused or not.
import Link from 'next/link';
import { useQuery } from '@tanstack/react-query';
import { Suspense, type ReactNode } from 'react';
import { McpTailCard } from '@/components/agent';
import { FieldView } from '@/components/assets';
import { Crumb, Shell } from '@/components/shell';
import { Braces, ChevronDown, ChevronRight, FilePen, Globe } from 'lucide-react';
import { Card, Chip, Empty, Loading, SectionTitle, type ChipTone } from '@/components/ui';
import { api, type FieldValue, type McpCallDetail, type SiteRow } from '@/lib/api';
import { when } from '@/lib/format';
import { useParam } from '@/lib/page';

const OUTCOME: Record<McpCallDetail['outcome'], ChipTone> = { ok: 'live', error: 'warn', refused: 'paused' };

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[110px_1fr] gap-4 px-4 py-3 text-[13px] sm:grid-cols-[160px_1fr] sm:px-5">
      <dt className="text-muted">{label}</dt>
      <dd className="min-w-0 break-words">{children}</dd>
    </div>
  );
}

/** The copy a call proposed, as fields: propose_copy's `copy`, or a copy slot's `html` (a JSON object). */
function copyOf(c: McpCallDetail): Record<string, FieldValue> | null {
  const a = c.arguments;
  let v: unknown = c.tool === 'propose_copy' ? a.copy : a.html;
  if (typeof v === 'string') {
    try {
      v = JSON.parse(v);
    } catch {
      return null;
    }
  }
  if (!v || typeof v !== 'object' || Array.isArray(v)) return null;
  const ok = Object.values(v).every((x) => typeof x === 'string' || (!!x && typeof x === 'object' && typeof (x as { alt?: unknown }).alt === 'string'));
  return ok && Object.keys(v).length ? (v as Record<string, FieldValue>) : null;
}

function Record({ c }: { c: McpCallDetail }) {
  const fields = copyOf(c);
  return (
    <div className="max-w-4xl space-y-8">
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
          <SectionTitle icon={FilePen}>What it wrote</SectionTitle>
          <Card className="divide-y divide-line text-[13px]">
            {c.variants.map((v) => (
              <div key={v.id} className="flex flex-wrap items-center gap-2 px-4 py-3">
                <span className="font-medium">{v.key}</span>
                <span className="text-muted">on {v.slot}</span>
                <Chip tone={v.status === 'draft' ? 'warn' : v.status === 'approved' ? 'live' : 'off'}>{v.status}</Chip>
                {v.status === 'draft' && <Link className="ml-auto inline-flex items-center gap-1 rounded-sm text-[13px] font-medium text-ink-2 transition-colors hover:text-ink" href={`/approvals/?site=${c.site}`}>Review <ChevronRight className="size-3.5" aria-hidden /></Link>}
              </div>
            ))}
          </Card>
        </section>
      )}
      {fields && (
        <section>
          <SectionTitle icon={FilePen} hint="as proposed">Copy</SectionTitle>
          <Card className="divide-y divide-line text-[13px]" data-testid="audit-copy-fields">
            {Object.entries(fields).map(([name, v]) => (
              <div key={name} className="grid grid-cols-[110px_1fr] gap-4 px-4 py-3 sm:grid-cols-[160px_1fr] sm:px-5">
                <span className="font-mono text-[12px] text-muted">{name}</span>
                <span className="min-w-0 break-words"><FieldView value={v} site={c.site} compact /></span>
              </div>
            ))}
          </Card>
        </section>
      )}
      <details className="group" open={!fields}>
        <summary className="flex cursor-pointer list-none items-center gap-1.5 rounded-sm text-[13px] font-medium text-ink-2 hover:text-ink [&::-webkit-details-marker]:hidden"><Braces className="size-4 text-muted" aria-hidden />Arguments as sent<ChevronDown className="size-3.5 text-muted transition-transform group-open:rotate-180" aria-hidden /></summary>
        <pre className="mt-3 overflow-x-auto rounded-lg border border-line bg-surface p-4 font-mono text-[12.5px] leading-relaxed whitespace-pre-wrap sm:p-5" data-testid="audit-arguments">{JSON.stringify(c.arguments, null, 2)}</pre>
      </details>
    </div>
  );
}

/** /audit with no id: the site's newest tool calls, each linking to its record. */
function AuditLog() {
  const param = useParam('site');
  const sites = useQuery({ queryKey: ['sites'], queryFn: () => api.get<SiteRow[]>('/sites'), enabled: !param });
  const site = param ?? (sites.data?.length === 1 ? sites.data[0].id : null);
  return (
    <Shell site={site} title="Audit log" help="Every tool call an agent makes leaves a record here, refused or not: who called what, for whom, why, and what it wrote.">
      {site ? (
        <div className="max-w-4xl"><McpTailCard site={site} connect /></div>
      ) : sites.isLoading ? (
        <Loading />
      ) : !sites.data?.length ? (
        <Empty>No sites yet, so no tool calls.</Empty>
      ) : (
        <Card className="max-w-xl divide-y divide-line text-[13px]">
          {sites.data.map((s) => (
            <Link key={s.id} href={`/audit/?site=${s.id}`} className="flex items-center gap-2 px-4 py-3 transition-colors hover:bg-surface-2/60">
              <Globe className="size-4 text-muted" aria-hidden /><span className="font-mono">{s.id}</span> <span className="text-muted">tool calls</span><ChevronRight className="ml-auto size-4 text-muted" aria-hidden />
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
    <Shell site={q.data?.site ?? null} title={`Tool call #${id}`} crumbs={q.data ? <Crumb href={`/audit/?site=${q.data.site}`}>Audit log</Crumb> : undefined}>
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
