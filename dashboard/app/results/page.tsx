'use client';
// Learn: what the outcomes say, in words a shop owner reads at a glance. Each answer is derived
// from the live results (lib/plain.ts); the technical figures sit behind "Details".
import Link from 'next/link';
import { useQueries, useQuery } from '@tanstack/react-query';
import { Suspense, useId, useState, type ReactNode } from 'react';
import dynamic from 'next/dynamic';
import { ChevronDown, Lightbulb, Trophy, Undo2 } from 'lucide-react';
import { AdvisorNote } from '@/components/advisor';
import { LiftPill } from '@/components/charts';
import { RateBars, VersionRanks, type RankRow } from '@/components/results-charts';
import { Shell } from '@/components/shell';
import { Card, Chip, Empty, Help, Loading, Prose, SectionTitle, type ChipTone } from '@/components/ui';
import { api, type AudienceRow, type Results, type SlotResult } from '@/lib/api';
import { ago, compact, inWindow, int, pp } from '@/lib/format';
import { useParam } from '@/lib/page';
import { human, moreOrFewer, sureness, unit, visitsToBeSure, type Sureness } from '@/lib/plain';

// Recharts is the heaviest dependency; it loads only when someone opens a slot's details.
const LiftCurve = dynamic(() => import('@/components/lift-curve').then((m) => m.LiftCurve), {
  ssr: false,
  loading: () => <div className="h-[160px] animate-pulse rounded-md bg-surface-2/60 motion-reduce:animate-none" aria-hidden />,
});

function money(i: { estimate: number } | null | undefined, currency = 'USD') {
  if (!i) return '—';
  return new Intl.NumberFormat(undefined, { style: 'currency', currency, maximumFractionDigits: 2 }).format(i.estimate);
}

const CHIP: Record<Sureness, { tone: ChipTone; label: string }> = {
  'sure-up': { tone: 'live', label: 'Sure' },
  'leaning-up': { tone: 'accent', label: 'Fairly sure' },
  flat: { tone: 'off', label: 'No clear difference' },
  unclear: { tone: 'off', label: 'Too early to tell' },
  'leaning-down': { tone: 'warn', label: 'Fairly sure it is worse' },
  'sure-down': { tone: 'paused', label: 'Sure it is worse' },
};

const ANSWER: Record<Sureness, string> = {
  'sure-up': 'Yes',
  'leaning-up': 'Probably yes',
  flat: 'Not noticeably',
  unclear: 'Too early to tell',
  'leaning-down': 'Probably not',
  'sure-down': 'No, it is hurting',
};

