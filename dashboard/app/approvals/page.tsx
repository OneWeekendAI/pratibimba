'use client';
// The approval gate. Each draft takes one deliberate click: there is no bulk approve, no
// select-all, nothing that approves more than the card it sits on.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Suspense, useEffect, useMemo, useState } from 'react';
import { Preview } from '@/components/preview';
import { Shell } from '@/components/shell';
import { ShareSlider } from '@/components/slider';
import { Button, Card, Chip, Empty, Field, Loading, SectionTitle, inputClass, type ChipTone } from '@/components/ui';
import { api, ApiError, type ApprovalCard, type Approvals, type Check, type Overview } from '@/lib/api';
import { diff } from '@/lib/diff';
import { ago } from '@/lib/format';
import { useParam } from '@/lib/page';

const CHECK_TONE: Record<Check['status'], [ChipTone, string]> = { pass: ['live', 'pass'], warn: ['warn', 'warn'], fail: ['paused', 'fail'], not_applicable: ['off', 'n/a'] };

function useApprover(): [string, (v: string) => void] {
  const [name, setName] = useState('');
  useEffect(() => {
    try {
      setName(localStorage.getItem('prb-approver') ?? '');
    } catch {}
  }, []);
  return [
    name,
    (v) => {
      setName(v);
      try {
        localStorage.setItem('prb-approver', v);
      } catch {}
    },
  ];
}

function Diff({ before, after }: { before: string | null; after: string }) {
  const parts = useMemo(() => diff(before ?? '', after), [before, after]);
  if (before === null) return <p className="text-xs text-muted">The champion is the baseline, which lives in your code, so there is nothing to diff against; the whole draft is new.</p>;
  return (
    <pre className="max-h-48 overflow-auto rounded-md border border-line bg-surface-2/50 p-3 font-mono text-[12px] leading-relaxed whitespace-pre-wrap" data-testid="diff">
      {parts.map((p, i) =>
        p.kind === 'same' ? <span key={i}>{p.text}</span> : p.kind === 'add' ? <ins key={i} className="bg-[color:var(--accent-soft)] text-ink no-underline">{p.text}</ins> : <del key={i} className="text-pause-ink">{p.text}</del>,
      )}
    </pre>
  );
}

