'use client';
// The approval gate. Each draft takes one deliberate click: there is no bulk approve, no
// select-all, nothing that approves more than the card it sits on.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import Link from 'next/link';
import clsx from 'clsx';
import { Suspense, useEffect, useMemo, useState } from 'react';
import { AdvisorQueue, AdvisorSettingsPanel } from '@/components/advisor';
import { FieldView } from '@/components/assets';
import { Bot, Check, ChevronDown, Clock, Code2, CornerDownLeft, Crown, FilePen, FileText, History, MessageSquare, Percent, ShieldCheck, Split, UserRound, X } from 'lucide-react';
import { Preview } from '@/components/preview';
import { Shell, useActor } from '@/components/shell';
import { ShareSlider } from '@/components/slider';
import { Button, Card, Chip, Empty, Field, Help, Loading, SectionTitle, Tip, inputClass, type ChipTone } from '@/components/ui';
import { api, ApiError, type ApprovalCard, type Approvals, type Check as CheckRow, type ShareRequest } from '@/lib/api';
import { diff, lineDiff, looksLikeCode } from '@/lib/diff';
import { ago } from '@/lib/format';
import { useParam } from '@/lib/page';

const CHECK_TONE: Record<CheckRow['status'], [ChipTone, string]> = { pass: ['live', 'pass'], warn: ['warn', 'warn'], fail: ['paused', 'fail'], not_applicable: ['off', 'n/a'] };