/** "add-to-carts and clicks", from the slots that have results. */
function unitsOf(slots: SlotResult[]) {
  const names = [...new Set(slots.filter((s) => s.treated.n + s.holdout.n > 0).map((s) => unit(s.metric).many))];
  if (names.length === 0) return 'conversions';
  return names.length === 1 ? names[0] : `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
}

/** The "?" text every sureness word shares. */
function SureHelp() {
  return (
    <Help title="How sure is this?">
      <p><b>Sure</b>: the whole likely range is on one side of the original. <b>Fairly sure</b>: at least 80% of it is. <b>Too early to tell</b>: it could go either way.</p>
      <p>Technically: the range is a 95% confidence sequence on the difference in conversion rate against the holdout, the visitors held back to always see the original. It stays valid however often you look.</p>
    </Help>
  );
}

/** A "Details" button and the panel it opens. The panel stays in the page (hidden) so its text is searchable. */
function useDetails() {
  const id = useId();
  const [open, setOpen] = useState(false);
  const button = (
    <button type="button" aria-expanded={open} aria-controls={id} onClick={() => setOpen((o) => !o)} className="inline-flex h-6 shrink-0 items-center gap-1 rounded-sm text-[12px] text-muted hover:text-ink">
      Details
      <ChevronDown className={`size-3.5 transition-transform ${open ? 'rotate-180' : ''}`} aria-hidden />
    </button>
  );
  const panel = (children: ReactNode) => (
    <div id={id} hidden={!open} className="mt-4 space-y-3 border-t border-line pt-4 text-xs text-ink-2">
      {children}
    </div>
  );
  return { open, button, panel };
}

function Headline({ data }: { data: Results }) {
  const { results: r, decisions_total } = data;
  const d = useDetails();
  const h = r.lift;
  const units = unitsOf(r.slots);
  const visits = r.treated.n + r.holdout.n;
  const sure = h ? sureness(h) : null;
  const need = h ? visitsToBeSure(h, visits) : null;
  const tRate = r.treated.conversion_rate;
  const hRate = r.holdout.conversion_rate;
  const hasRevenue = (r.treated.revenue ?? 0) + (r.holdout.revenue ?? 0) > 0 && r.treated.revenue_per_visitor && r.holdout.revenue_per_visitor;
  return (
    <Card className="p-5 sm:p-6" data-testid="headline">
      <div className="grid gap-8 md:grid-cols-[minmax(0,1.25fr)_minmax(0,1fr)] md:gap-10">
        <div className="min-w-0">
          <div className="flex items-center justify-between gap-3">
            <div className="flex items-center gap-0.5 text-[13px] text-muted">
              Is personalisation helping?
              <Help title="What this compares">
                <p>{Math.round(r.holdout_fraction * 100)}% of visitors are held back as a comparison group: they always see the original version of every slot. Everyone else gets personalised versions.</p>
                <p>A visit here is one slot shown to one visitor, so a page with three slots counts three. Visits count once their result is known (the attribution window has closed).</p>
              </Help>
            </div>
            {d.button}
          </div>
          <div className="mt-2 text-[32px] leading-tight font-semibold tracking-[-0.03em] sm:text-[40px]" data-testid="headline-answer">
            {sure ? ANSWER[sure] : 'Too early to tell'}
          </div>
          <p className="mt-3 max-w-prose text-[15px] leading-relaxed text-ink" data-testid="headline-sentence">
            {h ? (
              sure === 'flat' ? (
                <>So far personalised visits and the comparison group convert at about the same rate ({moreOrFewer(h.estimate, { one: 'conversion', many: units })} per 1,000 visits).</>
              ) : (
                <>Personalised visits bring {moreOrFewer(h.estimate, { one: 'conversion', many: units })} per 1,000 visits than showing everyone the original.</>
              )
            ) : (
              <>Nothing to compare yet: no visit has a known result.</>
            )}
          </p>
          <p className="mt-2 inline-flex flex-wrap items-center gap-x-2 gap-y-1 text-[13px] text-muted">
            {sure && <span className="inline-flex items-center"><Chip tone={CHIP[sure].tone}>{CHIP[sure].label}</Chip><SureHelp /></span>}
            <span className="tabular">
              {need ? <>Needs roughly {int(need)} more visits to be sure. </> : null}
              Based on {int(visits)} visits.
            </span>
          </p>
        </div>
        {tRate && hRate && (
          <div className="min-w-0 self-center">
            <RateBars treated={tRate.estimate} holdout={hRate.estimate} rangeT={tRate} rangeH={hRate} unitMany={units} />
            {hasRevenue && (
              <p className="mt-4 text-[13px] text-ink-2 tabular" data-testid="headline-revenue">
                Spend per visit: <b className="font-medium text-ink">{money(r.treated.revenue_per_visitor, r.currency)}</b> personalised vs {money(r.holdout.revenue_per_visitor, r.currency)} original
              </p>
            )}
          </div>
        )}
      </div>
      {d.panel(
        <p className="tabular">
          Lift vs holdout {h ? <>{pp(h.estimate)} ({pp(h.lo)} to {pp(h.hi)}, 95% confidence sequence)</> : '—'} · {compact(visits)} closed of {compact(decisions_total)} decisions · {Math.round(r.holdout_fraction * 100)}% holdout
          {r.revenue_lift && <> · revenue lift {money(r.revenue_lift, r.currency)} per visitor ({money({ estimate: r.revenue_lift.lo }, r.currency)} to {money({ estimate: r.revenue_lift.hi }, r.currency)})</>}
          {(r.pending?.decisions ?? 0) > 0 && <> · {inWindow(r.pending!.decisions)}</>}
        </p>,
      )}
    </Card>
  );
}

function SlotCard({ s, site, data, audiences }: { s: SlotResult; site: string; data: Results; audiences: Map<number, string> }) {
  const d = useDetails();
  const { results: r, curves } = data;
  const u = unit(s.metric);
  const visits = s.treated.n + s.holdout.n;
  const sure = s.lift ? sureness(s.lift) : null;
  const need = s.lift ? visitsToBeSure(s.lift, visits) : null;
  const rows: RankRow[] = s.variants.flatMap((v) =>
    v.lift ? [{ key: v.key, lift: v.lift, isOriginal: v.is_baseline, n: v.counts.n, conversions: v.counts.conversions, audience: v.audience ? audiences.get(v.audience) ?? null : null }] : [],
  );
  const leader = [...rows].sort((a, b) => b.lift.estimate - a.lift.estimate)[0];
  const leaderSure = leader ? sureness(leader.lift) : null;
  const pending = s.pending?.decisions ?? 0;
  const idx = new Map(s.variants.map((v, i) => [v.key, i]));
  const series = s.variants.map((v) => ({ key: v.key, index: idx.get(v.key)! })).filter((x) => (curves[s.key] ?? []).some((p) => p.variants[x.key]));
  const unseen = s.variants.filter((v) => !v.lift).length;
  return (
    <Card className="p-4 sm:p-5" data-testid={`result-${s.key}`}>
      <div className="mb-2 flex items-start justify-between gap-3">
        <div className="min-w-0">
          <Link href={`/slot/?site=${site}&slot=${s.key}`} className="rounded-sm text-[15px] font-semibold tracking-[-0.01em] hover:underline">{human(s.key)}</Link>
        </div>
        {d.button}
      </div>
      <p className="text-[14px] leading-relaxed text-ink" data-testid={`verdict-${s.key}`}>
        {s.lift ? (
          sure === 'flat' ? (
            <>Personalising this slot makes no clear difference so far ({moreOrFewer(s.lift.estimate, u)} per 1,000 visits).</>
          ) : (
            <>Personalising this slot gets {moreOrFewer(s.lift.estimate, u)} per 1,000 visits than always showing the original.</>
          )
        ) : (
          <>Too early to tell: {int(s.treated.n)} {s.treated.n === 1 ? 'visit' : 'visits'} counted so far{s.holdout.n === 0 ? ', and none yet in the comparison group' : ''}.</>
        )}
      </p>
      {s.lift && (
        <p className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-[13px] text-muted tabular">
          {sure && <Chip tone={CHIP[sure].tone}>{CHIP[sure].label}</Chip>}
          <span>
            {s.learned === false ? 'Not learning yet: no results have come back.' : <>Still learning from every visit ({int(s.lessons ?? s.treated.n)} so far).</>}
            {need ? <> Needs roughly {int(need)} more to be sure.</> : null}
          </span>
        </p>
      )}
      {/* "Not enough data" repeats "Too early to tell" on a slot with no result yet. */}
      {s.advisor && (s.lift || s.advisor.state !== 'not_enough_data') && <AdvisorNote a={s.advisor} metric={s.metric} site={site} />}
      {pending > 0 && <p className="mt-1.5 text-[13px] text-muted tabular">{int(pending)} recent {pending === 1 ? 'visit is' : 'visits are'} still waiting to see if {pending === 1 ? 'it converts' : 'they convert'}.</p>}
      {leader && rows.length > 1 && (
        <div className="mt-5">
          <p className="mb-2 text-[13px] text-ink-2" data-testid={`leader-${s.key}`}>
            {leader.isOriginal ? (
              <>The original, <b className="font-medium text-ink">‘{human(leader.key)}’</b>, is still doing best.</>
            ) : (
              <>
                <b className="font-medium text-ink">‘{human(leader.key)}’</b>{leader.audience ? <>, shown to {leader.audience},</> : null} is doing best: {moreOrFewer(leader.lift.estimate, u)} per 1,000 visits that saw it
                {leaderSure && <> ({CHIP[leaderSure].label.toLowerCase()})</>}.
              </>
            )}
            <Help title="Each version vs the original">
              <p>Each row is the extra {u.many} per 1,000 visits for the visits that version was shown on, against the comparison group, who always see the original. The dot is the best guess, the bar the likely range, the thin line the comparison group.</p>
              <p>The system picks who sees which version, so rows are not a fair head-to-head; the slot total above is. Technically: per-variant lift vs the holdout, descriptive only, with 95% confidence sequences.</p>
            </Help>
          </p>
          <VersionRanks rows={rows} unitMany={u.many} testid={`ranks-${s.key}`} />
        </div>
      )}
      {d.panel(
        <>
          <div className="flex flex-wrap items-center gap-x-4 gap-y-1">
            <span className="inline-flex items-center gap-1.5">Lift vs holdout <LiftPill lift={s.lift} small /></span>
            <span className="tabular">Personalised {int(s.treated.n)} / holdout {int(s.holdout.n)} decisions</span>
            <span>Metric <code className="font-mono">{s.metric}</code></span>
          </div>
          {pending > 0 && (
            <p className="tabular" data-testid={`pending-${s.key}`}>
              {inWindow(pending)} ({int(s.pending!.exposures)} shown, {int(s.pending!.conversions)} converted so far), counted when it closes
            </p>
          )}
          {(s.treated.revenue ?? 0) + (s.holdout.revenue ?? 0) > 0 && (
            <p className="tabular" data-testid={`revenue-${s.key}`}>
              Revenue per visitor {money(s.treated.revenue_per_visitor, r.currency)} vs {money(s.holdout.revenue_per_visitor, r.currency)} holdout
              {s.revenue_lift && <> · lift {money(s.revenue_lift, r.currency)} ({money({ estimate: s.revenue_lift.lo }, r.currency)} to {money({ estimate: s.revenue_lift.hi }, r.currency)})</>}
              {s.goal === 'revenue' && <> · optimising revenue</>}
            </p>
          )}
          {unseen > 0 && <p>{unseen} {unseen === 1 ? 'version has' : 'versions have'} no results yet.</p>}
          {d.open && series.length > 0 && (
            <div>
              <p className="mb-2 text-muted">Over time, each version vs the comparison group (percentage points){leader && !leader.isOriginal ? <>, ‘{human(leader.key)}’ highlighted</> : null}</p>
              <LiftCurve curve={curves[s.key] ?? []} series={series} height={160} highlight={leader?.key} yLabel="pp vs comparison" />
            </div>
          )}
          <Link href={`/slot/?site=${site}&slot=${s.key}`} className="inline-block rounded-sm text-ink hover:underline">Open the slot workbench</Link>
        </>,
      )}
    </Card>
  );
}

function plainLearning(l: Results['learnings'][number]) {
  const v = l.variant ? `‘${human(l.variant)}’` : 'A version';
  const on = l.slot ? ` on ${human(l.slot)}` : '';
  if (l.kind === 'winning') return `${v}${on} clearly beats the original.`;
  if (l.kind === 'rollback') return `${v}${on} was switched off automatically: it was doing worse than the original.`;
  return null;
}

function View() {
  const site = useParam('site')!;
  const q = useQuery({ queryKey: ['results', site], queryFn: () => api.get<Results>(`/sites/${site}/results`), refetchInterval: 30_000 });
  // Audience names, only for slots where a version is limited to an audience.
  const gated = (q.data?.results.slots ?? []).filter((s) => s.variants.some((v) => v.audience)).map((s) => s.key);
  const aq = useQueries({ queries: gated.map((k) => ({ queryKey: ['slot', site, k], queryFn: () => api.get<{ audiences: AudienceRow[] }>(`/sites/${site}/slots/${k}`), staleTime: 60_000 })) });
  if (q.isLoading || !q.data) return <Loading />;
  const audiences = new Map<number, string>();
  for (const a of aq) for (const x of a.data?.audiences ?? []) audiences.set(x.id, x.name);
  const { results: r, learnings } = q.data;
  // Ranked by contribution: the slot's lift times its share of personalised visitors.
  const ranked = [...r.slots].sort((a, b) => (b.lift?.estimate ?? -Infinity) * b.treated.n - (a.lift?.estimate ?? -Infinity) * a.treated.n);
  return (
    <div className="space-y-12">
      <Headline data={q.data} />
      <section>
        <SectionTitle help="One card per slot (a place on your pages that shows different versions). The slots doing the most good come first.">Slots, most helpful first</SectionTitle>
        {ranked.length === 0 && <Empty>No slots.</Empty>}
        <div className="grid items-start gap-4 lg:grid-cols-2">
          {ranked.map((s) => <SlotCard key={s.key} s={s} site={site} data={q.data} audiences={audiences} />)}
        </div>
      </section>
      <section>
        <SectionTitle help="A line lands here when the evidence becomes clear: a version clearly beats the original, or one is switched off automatically for doing worse. Not on a schedule.">What we have learned</SectionTitle>
        {learnings.length === 0 ? (
          <Empty icon={Lightbulb}>Nothing certain yet.</Empty>
        ) : (
          <Card className="divide-y divide-line">
            {learnings.map((l) => {
              const plain = plainLearning(l);
              return (
                <div key={l.id} className="flex flex-wrap gap-x-3 gap-y-1.5 px-4 py-3.5 text-[13px]" data-testid="learning">
                  <span className="pt-0.5">{l.kind === 'rollback' ? <Chip tone="paused" icon={Undo2}>Switched off</Chip> : <Chip tone="live" icon={Trophy}>Winner</Chip>}</span>
                  <div className="min-w-0 flex-1 basis-60">
                    <p className="text-ink">{plain ?? <Prose text={l.sentence} />}</p>
                    {plain && (
                      <details className="mt-1 text-xs text-muted">
                        <summary className="cursor-pointer rounded-sm hover:text-ink">Details</summary>
                        <p className="mt-1"><Prose text={l.sentence} /></p>
                      </details>
                    )}
                  </div>
                  <span className="shrink-0 text-xs text-muted">{ago(l.at)}</span>
                </div>
              );
            })}
          </Card>
        )}
      </section>
    </div>
  );
}

function Page() {
  const site = useParam('site');
  return (
    <Shell site={site} title="Results">
      {site ? <View /> : <Empty>No site given.</Empty>}
    </Shell>
  );
}

export default function ResultsPage() {
  return (
    <Suspense>
      <Page />
    </Suspense>
  );
}