function DraftCard({ card, site, approver, max }: { card: ApprovalCard; site: string; approver: string; max: number }) {
  const qc = useQueryClient();
  const [share, setShare] = useState(10);
  const [comment, setComment] = useState('');
  const [error, setError] = useState<string | null>(null);
  const blocked = card.checks.some((c) => c.status === 'fail');
  const done = () => {
    qc.invalidateQueries({ queryKey: ['approvals', site] });
    qc.invalidateQueries({ queryKey: ['sites'] });
  };
  const approve = useMutation({
    mutationFn: () => api.post(`/variants/${card.id}/approve`, { approver, traffic_share: share / 100, comment: comment.trim() || undefined }),
    onSuccess: done,
    onError: (e) => setError(e instanceof ApiError ? e.message : 'failed'),
  });
  const reject = useMutation({
    mutationFn: () => api.post(`/variants/${card.id}/reject`, { actor: approver, comment }),
    onSuccess: done,
    onError: (e) => setError(e instanceof ApiError ? e.message : 'failed'),
  });
  const remark = useMutation({
    mutationFn: () => api.post(`/variants/${card.id}/comment`, { actor: approver, comment }),
    onSuccess: () => {
      setComment('');
      done();
    },
  });
  return (
    <Card className="p-5" data-testid={`card-${card.key}`}>
      <div className="mb-4 flex flex-wrap items-center gap-2">
        <span className="font-mono text-[15px] font-semibold">{card.key}</span>
        <span className="text-xs text-muted">for slot <span className="font-mono">{card.slot}</span></span>
        <Chip tone="warn">review</Chip>
        <span className="text-xs text-muted">written by a {card.author === 'human' ? 'person' : card.author} · {ago(card.created_at)}</span>
        <span className="ml-auto text-xs text-muted">audience: everyone</span>
      </div>
      <div className="grid gap-4 lg:grid-cols-2">
        <div>
          <div className="mb-1 text-xs text-muted">Current champion: <span className="font-mono">{card.champion?.key ?? '—'}</span></div>
          <Preview html={card.champion?.is_baseline ? null : (card.champion?.payload ?? null)} label={`champion ${card.champion?.key}`} />
        </div>
        <div>
          <div className="mb-1 text-xs text-muted">Draft: <span className="font-mono">{card.key}</span></div>
          <Preview html={card.html ?? ''} label={`draft ${card.key}`} />
        </div>
      </div>
      <div className="mt-4">
        <div className="mb-1 text-xs text-muted">What changed</div>
        <Diff before={card.champion?.is_baseline ? null : (card.champion?.payload ?? null)} after={card.html ?? ''} />
      </div>
      <div className="mt-4 grid grid-cols-1 gap-1.5 sm:grid-cols-2" data-testid="checks">
        {card.checks.map((c) => (
          <div key={c.name} className="flex items-start gap-2 text-[12.5px]">
            <Chip tone={CHECK_TONE[c.status][0]}>{c.name} · {CHECK_TONE[c.status][1]}</Chip>
            <span className="pt-0.5 text-ink-2">{c.reason}</span>
          </div>
        ))}
      </div>
      {card.comments.length > 0 && (
        <ul className="mt-4 space-y-1 border-t border-line pt-3 text-[12.5px]">
          {card.comments.map((c, i) => (
            <li key={i}><b className="font-medium">{c.actor}</b> <span className="text-muted">{c.action === 'comment' ? 'commented' : c.action} {ago(c.at)}:</span> {c.comment}</li>
          ))}
        </ul>
      )}
      <div className="mt-5 grid gap-4 border-t border-line pt-4 md:grid-cols-[1fr_1fr]">
        <div>
          <div className="mb-2 flex items-baseline justify-between text-xs">
            <span className="font-medium text-ink-2">Planned traffic share</span>
            <span className="tabular font-semibold" data-testid="share">{share}%</span>
          </div>
          <ShareSlider value={share} onChange={setShare} max={Math.round(max * 100)} label={`planned traffic share for ${card.key}`} />
          <p className="mt-2 text-xs text-muted">The most this variant can reach at first; hard-capped at {Math.round(max * 100)}%. The bandit may show it less.</p>
        </div>
        <Field label="Comment (stays on the variant)">
          <textarea className={`${inputClass} w-full h-[68px] py-2`} value={comment} onChange={(e) => setComment(e.target.value)} placeholder="Why approve or reject?" />
        </Field>
      </div>
      {error && <p role="alert" className="mt-3 text-[13px] text-pause-ink">{error}</p>}
      <div className="mt-4 flex flex-wrap items-center gap-2">
        <Button onClick={() => approve.mutate()} disabled={!approver.trim() || blocked || approve.isPending}>
          Approve {card.key} at {share}%
        </Button>
        <Button variant="danger" onClick={() => reject.mutate()} disabled={!approver.trim() || !comment.trim() || reject.isPending}>Reject</Button>
        <Button variant="ghost" onClick={() => remark.mutate()} disabled={!approver.trim() || !comment.trim()}>Comment</Button>
        {blocked && <span className="text-xs text-pause-ink">A failing check blocks approval.</span>}
        {!approver.trim() && <span className="text-xs text-muted">Enter your name above to review.</span>}
      </div>
    </Card>
  );
}

