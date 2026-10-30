'use client';
// Webhooks out: add an endpoint, see whether deliveries land, remove it. The secret is shown
// once, when the endpoint is added.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Suspense, useState } from 'react';
import { Shell } from '@/components/shell';
import { Check, Copy, KeyRound, Link2, List, Plus, Trash2, Webhook } from 'lucide-react';
import { Button, Card, Chip, Empty, Loading, SectionTitle, inputClass, type ChipTone } from '@/components/ui';
import { api, ApiError, type WebhookCreated, type WebhookDelivery, type WebhookEvent, type WebhookList, type WebhookSubscription } from '@/lib/api';
import { ago } from '@/lib/format';
import { useParam } from '@/lib/page';

const STATUS: Record<WebhookDelivery['status'], ChipTone> = { delivered: 'live', pending: 'warn', failed: 'paused' };

function AddEndpoint({ site, types, onCreated }: { site: string; types: WebhookEvent[]; onCreated: (c: WebhookCreated) => void }) {
  const qc = useQueryClient();
  const [url, setUrl] = useState('');
  const [events, setEvents] = useState<WebhookEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const add = useMutation({
    mutationFn: () => api.post<WebhookCreated>(`/sites/${site}/webhooks`, { url: url.trim(), events }),
    onSuccess: (c) => {
      setError(null);
      setUrl('');
      setEvents([]);
      onCreated(c);
      qc.invalidateQueries({ queryKey: ['webhooks', site] });
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'failed'),
  });
  const toggle = (e: WebhookEvent) => setEvents((xs) => (xs.includes(e) ? xs.filter((x) => x !== e) : [...xs, e]));
  return (
    <Card className="space-y-4 p-5" data-testid="webhook-add">
      <label className="relative flex items-center">
        <Link2 className="pointer-events-none absolute left-2.5 size-4 text-muted" aria-hidden />
        <input className={`${inputClass} w-full pl-8 font-mono`} placeholder="https://example.com/pratibimba" aria-label="Endpoint URL" value={url} onChange={(e) => setUrl(e.target.value)} />
      </label>
      <fieldset className="flex flex-wrap gap-x-5 gap-y-2">
        <legend className="sr-only">Events</legend>
        {types.map((t) => (
          <label key={t} className="inline-flex cursor-pointer items-center gap-2 font-mono text-[12.5px] text-ink-2">
            <input type="checkbox" checked={events.includes(t)} onChange={() => toggle(t)} className="size-3.5 accent-[color:var(--primary)]" />
            {t}
          </label>
        ))}
      </fieldset>
      <div className="flex items-center gap-3 border-t border-line pt-4">
        <Button icon={Plus} disabled={!url.trim() || events.length === 0 || add.isPending} onClick={() => add.mutate()}>
          Add endpoint
        </Button>
        {error && <span role="alert" className="text-xs text-pause-ink">{error}</span>}
      </div>
    </Card>
  );
}

function Secret({ c, onDone }: { c: WebhookCreated; onDone: () => void }) {
  const [copied, setCopied] = useState(false);
  return (
    <Card className="space-y-3 bg-warn-soft p-4" data-testid="webhook-secret">
      <div className="flex items-center gap-1.5 text-[13px] font-medium"><KeyRound className="size-4 text-warn-ink" aria-hidden />Signing secret for {c.url}. Copy it now; it is not shown again.</div>
      <div className="flex flex-wrap items-center gap-2">
        <code className="min-w-0 flex-1 truncate rounded-md border border-line bg-surface px-2.5 py-1.5 font-mono text-[12px]" data-testid="webhook-secret-value">{c.secret}</code>
        <Button
          variant="ghost"
          icon={copied ? Check : Copy}
          onClick={async () => {
            try {
              await navigator.clipboard.writeText(c.secret);
              setCopied(true);
            } catch {}
          }}
        >
          {copied ? 'Copied' : 'Copy'}
        </Button>
        <Button variant="ghost" onClick={onDone}>Done</Button>
      </div>
    </Card>
  );
}