/** Token-level for an inline payload, line-level (a code diff) for a component or any source. */
function Diff({ before, after }: { before: string | null; after: string }) {
  const [mode, setMode] = useState<'words' | 'code'>(() => (looksLikeCode(after) ? 'code' : 'words'));
  const parts = useMemo(() => diff(before ?? '', after), [before, after]);
  const lines = useMemo(() => lineDiff(before ?? '', after), [before, after]);
  if (before === null) return <p className="text-xs text-muted">All new: the champion is the page&apos;s own content.</p>;
  const added = lines.filter((l) => l.kind === 'add').length;
  const removed = lines.filter((l) => l.kind === 'del').length;
  return (
    <div>
      <div className="mb-1.5 flex items-center gap-3">
        <div role="tablist" aria-label="Diff mode" className="inline-flex rounded-md bg-surface-2 p-0.5 text-[12px]">
          {(['words', 'code'] as const).map((m) => (
            <button key={m} role="tab" aria-selected={mode === m} onClick={() => setMode(m)} className={clsx('h-6 rounded-[5px] px-2.5 transition-colors duration-150', mode === m ? 'bg-surface font-medium text-ink shadow-[0_0_0_1px_var(--line),0_1px_2px_rgb(0_0_0/0.06)]' : 'text-muted hover:text-ink')}>
              {m === 'words' ? 'Words' : 'Code'}
            </button>
          ))}
        </div>
        {mode === 'code' && <span className="text-xs text-muted tabular"><span className="text-good-ink">+{added}</span> <span className="text-pause-ink">−{removed}</span> lines</span>}
      </div>
      {mode === 'words' ? (
        <pre className="max-h-48 overflow-auto rounded-md border border-line bg-paper p-3 font-mono text-[12px] leading-relaxed whitespace-pre-wrap" data-testid="diff">
          {parts.map((p, i) =>
            p.kind === 'same' ? <span key={i}>{p.text}</span> : p.kind === 'add' ? <ins key={i} className="rounded-sm bg-good-soft text-good-ink no-underline">{p.text}</ins> : <del key={i} className="rounded-sm bg-pause-soft text-pause-ink">{p.text}</del>,
          )}
        </pre>
      ) : (
        <div className="max-h-72 overflow-auto rounded-md border border-line bg-paper py-1.5 font-mono text-[12px] leading-relaxed" data-testid="code-diff">
          <table className="w-full border-collapse">
            <tbody>
              {lines.map((l, i) => (
                <tr key={i} data-kind={l.kind} className={l.kind === 'add' ? 'bg-good-soft' : l.kind === 'del' ? 'bg-pause-soft' : ''}>
                  <td className="w-8 pr-1 text-right text-muted select-none tabular">{l.a ?? ''}</td>
                  <td className="w-8 pr-2 text-right text-muted select-none tabular">{l.b ?? ''}</td>
                  <td className={clsx('w-4 select-none', l.kind === 'add' ? 'text-good-ink' : l.kind === 'del' ? 'text-pause-ink' : 'text-muted')}>{l.kind === 'add' ? '+' : l.kind === 'del' ? '−' : ''}</td>
                  <td className={clsx('pr-3 whitespace-pre-wrap', l.kind === 'del' && 'text-pause-ink line-through decoration-1')}>{l.text}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

/** Who wrote a draft over MCP, why, and where the tool call is recorded. */
/** A copy slot's draft beside its champion, field by field: text as text, images as images (F4, S11). */
function CopyFields({ card, site }: { card: ApprovalCard; site: string }) {
  const own = card.champion?.is_baseline;
  return (
    <table className="w-full text-[13px]" data-testid="copy-fields">
      <thead>
        <tr className="text-left text-[12px] text-muted">
          <th className="py-1 pr-3 font-normal">Field</th>
          <th className="py-1 pr-3 font-normal">Champion <span className="font-mono">{card.champion?.key ?? '—'}</span></th>
          <th className="py-1 font-normal">Draft <span className="font-mono">{card.key}</span></th>
        </tr>
      </thead>
      <tbody>
        {(card.fields ?? []).map((f) => (
          <tr key={f.name} className="border-t border-line align-top">
            <td className="py-2 pr-3 font-mono text-[12px] text-muted">{f.name}</td>
            <td className="w-[40%] py-2 pr-3 text-ink-2">{own ? 'the page’s own' : <FieldView value={card.champion?.copy?.[f.name]} site={site} compact />}</td>
            <td className="w-[40%] py-2"><FieldView value={card.copy?.[f.name]} field={f} site={site} /></td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** A field's template as written, from the stored `{src, tpl}` (or the plain text). */
function sourceOf(v: unknown): string {
  if (typeof v === 'string') return v;
  if (v && typeof v === 'object' && typeof (v as { src?: unknown }).src === 'string') return (v as { src: string }).src;
  return '';
}

/** A templated copy draft as each preview persona sees it; the template itself behind "Source". */
function Renders({ card, site }: { card: ApprovalCard; site: string }) {
  const [open, setOpen] = useState(false);
  const fields = card.fields ?? [];
  const many = fields.length > 1;
  return (
    <div>
      <div className="mb-2 flex items-center gap-1 text-[12px] text-muted">
        <UserRound className="size-3.5" aria-hidden />As each preview visitor sees it
        <Help title="Filled for each visitor"><p>The draft is a template: each visitor sees it filled from their own order history. These are made-up visitors.</p>{card.requires?.length ? <p>A visitor without <code className="font-mono">{card.requires.join(', ')}</code> sees the page’s own instead.</p> : null}</Help>
      </div>
      <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-4" data-testid="renders">
        {(card.renders ?? []).map((r) => (
          <div key={r.persona} className="min-w-0 rounded-md border border-line bg-paper px-3 py-2.5" data-testid={`render-${r.persona}`}>
            <div className="mb-1 truncate text-[12px] text-muted">{r.label}</div>
            {r.fields ? (
              <dl className="space-y-1 text-[13px] text-ink">
                {fields.map((f) => (
                  <div key={f.name}>
                    {many && <dt className="font-mono text-[11px] text-muted">{f.name}</dt>}
                    <dd className="break-words"><FieldView value={r.fields?.[f.name]} site={site} compact /></dd>
                  </div>
                ))}
              </dl>
            ) : (
              <Tip label={`Needs ${card.requires?.join(', ') || 'a fact this visitor lacks'}`}>
                <span tabIndex={0} className="inline-flex items-center gap-1 text-[13px] text-ink-2">
                  <CornerDownLeft className="size-3.5 shrink-0" aria-hidden />Falls to <span className="font-mono text-[12px]">{r.falls_to}</span>
                </span>
              </Tip>
            )}
          </div>
        ))}
      </div>
      <button type="button" aria-expanded={open} onClick={() => setOpen(!open)} className="mt-2 inline-flex h-7 items-center gap-1 rounded-md px-1.5 text-[12px] text-muted transition-colors hover:bg-surface-2 hover:text-ink">
        <Code2 className="size-3.5" aria-hidden />Source<ChevronDown className={clsx('size-3.5 transition-transform duration-150', open && 'rotate-180')} aria-hidden />
      </button>
      {open && (
        <dl className="mt-1 space-y-1.5 rounded-md border border-line bg-paper p-3 font-mono text-[12px] text-ink-2" data-testid="template-source">
          {fields.map((f) => (
            <div key={f.name} className="flex gap-3">
              <dt className="w-20 shrink-0 text-muted">{f.name}</dt>
              <dd className="min-w-0 break-words">{sourceOf(card.copy?.[f.name]) || '—'}</dd>
            </div>
          ))}
        </dl>
      )}
    </div>
  );
}

function AgentBadge({ agent }: { agent: NonNullable<ApprovalCard['agent']> }) {
  return (
    <div className="mb-5 border-l-2 border-line-strong pl-3 text-[13px]" data-testid="agent-badge">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <span className="inline-flex items-center gap-1.5 font-medium text-ink"><Bot className="size-4 text-ink-2" aria-hidden />{agent.name}</span>
        <Tip label={<>Token <span className="font-mono">{agent.token}</span>, over MCP</>}>
          <span tabIndex={0} className="rounded-sm text-muted">{agent.on_behalf_of ? <>for <span className="text-ink-2">{agent.on_behalf_of}</span>, </> : null}{ago(agent.at)}</span>
        </Tip>
        <Link href={`/audit/?id=${agent.audit_id}`} className="ml-auto inline-flex items-center gap-1 rounded-sm text-[12px] text-muted transition-colors hover:text-ink" data-testid="audit-link"><History className="size-3.5" aria-hidden />Tool call #{agent.audit_id}</Link>
      </div>
      <p className="mt-1 text-ink-2"><span data-testid="agent-justification">“{agent.justification}”</span></p>
    </div>
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
    <Card data-testid={`card-${card.key}`}>
      <div className="flex flex-wrap items-center gap-2 border-b border-line px-5 py-3">
        <span className="text-[14px] font-semibold">{card.key}</span>
        <span className="text-[13px] text-muted" title="slot">on <span className="text-ink-2">{card.slot}</span></span>
        <Chip tone="warn" icon={Clock}>review</Chip>
        <span className="ml-auto inline-flex items-center gap-1 text-[12px] text-muted">{card.agent ? <Bot className="size-3.5" aria-label="agent" /> : <UserRound className="size-3.5" aria-label={card.author === 'human' ? 'person' : card.author} />}{ago(card.created_at)}</span>
      </div>
      <div className="p-5">
      {card.agent && <AgentBadge agent={card.agent} />}
      {card.renders?.length ? <Renders card={card} site={site} /> : card.fields ? <CopyFields card={card} site={site} /> : <>
      <div className="grid gap-4 lg:grid-cols-2">
        <div>
          <div className="mb-1.5 inline-flex items-center gap-1 text-[12px] text-muted"><Crown className="size-3.5" aria-hidden />Champion: <span className="font-mono">{card.champion?.key ?? '—'}</span></div>
          <Preview html={card.champion?.is_baseline ? null : (card.champion?.payload ?? null)} label={`champion ${card.champion?.key}`} />
        </div>
        <div>
          <div className="mb-1.5 inline-flex items-center gap-1 text-[12px] text-muted"><FileText className="size-3.5" aria-hidden />Draft: <span className="font-mono">{card.key}</span></div>
          <Preview html={card.html ?? ''} label={`draft ${card.key}`} />
        </div>
      </div>
      <div className="mt-5">
        <div className="mb-1.5 inline-flex items-center gap-1 text-[12px] text-muted"><Split className="size-3.5" aria-hidden />What changed</div>
        <Diff before={card.champion?.is_baseline ? null : (card.champion?.payload ?? null)} after={card.html ?? ''} />
      </div>
      </>}
      <div className="mt-5 grid grid-cols-1 gap-2 sm:grid-cols-2" data-testid="checks">
        {card.checks.map((c) => (
          <div key={c.name} className="flex items-start gap-2 text-[12.5px]">
            <Chip tone={CHECK_TONE[c.status][0]} title={c.status === 'pass' || c.status === 'not_applicable' ? c.reason : undefined}>{c.name} · {CHECK_TONE[c.status][1]}</Chip>
            {(c.status === 'fail' || c.status === 'warn') && <span className="pt-0.5 text-ink-2">{c.reason}</span>}
          </div>
        ))}
      </div>
      {card.comments.length > 0 && (
        <ul className="mt-5 space-y-1.5 border-t border-line pt-4 text-[13px]">
          {card.comments.map((c, i) => (
            <li key={i} className="flex gap-1.5"><MessageSquare className="mt-0.5 size-3.5 shrink-0 text-muted" aria-hidden /><span><b className="font-medium">{c.actor}</b> <span className="text-muted">{c.action === 'comment' ? 'commented' : c.action} {ago(c.at)}:</span> {c.comment}</span></li>
          ))}
        </ul>
      )}
      <div className="mt-5 grid gap-5 border-t border-line pt-5 md:grid-cols-[1fr_1fr]">
        <div>
          <div className="mb-3 flex items-center justify-between text-[13px]">
            <span className="inline-flex items-center gap-0.5 font-medium text-ink-2">Planned traffic share<Help title="Planned traffic share"><p>The most this variant can reach at first, capped at {Math.round(max * 100)}%. The bandit may show it less.</p></Help></span>
            <span className="tabular font-semibold text-ink" data-testid="share">{share}%</span>
          </div>
          <ShareSlider value={share} onChange={setShare} max={Math.round(max * 100)} label={`planned traffic share for ${card.key}`} />
        </div>
        <Field label="Comment (stays on the variant)">
          <textarea className={`${inputClass} h-[68px] w-full resize-none py-2 max-sm:h-[68px]`} value={comment} onChange={(e) => setComment(e.target.value)} placeholder="Why approve or reject…" />
        </Field>
      </div>
      {error && <p role="alert" className="mt-3 text-[13px] text-pause-ink">{error}</p>}
      </div>
      <div className="flex flex-wrap items-center gap-2 rounded-b-lg border-t border-line bg-paper/60 px-5 py-3">
        <Button icon={Check} onClick={() => approve.mutate()} disabled={!approver.trim() || blocked || approve.isPending}>
          Approve {card.key} at {share}%
        </Button>
        <Button variant="danger" icon={X} onClick={() => reject.mutate()} disabled={!approver.trim() || !comment.trim() || reject.isPending}>Reject</Button>
        <Button variant="ghost" icon={MessageSquare} onClick={() => remark.mutate()} disabled={!approver.trim() || !comment.trim()}>Comment</Button>
        {blocked && <span className="text-[12px] text-pause-ink">A failing check blocks approval.</span>}
        {!approver.trim() && <span className="text-[12px] text-muted">Add your name at the top to decide.</span>}
      </div>
    </Card>
  );
}

const pctOf = (s: number | null) => (s === null ? 'uncapped' : `${Math.round(s * 100)}%`);

function ShareProposal({ r, site, approver }: { r: ShareRequest; site: string; approver: string }) {
  const qc = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  const answer = useMutation({
    mutationFn: (verb: 'confirm' | 'decline') => api.post(`/share-requests/${r.id}/${verb}`, { actor: approver }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['approvals', site] }),
    onError: (e) => setError(e instanceof ApiError ? e.message : 'failed'),
  });
  return (
    <div className="flex flex-wrap items-center gap-2 px-4 py-3 text-[13px]" data-testid={`share-request-${r.variant}`}>
      <span className="font-medium">{r.variant}</span>
      <span className="text-muted">on {r.slot}: {pctOf(r.current)} → <b className="text-ink">{pctOf(r.traffic_share)}</b>, asked by {r.agent.name}</span>
      <span className="text-ink-2">“{r.reason}”</span>
      <span className="ml-auto flex gap-2">
        <Button icon={Check} onClick={() => answer.mutate('confirm')} disabled={!approver.trim() || answer.isPending}>Accept</Button>
        <Button variant="ghost" icon={X} onClick={() => answer.mutate('decline')} disabled={!approver.trim() || answer.isPending}>Decline</Button>
      </span>
      {error && <p role="alert" className="w-full text-[12.5px] text-pause-ink">{error}</p>}
    </div>
  );
}

const RECENT: Record<string, [ChipTone, string]> = { approve: ['live', 'approved'], share: ['accent', 'share changed'], retire: ['off', 'retired'], keep: ['off', 'kept'], pause: ['paused', 'paused'] };

function View() {
  const site = useParam('site')!;
  const [approver] = useActor();
  const q = useQuery({ queryKey: ['approvals', site], queryFn: () => api.get<Approvals>(`/sites/${site}/approvals`) });
  if (q.isLoading || !q.data) return <Loading />;
  return (
    <div className="max-w-5xl space-y-12">
      <AdvisorQueue cards={q.data.cards ?? []} site={site} approver={approver} />
      <section>
      <SectionTitle hint={`${q.data.drafts.length} waiting`}>Drafts</SectionTitle>
      {q.data.drafts.length === 0 ? <Empty icon={ShieldCheck}>Nothing waiting. <Link className="font-medium text-accent hover:underline" href={`/drafts/?site=${site}`}>Write a draft</Link></Empty> : <div className="space-y-6">{q.data.drafts.map((c) => <DraftCard key={c.id} card={c} site={site} approver={approver} max={q.data.max_share} />)}</div>}
      </section>
      {(q.data.share_requests ?? []).length > 0 && (
        <section>
          <SectionTitle icon={Percent} help="An agent asked to change a live variant's traffic share. Nothing changes until you accept." hint={`${q.data.share_requests.length} waiting`}>Proposed share changes</SectionTitle>
          <Card className="divide-y divide-line">{q.data.share_requests.map((r) => <ShareProposal key={r.id} r={r} site={site} approver={approver} />)}</Card>
        </section>
      )}
      {q.data.recent.length > 0 && (
        <section>
          <SectionTitle icon={History}>Recent decisions</SectionTitle>
          <Card className="divide-y divide-line text-[13px]">
            {q.data.recent.map((r, i) => (
              <div key={i} className="flex flex-wrap items-center gap-2 px-4 py-3">
                <Chip tone={(RECENT[r.action] ?? ['off', 'rejected'])[0]}>{(RECENT[r.action] ?? ['off', 'rejected'])[1]}</Chip>
                <span className="font-medium">{r.variant}</span>
                <span className="text-muted">on {r.slot} by {r.actor}{r.traffic_share !== null ? ` at ${Math.round(r.traffic_share * 100)}%` : ''}</span>
                {r.comment && <span className="text-ink-2">“{r.comment}”</span>}
                <span className="ml-auto text-xs text-muted">{ago(r.at)}</span>
              </div>
            ))}
          </Card>
        </section>
      )}
      <AdvisorSettingsPanel site={site} />
    </div>
  );
}

function Page() {
  const site = useParam('site');
  return (
    <Shell site={site} title="Review" help={<><p>Each draft takes its own deliberate click. There is no bulk approve.</p><p>Read the diff and the checks; your decision is recorded with your name.</p></>}>
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
