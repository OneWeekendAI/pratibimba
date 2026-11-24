'use client';
// A1, the advisor: its pause card in Review, its line on a Results slot, and its thresholds.
// The advisor never changes traffic itself; a person approves each card.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import clsx from 'clsx';
import Link from 'next/link';
import { useEffect, useId, useState } from 'react';
import { Check, ChevronDown, CircleCheck, CircleHelp, CirclePause, Lightbulb, RotateCcw, Save, ShieldCheck, X } from 'lucide-react';
import { Button, Card, Chip, Help, SectionTitle, Tip, inputClass } from '@/components/ui';
import { api, ApiError, type AdvisorArm, type AdvisorCard, type AdvisorLine, type AdvisorSettings } from '@/lib/api';
import { ago, int } from '@/lib/format';
import { human, unit } from '@/lib/plain';

/** A per-100 number with its sign: 1.44 -> "+1.4", -0.5 -> "−0.5". */
function signed(x: number) {
  const r = Math.round(x * 10) / 10;
  return `${r > 0 ? '+' : r < 0 ? '−' : '±'}${Math.abs(r).toFixed(1)}`;
}

/** "12 Oct". */
function day(iso: string) {
  return new Date(iso).toLocaleDateString('en-GB', { day: 'numeric', month: 'short' });
}

function useAdvisorSettings(site: string) {
  return useQuery({ queryKey: ['advisor-settings', site], queryFn: () => api.get<AdvisorSettings>(`/sites/${site}/advisor-settings`) });
}

function ArmRow({ a, held }: { a: AdvisorArm; held?: AdvisorArm }) {
  return (
    <tr className="border-t border-line">
      <td className="py-1.5 pr-3 font-mono text-[12px] text-ink-2">{a.key}</td>
      <td className="py-1.5 pr-3 tabular text-ink">{a.uplift === null ? '—' : <>{signed(a.uplift)} <span className="text-muted">({signed(a.lo!)} to {signed(a.hi!)})</span></>}</td>
      <td className="py-1.5 tabular text-ink-2">{held?.uplift == null ? '—' : signed(held.uplift)}</td>
    </tr>
  );
}