function Deliveries({ id }: { id: number }) {
  const q = useQuery({ queryKey: ['webhook-deliveries', id], queryFn: () => api.get<{ deliveries: WebhookDelivery[] }>(`/webhooks/${id}/deliveries`), refetchInterval: 5000 });
  if (!q.data) return <Loading />;
  if (q.data.deliveries.length === 0) return <div className="px-4 py-4 text-[13px] text-muted">Nothing sent yet.</div>;
  return (
    <table className="w-full text-[12.5px]" data-testid="webhook-deliveries">
      <thead className="border-b border-line text-left text-[12px] text-muted">
        <tr>
          <th className="h-9 px-4 font-normal">Event</th>
          <th className="h-9 px-4 font-normal">Status</th>
          <th className="h-9 px-4 font-normal">Attempts</th>
          <th className="h-9 px-4 font-normal">Response</th>
          <th className="h-9 px-4 font-normal">Created</th>
        </tr>
      </thead>
      <tbody className="divide-y divide-line">
        {q.data.deliveries.map((d) => (
          <tr key={d.id}>
            <td className="h-10 px-4 font-mono">{d.event}</td>
            <td className="h-10 px-4"><Chip tone={STATUS[d.status]}>{d.status}</Chip></td>
            <td className="h-10 px-4 tabular-nums">{d.attempts}</td>
            <td className="h-10 px-4 text-ink-2">{d.last_status ?? d.last_error ?? '–'}</td>
            <td className="h-10 px-4 text-muted">{ago(d.created_at)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function Endpoint({ s, site }: { s: WebhookSubscription; site: string }) {
  const qc = useQueryClient();
  const [open, setOpen] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const remove = useMutation({
    mutationFn: () => api.del(`/webhooks/${s.id}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['webhooks', site] }),
  });
  return (
    <div data-testid={`webhook-${s.id}`}>
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3.5">
        <div className="min-w-0 flex-1">
          <div className="truncate font-mono text-[13px]">{s.url}</div>
          <div className="mt-1 flex flex-wrap gap-1">{s.events.map((e) => <span key={e} className="rounded bg-surface-2 px-1.5 py-px font-mono text-[11.5px] text-ink-2">{e}</span>)}</div>
        </div>
        <div className="text-[12.5px] text-muted tabular-nums" data-testid="webhook-counts">
          {s.counts.delivered} delivered, {s.counts.pending} pending
          {s.counts.failed > 0 && <span className="text-pause-ink">, {s.counts.failed} failed</span>}
        </div>
        <Button variant="ghost" icon={List} onClick={() => setOpen((o) => !o)} aria-expanded={open}>
          {open ? 'Hide deliveries' : 'Deliveries'}
        </Button>
        <Button variant="danger" icon={Trash2} disabled={remove.isPending} onClick={() => (confirming ? remove.mutate() : setConfirming(true))} onBlur={() => setConfirming(false)}>
          {confirming ? 'Confirm delete' : 'Delete'}
        </Button>
      </div>
      {open && <div className="overflow-x-auto border-t border-line bg-paper/60"><Deliveries id={s.id} /></div>}
    </div>
  );
}

function Page() {
  const site = useParam('site');
  const [created, setCreated] = useState<WebhookCreated | null>(null);
  const q = useQuery({ queryKey: ['webhooks', site], queryFn: () => api.get<WebhookList>(`/sites/${site}/webhooks`), enabled: !!site, refetchInterval: 5000 });
  return (
    <Shell site={site} title="Webhooks" help="Pratibimba posts to these URLs when something changes. Each request is signed with HMAC-SHA256 using the endpoint's secret.">
      {!site ? (
        <Empty>No site given.</Empty>
      ) : !q.data ? (
        <Loading />
      ) : (
        <div className="max-w-4xl space-y-12">
          <section className="space-y-3">
            <SectionTitle icon={Plus}>Add an endpoint</SectionTitle>
            {created && <Secret c={created} onDone={() => setCreated(null)} />}
            <AddEndpoint site={site} types={q.data.event_types} onCreated={setCreated} />
          </section>
          <section>
            <SectionTitle icon={Webhook}>Endpoints</SectionTitle>
            {q.data.subscriptions.length === 0 ? (
              <Empty icon={Webhook}>No endpoints yet.</Empty>
            ) : (
              <Card className="divide-y divide-line">
                {q.data.subscriptions.map((s) => <Endpoint key={s.id} s={s} site={site} />)}
              </Card>
            )}
          </section>
        </div>
      )}
    </Shell>
  );
}

export default function WebhooksPage() {
  return (
    <Suspense>
      <Page />
    </Suspense>
  );
}