function NewDraft({ site, slots, initial }: { site: string; slots: string[]; initial: string | null }) {
  const qc = useQueryClient();
  const [slot, setSlot] = useState(initial ?? slots[0] ?? '');
  const [key, setKey] = useState('');
  const [html, setHtml] = useState('');
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!slot && slots[0]) setSlot(slots[0]);
  }, [slots, slot]);
  const create = useMutation({
    mutationFn: () => api.post(`/sites/${site}/slots/${slot}/variants`, { key, html }),
    onSuccess: () => {
      setKey('');
      setHtml('');
      setError(null);
      qc.invalidateQueries({ queryKey: ['approvals', site] });
      qc.invalidateQueries({ queryKey: ['sites'] });
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'failed'),
  });
  return (
    <Card className="p-5">
      <SectionTitle hint="a draft is never shown until someone approves it here">Write a draft</SectionTitle>
      <form
        className="grid gap-3 md:grid-cols-[180px_220px_1fr]"
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate();
        }}
      >
        <Field label="Slot">
          <select className={`${inputClass} w-full w-full`} value={slot} onChange={(e) => setSlot(e.target.value)}>{slots.map((s) => <option key={s}>{s}</option>)}</select>
        </Field>
        <Field label="Key" hint="lowercase, e.g. gift_v2">
          <input className={`${inputClass} w-full font-mono`} value={key} onChange={(e) => setKey(e.target.value)} placeholder="new_variant" />
        </Field>
        <Field label="HTML">
          <textarea className={`${inputClass} w-full h-24 py-2 font-mono text-[12.5px]`} value={html} onChange={(e) => setHtml(e.target.value)} placeholder="<h1>…</h1>" />
        </Field>
        <div className="md:col-span-3 flex items-center gap-3">
          <Button type="submit" variant="ghost" disabled={!key || !html || !slot || create.isPending}>Save as draft</Button>
          {error && <span role="alert" className="text-[13px] text-pause-ink">{error}</span>}
        </div>
      </form>
    </Card>
  );
}

function View() {
  const site = useParam('site')!;
  const initialSlot = useParam('slot');
  const [approver, setApprover] = useApprover();
  const q = useQuery({ queryKey: ['approvals', site], queryFn: () => api.get<Approvals>(`/sites/${site}/approvals`) });
  const ov = useQuery({ queryKey: ['overview', site], queryFn: () => api.get<Overview>(`/sites/${site}/overview`) });
  if (q.isLoading || !q.data) return <Loading />;
  const slots = ov.data?.slots.map((s) => s.key) ?? [];
  return (
    <div className="max-w-5xl space-y-6">
      <Card className="flex flex-wrap items-end gap-4 p-4">
        <div className="w-64">
          <Field label="You are reviewing as">
            <input className={`${inputClass} w-full w-full`} value={approver} onChange={(e) => setApprover(e.target.value)} placeholder="Your name" aria-label="Reviewer name" />
          </Field>
        </div>
        <p className="flex-1 text-xs text-muted">Approvals are the one place this dashboard refuses to be fast. Read the diff and the checks; every draft gets its own decision and is recorded with your name.</p>
      </Card>
      <SectionTitle hint={`${q.data.drafts.length} waiting`}>Drafts</SectionTitle>
      {q.data.drafts.length === 0 ? <Empty>Nothing waiting. Drafts you or your team write land here.</Empty> : q.data.drafts.map((c) => <DraftCard key={c.id} card={c} site={site} approver={approver} max={q.data.max_share} />)}
      <NewDraft site={site} slots={slots} initial={initialSlot} />
      {q.data.recent.length > 0 && (
        <section>
          <SectionTitle>Recent decisions</SectionTitle>
          <Card className="divide-y divide-line text-[13px]">
            {q.data.recent.map((r, i) => (
              <div key={i} className="flex flex-wrap items-center gap-2 px-4 py-2.5">
                <Chip tone={r.action === 'approve' ? 'live' : 'off'}>{r.action === 'approve' ? 'approved' : 'rejected'}</Chip>
                <span className="font-mono">{r.variant}</span>
                <span className="text-muted">on {r.slot} by {r.actor}{r.traffic_share !== null ? ` at ${Math.round(r.traffic_share * 100)}%` : ''}</span>
                {r.comment && <span className="text-ink-2">“{r.comment}”</span>}
                <span className="ml-auto text-xs text-muted">{ago(r.at)}</span>
              </div>
            ))}
          </Card>
        </section>
      )}
    </div>
  );
}

function Page() {
  const site = useParam('site');
  return (
    <Shell site={site} title={<><div className="text-xs text-muted">{site}</div><h1 className="font-serif text-3xl">Approvals</h1></>}>
      {site ? <View /> : <Empty>No site given.</Empty>}
    </Shell>
  );
}

export default function ApprovalsPage() {
  return (
    <Suspense>
      <Page />
    </Suspense>
  );
}