/** One pause card: the claim in plain words, the numbers behind "Details", one decision. */
function PauseCard({ card, site, approver, quiet, heldDays }: { card: AdvisorCard; site: string; approver: string; quiet: number; heldDays: number }) {
  const qc = useQueryClient();
  const [open, setOpen] = useState(false);
  const [reason, setReason] = useState('');
  const [error, setError] = useState<string | null>(null);
  const ev = card.evidence;
  const u = unit(card.metric);
  const done = () => {
    qc.invalidateQueries({ queryKey: ['approvals', site] });
    qc.invalidateQueries({ queryKey: ['results', site] });
  };
  const decide = useMutation({
    mutationFn: (verb: 'approve' | 'reject') => api.post(`/advisor-cards/${card.id}/${verb}`, { actor: approver, comment: reason.trim() || undefined }),
    onSuccess: done,
    onError: (e) => setError(e instanceof ApiError ? e.message : 'failed'),
  });
  const n = card.pauses.length;
  return (
    <Card data-testid={`advisor-card-${card.slot}`}>
      <div className="flex flex-wrap items-center gap-2 border-b border-line px-5 py-3">
        <CirclePause className="size-4 text-ink-2" aria-hidden />
        <span className="text-[14px] font-semibold">Pause {human(card.slot)}</span>
        {card.state === 'confident' ? (
          <Chip tone="live" icon={ShieldCheck} title={`Every version’s likely range is inside ±${ev.margin} per 100, and the last ${heldDays} days agree`}>Confident</Chip>
        ) : (
          <Chip tone="off" title="Enough data, but the range is still wide">Early</Chip>
        )}
        <span className="ml-auto inline-flex items-center gap-1 text-[12px] text-muted"><Lightbulb className="size-3.5" aria-hidden />Advisor · {ago(card.created_at)}</span>
      </div>
      <div className="space-y-3 p-5">
        <p className="text-[15px] leading-relaxed text-ink" data-testid="advisor-claim">
          {human(card.slot)} makes no clear difference: {n === 1 ? 'its version gets' : `its ${n} versions get`} about as many {u.many} as the page’s own.
        </p>
        <p className="text-[13px] text-muted tabular">
          Within ±{ev.margin} {u.many} per 100 visits, over {int(ev.rows)} visits. About {int(ev.visits_per_week)} visits a week go through it.
        </p>
        <p className="text-[13px] text-ink-2">
          Approving pauses {card.pauses.map((k, i) => <span key={k}>{i > 0 && (i === n - 1 ? ' and ' : ', ')}<span className="font-mono text-[12px]">{k}</span></span>)}. Everyone then sees the page’s own.
        </p>
        <button type="button" aria-expanded={open} onClick={() => setOpen(!open)} className="inline-flex h-6 items-center gap-1 rounded-sm text-[12px] text-muted hover:text-ink">
          Details<ChevronDown className={clsx('size-3.5 transition-transform duration-150', open && 'rotate-180')} aria-hidden />
        </button>
        {open && (
          <div className="text-[13px]" data-testid="advisor-evidence">
            <table className="w-full">
              <thead>
                <tr className="text-left text-[12px] text-muted">
                  <th className="py-1 pr-3 font-normal">Version</th>
                  <th className="py-1 pr-3 font-normal">Per 100 vs the page’s own</th>
                  <th className="py-1 font-normal">Last {heldDays} days</th>
                </tr>
              </thead>
              <tbody>{ev.arms.map((a) => <ArmRow key={a.key} a={a} held={ev.held_out.find((h) => h.key === a.key)} />)}</tbody>
            </table>
            <p className="mt-2 text-[12px] text-muted">Each range is 95%, corrected for who the system chose to show each version to. The last {heldDays} days were kept aside to check it.</p>
          </div>
        )}
        <div className="max-w-sm">
          <label className="mb-1.5 block text-[13px] font-medium text-ink-2" htmlFor={`reason-${card.id}`}>Note (optional)</label>
          <input id={`reason-${card.id}`} className={`${inputClass} w-full`} value={reason} onChange={(e) => setReason(e.target.value)} placeholder="Why approve or reject…" />
        </div>
        {error && <p role="alert" className="text-[13px] text-pause-ink">{error}</p>}
      </div>
      <div className="flex flex-wrap items-center gap-2 rounded-b-lg border-t border-line bg-paper/60 px-5 py-3">
        <Button icon={Check} onClick={() => decide.mutate('approve')} disabled={!approver.trim() || decide.isPending}>Approve pause</Button>
        <Button variant="danger" icon={X} onClick={() => decide.mutate('reject')} disabled={!approver.trim() || decide.isPending}>Reject</Button>
        <Help title="Rejecting"><p>Nothing changes on the slot. The advisor leaves it alone for {quiet} days.</p></Help>
        {!approver.trim() && <span className="text-[12px] text-muted">Add your name at the top to decide.</span>}
      </div>
    </Card>
  );
}

/** The advisor's queue in Review: one card at a time, oldest first. */
export function AdvisorQueue({ cards, site, approver }: { cards: AdvisorCard[]; site: string; approver: string }) {
  const s = useAdvisorSettings(site);
  if (cards.length === 0) return null;
  return (
    <section>
      <SectionTitle
        hint={cards.length > 1 ? `1 of ${cards.length}` : undefined}
        help={<><p>The advisor reads each slot’s results every night. When a slot’s versions make no difference, it suggests pausing them.</p><p>It never changes traffic itself.</p></>}
      >
        Advisor
      </SectionTitle>
      <PauseCard key={cards[0].id} card={cards[0]} site={site} approver={approver} quiet={s.data?.quiet_days ?? 30} heldDays={s.data?.held_out_days ?? 14} />
    </section>
  );
}

