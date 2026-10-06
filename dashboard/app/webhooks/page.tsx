'use client';
// Webhooks out: add an endpoint, see whether deliveries land, remove it. The secret is shown
// once, when the endpoint is added.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Suspense, useState } from 'react';
import { Shell } from '@/components/shell';
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
    <Card className="space-y-4 p-4" data-testid="webhook-add">
      <input className={`${inputClass} w-full font-mono`} placeholder="https://example.com/pratibimba" aria-label="Endpoint URL" value={url} onChange={(e) => setUrl(e.target.value)} />
      <fieldset className="flex flex-wrap gap-x-5 gap-y-2">
        <legend className="sr-only">Events</legend>
        {types.map((t) => (
          <label key={t} className="inline-flex items-center gap-2 font-mono text-[12.5px]">
            <input type="checkbox" checked={events.includes(t)} onChange={() => toggle(t)} className="accent-[color:var(--accent)]" />
            {t}
          </label>
        ))}
      </fieldset>
      <div className="flex items-center gap-3">
        <Button disabled={!url.trim() || events.length === 0 || add.isPending} onClick={() => add.mutate()}>
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
    <Card className="space-y-2 border-[color:var(--warn)] p-4" data-testid="webhook-secret">
      <div className="text-[13px] font-medium">Signing secret for {c.url}. Copy it now; it is not shown again.</div>
      <div className="flex items-center gap-2">
        <code className="min-w-0 flex-1 truncate rounded bg-surface-2 px-2 py-1.5 font-mono text-[12px]" data-testid="webhook-secret-value">{c.secret}</code>
        <Button
          variant="ghost"
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
  if (q.data.deliveries.length === 0) return <div className="px-4 py-3 text-[13px] text-muted">Nothing sent yet.</div>;
  return (
    <table className="w-full text-[12.5px]" data-testid="webhook-deliveries">
      <thead className="text-left text-[11px] tracking-wide text-muted uppercase">
        <tr>
          <th className="px-4 py-2 font-medium">Event</th>
          <th className="px-4 py-2 font-medium">Status</th>
          <th className="px-4 py-2 font-medium">Attempts</th>
          <th className="px-4 py-2 font-medium">Response</th>
          <th className="px-4 py-2 font-medium">Created</th>
        </tr>
      </thead>
      <tbody className="divide-y divide-line">
        {q.data.deliveries.map((d) => (
          <tr key={d.id}>
            <td className="px-4 py-2 font-mono">{d.event}</td>
            <td className="px-4 py-2"><Chip tone={STATUS[d.status]}>{d.status}</Chip></td>
            <td className="px-4 py-2 tabular-nums">{d.attempts}</td>
            <td className="px-4 py-2 text-ink-2">{d.last_status ?? d.last_error ?? '–'}</td>
            <td className="px-4 py-2 text-muted">{ago(d.created_at)}</td>
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
    <Card data-testid={`webhook-${s.id}`}>
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3">
        <div className="min-w-0 flex-1">
          <div className="truncate font-mono text-[13px]">{s.url}</div>
          <div className="mt-0.5 font-mono text-[11.5px] text-muted">{s.events.join(' · ')}</div>
        </div>
        <div className="text-[12.5px] text-ink-2 tabular-nums" data-testid="webhook-counts">
          {s.counts.delivered} delivered · {s.counts.pending} pending
          {s.counts.failed > 0 && <span className="text-pause-ink"> · {s.counts.failed} failed</span>}
        </div>
        <Button variant="ghost" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
          {open ? 'Hide deliveries' : 'Deliveries'}
        </Button>
        <Button variant="danger" disabled={remove.isPending} onClick={() => (confirming ? remove.mutate() : setConfirming(true))} onBlur={() => setConfirming(false)}>
          {confirming ? 'Confirm delete' : 'Delete'}
        </Button>
      </div>
      {open && <div className="border-t border-line"><Deliveries id={s.id} /></div>}
    </Card>
  );
}

function Page() {
  const site = useParam('site');
  const [created, setCreated] = useState<WebhookCreated | null>(null);
  const q = useQuery({ queryKey: ['webhooks', site], queryFn: () => api.get<WebhookList>(`/sites/${site}/webhooks`), enabled: !!site, refetchInterval: 5000 });
  return (
    <Shell site={site} title={<><div className="text-xs text-muted">{site}</div><h1 className="font-serif text-3xl">Webhooks</h1></>}>
      {!site ? (
        <Empty>No site given.</Empty>
      ) : !q.data ? (
        <Loading />
      ) : (
        <div className="max-w-4xl space-y-8">
          <section className="space-y-3">
            <SectionTitle hint="signed with HMAC-SHA256">Add an endpoint</SectionTitle>
            {created && <Secret c={created} onDone={() => setCreated(null)} />}
            <AddEndpoint site={site} types={q.data.event_types} onCreated={setCreated} />
          </section>
          <section>
            <SectionTitle>Endpoints</SectionTitle>
            {q.data.subscriptions.length === 0 ? (
              <Empty>No endpoints yet.</Empty>
            ) : (
              <div className="space-y-3">
                {q.data.subscriptions.map((s) => <Endpoint key={s.id} s={s} site={site} />)}
              </div>
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