/** Results: the advisor's one line on a slot. */
export function AdvisorNote({ a, metric, site }: { a: AdvisorLine; metric: string; site: string }) {
  const u = unit(metric);
  const per100 = (x: number) => signed(x * 100);
  let text;
  if (a.state === 'not_enough_data') text = <>Advisor: not enough data yet.</>;
  else if (a.state === 'card_pending')
    text = <>Advisor suggestion: pause this slot. <Link href={`/approvals/?site=${site}`} className="text-ink hover:underline">Waiting in Review</Link>.</>;
  else {
    const d = a.orders_delta;
    text = (
      <>
        Advisor suggestion: paused {a.since ? day(a.since) : ''}. Since then:{' '}
        {d ? <><span className="text-ink">{per100(d.estimate)} {u.many} per 100</span> ({per100(d.lo)} to {per100(d.hi)}).</> : <>no results yet.</>}
      </>
    );
  }
  return (
    <p className="mt-1.5 flex items-start gap-1.5 text-[13px] text-muted tabular" data-testid="advisor-line">
      <Lightbulb className="mt-0.5 size-3.5 shrink-0" aria-hidden />
      <span>
        {text}
        {a.state === 'approved' && (
          <Help title="Since the pause">
            <p>The slot against the comparison group since its versions were paused. Near zero means pausing cost nothing.</p>
            {a.before && <p>Over the same length of time before: {per100(a.before.estimate)} per 100 ({per100(a.before.lo)} to {per100(a.before.hi)}).</p>}
          </Help>
        )}
      </span>
    </p>
  );
}

type Key = 'min_rows' | 'margin' | 'held_out_days' | 'fdr_q' | 'cards_per_week' | 'quiet_days';

const FIELDS: { key: Key; label: string; tip: string; whole: boolean }[] = [
  { key: 'margin', label: 'Margin (per 100)', tip: 'How close to the page’s own counts as no difference, per 100 visits. A small margin needs a lot of traffic: ±0.3 needs about 80,000 visits per version at a 10% rate.', whole: false },
  { key: 'min_rows', label: 'Visits per version', tip: 'How many visits each version needs before the advisor judges the slot.', whole: true },
  { key: 'held_out_days', label: 'Check window (days)', tip: 'The newest days are kept aside to check a finding before a card is shown.', whole: true },
  { key: 'fdr_q', label: 'False alarm rate', tip: 'Across one night’s slots, at most this share of cards may be wrong. Lower is stricter.', whole: false },
  { key: 'cards_per_week', label: 'Cards per week', tip: 'The most suggestions the advisor queues in one week.', whole: true },
  { key: 'quiet_days', label: 'Quiet after reject (days)', tip: 'After you reject a card, the advisor leaves that slot alone this long.', whole: true },
];

/** Visits each version needs for a 95% range of ±margin per 100 at a 10% rate: 2·1.96²·p(1−p)/m². */
function visitsFor(margin: number) {
  const n = (2 * 1.96 ** 2 * 0.1 * 0.9) / (margin / 100) ** 2;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e4) return `${Math.round(n / 1000)}k`;
  return int(Math.ceil(n / 10) * 10);
}

function NumberField({ k, value, onChange, settings }: { k: (typeof FIELDS)[number]; value: string; onChange: (v: string) => void; settings: AdvisorSettings }) {
  const id = useId();
  const [lo, hi] = settings.bounds[k.key];
  const ok = valid(value, k.whole, [lo, hi]);
  const m = Number(value);
  return (
    <div className="w-full sm:w-52">
      <div className="mb-1.5 flex items-center gap-1">
        <label htmlFor={id} className="text-[13px] font-medium text-ink-2">{k.label}</label>
        <Tip label={k.tip} width={260}>
          <span tabIndex={0} aria-label={`About ${k.label.toLowerCase()}`} className="grid size-5 place-items-center rounded-full text-muted hover:text-ink">
            <CircleHelp className="size-3.5" aria-hidden />
          </span>
        </Tip>
      </div>
      <input id={id} type="number" inputMode={k.whole ? 'numeric' : 'decimal'} step={k.whole ? 1 : 'any'} min={lo} max={hi} aria-invalid={!ok} className={`${inputClass} w-full tabular`} value={value} onChange={(e) => onChange(e.target.value)} />
      <span className={clsx('mt-1 block text-xs text-muted tabular', !ok && 'text-pause-ink')}>
        {lo}–{int(hi)}, default {settings.defaults[k.key]}
        {k.key === 'margin' && ok && <> · needs ~{visitsFor(m)} visits per version</>}
      </span>
    </div>
  );
}

function valid(v: string, whole: boolean, [lo, hi]: [number, number]) {
  if (!(whole ? /^\d+$/ : /^\d*\.?\d+$/).test(v.trim())) return false;
  const n = Number(v);
  return n >= lo && n <= hi;
}

/** The advisor's six thresholds for this site. */
export function AdvisorSettingsPanel({ site }: { site: string }) {
  const qc = useQueryClient();
  const q = useAdvisorSettings(site);
  const [vals, setVals] = useState<Record<Key, string> | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    if (q.data) setVals(Object.fromEntries(FIELDS.map((f) => [f.key, String(q.data[f.key])])) as Record<Key, string>);
  }, [q.data]);
  const save = useMutation({
    mutationFn: () => api.post<AdvisorSettings>(`/sites/${site}/advisor-settings`, Object.fromEntries(FIELDS.filter((f) => Number(vals![f.key]) !== q.data!.defaults[f.key]).map((f) => [f.key, Number(vals![f.key])]))),
    onSuccess: (d) => {
      qc.setQueryData(['advisor-settings', site], d);
      setError(null);
      setSaved(true);
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'failed'),
  });
  if (!q.data || !vals) return null;
  const d = q.data;
  const allOk = FIELDS.every((f) => valid(vals[f.key], f.whole, d.bounds[f.key]));
  const dirty = FIELDS.some((f) => Number(vals[f.key]) !== d[f.key]);
  const isDefault = FIELDS.every((f) => Number(vals[f.key]) === d.defaults[f.key]);
  const set = (k: Key, v: string) => {
    setVals({ ...vals, [k]: v });
    setSaved(false);
  };
  return (
    <Card className="px-5 py-4 sm:px-6" data-testid="advisor-settings">
      <details className="group" data-testid="advisor-settings-details">
      <summary className="flex cursor-pointer list-none items-center justify-between gap-2 rounded-sm text-[14px] font-semibold text-ink [&::-webkit-details-marker]:hidden">
        <span>Advisor settings <span className="ml-1 text-[13px] font-normal text-muted">when it suggests a pause</span></span>
        <ChevronDown className="size-4 text-muted transition-transform group-open:rotate-180" aria-hidden />
      </summary>
      <p className="mt-3 text-[13px] text-muted">Stricter settings mean fewer, surer cards. A card only appears once every version is inside the margin and the newest days agree.</p>
      <form
        className="mt-4 flex flex-wrap items-start gap-x-6 gap-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          if (allOk) save.mutate();
        }}
      >
        {FIELDS.map((f) => <NumberField key={f.key} k={f} value={vals[f.key]} onChange={(v) => set(f.key, v)} settings={d} />)}
        <div className="flex w-full flex-wrap items-center gap-2 border-t border-line pt-4">
          <Button type="submit" icon={Save} disabled={!dirty || !allOk || save.isPending}>Save</Button>
          <Button variant="quiet" icon={RotateCcw} disabled={isDefault} onClick={() => { setVals(Object.fromEntries(FIELDS.map((f) => [f.key, String(d.defaults[f.key])])) as Record<Key, string>); setSaved(false); }}>Restore defaults</Button>
          {error && <span role="alert" className="text-[13px] text-pause-ink">{error}</span>}
          {saved && !error && !dirty && <span role="status" className="inline-flex items-center gap-1.5 text-[13px] text-good-ink"><CircleCheck className="size-4" aria-hidden />Saved</span>}
        </div>
      </form>
      </details>
    </Card>
  );
}
