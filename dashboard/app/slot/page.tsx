'use client';
// Run, one slot: the variant strip, the lift curve, the decisions table.
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import { useQuery } from '@tanstack/react-query';
import { createColumnHelper, tableFeatures, useTable } from '@tanstack/react-table';
import { Suspense, useMemo, useState } from 'react';
import dynamic from 'next/dynamic';
import { LiftPill } from '@/components/charts';
import { Audiences } from '@/components/audiences';
import { FieldView } from '@/components/assets';
import { Preview } from '@/components/preview';
import { ShareEditor } from '@/components/share-editor';
import { Crumb, Shell } from '@/components/shell';
import { Activity, Clock, Crown, FilePlus2, GitBranch, LayoutList, ListOrdered, Magnet, Target, Timer, TrendingUp, Users } from 'lucide-react';
import { Card, Chip, Empty, Loading, SectionTitle, Tip, inputClass, type ChipTone } from '@/components/ui';
import { api, type DecisionRow, type FieldValue, type ItemRow, type SlotDetail, type VariantRow } from '@/lib/api';
import { ago, inWindow, int, pct, pp, seriesVar, shortId, when } from '@/lib/format';
import { useParam } from '@/lib/page';

// Recharts is the heaviest dependency; it loads only on the pages that draw a curve.
const LiftCurve = dynamic(() => import('@/components/lift-curve').then((m) => m.LiftCurve), {
  ssr: false,
  loading: () => <div className="h-[160px] animate-pulse rounded-md bg-surface-2/60 motion-reduce:animate-none" aria-hidden />,
});

const STATUS: Record<VariantRow['status'], [ChipTone, string]> = {
  approved: ['live', 'live'],
  paused: ['paused', 'paused by rollback'],
  draft: ['warn', 'draft'],
  rejected: ['off', 'rejected'],
  retired: ['off', 'retired'],
};

/** A copy slot's variant (F4): its fields in the slot's declared order, text as text and images as images (S11). */
function CopyFields({ payload, fields, site }: { payload: string | null; fields: SlotDetail['fields']; site: string }) {
  let text: Record<string, FieldValue> = {};
  try {
    text = JSON.parse(payload ?? '{}') ?? {};
  } catch {}
  const names = fields?.map((f) => f.name) ?? Object.keys(text);
  return (
    <dl className="min-h-24 space-y-2 text-[13px] leading-snug" data-testid="copy-fields">
      {names.map((n) => (
        <div key={n}>
          <dt className="font-mono text-[11px] text-muted">{n}</dt>
          <dd className="text-ink"><FieldView value={text[n]} site={site} compact /></dd>
        </div>
      ))}
    </dl>
  );
}

/** S9: the planned share and where it came from, e.g. "planned share 20% · edited by Ritwik". */
function plannedShare(v: VariantRow) {
  const p = (x: number) => `${Math.round(x * 100)}%`;
  let from = '';
  if (v.share_source === 'file') from = ' · site file';
  else if (v.share_source === 'edited') from = ` · edited by ${v.share_set_by}`;
  else if (v.share_source === 'approval') from = ` · set at approval by ${v.share_set_by}`;
  const file = v.share_source !== 'file' && v.file_share != null && v.file_share !== v.traffic_share ? ` (file says ${p(v.file_share)})` : '';
  return `planned share ${p(v.traffic_share!)}${from}${file}`;
}

function VariantCard({ v, index, champion, site, slot, fields }: { v: VariantRow; index: number; champion: boolean; site: string; slot: string; fields: SlotDetail['fields'] }) {
  const byAdvisor = !!v.pause_reason && 'advisor_card' in v.pause_reason;
  const [tone, label] = v.status === 'paused' && byAdvisor ? (['paused', 'paused on advice'] as [ChipTone, string]) : STATUS[v.status];
  return (
    <Card className="flex w-full shrink-0 flex-col sm:w-[17rem]" data-testid={`variant-${v.key}`}>
      <div className="flex items-center gap-2 border-b border-line px-4 py-2.5">
        <span aria-hidden data-testid="series-dot" className="size-2 shrink-0 rounded-full" style={{ background: seriesVar(index) }} />
        <span className="min-w-0 truncate text-[13px] font-medium">{v.key}</span>
        {v.is_baseline && <span className="text-[12px] text-muted">baseline</span>}
        {champion && <span className="ml-auto inline-flex items-center gap-1 text-[12px] font-medium text-ink"><Crown className="size-3.5" aria-hidden />champion</span>}
      </div>
      <div className="flex flex-1 flex-col p-4">
        {fields && !v.is_baseline ? (
          <CopyFields payload={v.payload} fields={fields} site={site} />
        ) : (
          <Preview html={v.is_baseline ? null : v.payload} label={`preview of ${v.key}`} height={96} scale={0.62} />
        )}
        <div className="mt-4 flex items-center justify-between text-[12px]">
          <Chip tone={tone}>{label}</Chip>
          <span className="tabular text-ink-2">{pct(v.share, 0)} of traffic</span>
        </div>
        <div className="mt-2.5"><LiftPill lift={v.lift} small /></div>
        <Tip label={<>{int(v.counts.n)} closed decisions, {credited(v.counts.credit ?? v.counts.conversions)} converted{(v.pending?.decisions ?? 0) > 0 ? `, ${int(v.pending!.decisions)} still in their window` : ''}</>}>
          <div tabIndex={0} className="mt-1.5 inline-flex items-center gap-2.5 rounded-sm text-[12px] text-muted tabular">
            <span className="inline-flex items-center gap-0.5"><Users className="size-3" aria-hidden />{int(v.counts.n)}</span>
            <span className="inline-flex items-center gap-0.5"><Target className="size-3" aria-hidden />{credited(v.counts.credit ?? v.counts.conversions)}</span>
            {(v.pending?.decisions ?? 0) > 0 && <span className="inline-flex items-center gap-0.5"><Clock className="size-3" aria-hidden />{int(v.pending!.decisions)}</span>}
          </div>
        </Tip>
        {v.traffic_share !== null && (
          <div className="mt-1 text-[12px] text-muted tabular" data-testid="planned-share">{plannedShare(v)}</div>
        )}
        {!v.is_baseline && (v.status === 'approved' || v.status === 'paused') && <ShareEditor site={site} slot={slot} v={v} />}
        {v.pause_reason && 'advisor_card' in v.pause_reason && (
          <div className="mt-3 rounded-md bg-surface-2 px-2.5 py-2 text-[12px] leading-snug text-ink-2" data-testid="advisor-pause">
            <b className="text-ink">Paused {when(v.paused_at)}</b> by {v.pause_reason.by}, on the advisor’s suggestion: this slot made no clear difference.
          </div>
        )}
        {v.pause_reason && 'lift' in v.pause_reason && (
          <div className="mt-3 rounded-md bg-pause-soft px-2.5 py-2 text-[12px] leading-snug text-ink-2" data-testid="death-certificate">
            <b className="text-pause-ink">Paused {when(v.paused_at)}.</b> Converted {pct(v.pause_reason.variant.rate)} vs {pct(v.pause_reason.holdout.rate)} for the holdout (lift{' '}
            {pp(v.pause_reason.lift.estimate)}, CS {pp(v.pause_reason.lift.lo)} to {pp(v.pause_reason.lift.hi)}).
            {v.pause_reason.restored_to_version !== null && ` Model restored to v${v.pause_reason.restored_to_version}.`}
          </div>
        )}
      </div>
    </Card>
  );
}

const features = tableFeatures({});
const helper = createColumnHelper<typeof features, DecisionRow>();

function outcomeOf(d: DecisionRow): [ChipTone, string] {
  if (d.outcome === 1) return ['live', 'converted'];
  if (d.outcome === 0) return ['off', 'missed'];
  if (!d.exposed) return [d.settled ? 'off' : 'warn', d.settled ? 'never shown' : 'not shown yet'];
  return ['accent', 'window open'];
}

function DecisionsTable({ site, slot, variants }: { site: string; slot: string; variants: VariantRow[] }) {
  const router = useRouter();
  const [variant, setVariant] = useState('');
  const [outcome, setOutcome] = useState('all');
  const q = useQuery({
    queryKey: ['decisions', site, slot, variant, outcome],
    queryFn: () => api.get<DecisionRow[]>(`/sites/${site}/decisions?slot=${slot}&outcome=${outcome}${variant ? `&variant=${variant}` : ''}&limit=500`),
    placeholderData: (prev) => prev,
  });
  const columns = useMemo(
    () =>
      helper.columns([
        helper.accessor('created_at', { header: 'When', cell: (c) => <Link href={`/decision/?id=${c.row.original.id}&site=${site}`} title={when(c.getValue())} className="rounded-sm text-ink" onClick={(e) => e.stopPropagation()}>{ago(c.getValue())}</Link> }),
        helper.accessor('subject', { header: 'Visitor', cell: (c) => <span className="font-mono text-[12px]">{shortId(c.getValue())}</span> }),
        helper.accessor('variant', { header: 'Variant', cell: (c) => <span className="font-mono text-[12px]">{c.getValue()}</span> }),
        helper.accessor('features', {
          header: 'Context',
          cell: (c) => {
            const f = c.getValue();
            return <span className="text-xs text-ink-2">{[f.returning ? 'returning' : 'new', f.mobile ? 'mobile' : 'desktop', f.from_ad ? 'ad' : null].filter(Boolean).join(' · ')}</span>;
          },
        }),
        helper.accessor('in_holdout', { header: 'Group', cell: (c) => <span className="text-xs">{c.getValue() ? 'holdout' : 'policy'}</span> }),
        helper.display({ id: 'outcome', header: 'Outcome', cell: (c) => { const [t, l] = outcomeOf(c.row.original); return <Chip tone={t}>{l}</Chip>; } }),
      ]),
    [site],
  );
  const data = useMemo(() => q.data ?? [], [q.data]);
  const table = useTable({ features, columns, data });
  return (
    <div>
      <div className="mb-3 flex flex-wrap gap-2">
        <select aria-label="Filter by variant" className={`${inputClass} w-44`} value={variant} onChange={(e) => setVariant(e.target.value)}>
          <option value="">All variants</option>
          {variants.map((v) => <option key={v.id} value={v.key}>{v.key}</option>)}
        </select>
        <select aria-label="Filter by outcome" className={`${inputClass} w-44`} value={outcome} onChange={(e) => setOutcome(e.target.value)}>
          {['all', 'converted', 'missed', 'open', 'unexposed', 'holdout'].map((o) => <option key={o} value={o}>{o === 'all' ? 'All outcomes' : o}</option>)}
        </select>
        <span className="self-center text-[12px] text-muted tabular">{q.data ? `${int(q.data.length)} shown (newest 500)` : ''}</span>
      </div>
      <Card className={`max-h-[460px] overflow-auto transition-opacity duration-150 ${q.isFetching ? 'opacity-60' : ''}`} data-testid="decisions-panel">
        {data.length === 0 && !q.isLoading ? (
          <div className="p-6 text-center text-[13px] text-muted">No decisions match.</div>
        ) : (
          <table className="w-full text-[13px]" data-testid="decisions-table">
            <thead className="sticky top-0 z-10 bg-surface text-left text-[12px] text-muted shadow-[inset_0_-1px_0_var(--line)]">
              {table.getHeaderGroups().map((g) => (
                <tr key={g.id}>{g.headers.map((h) => <th key={h.id} className="h-10 px-4 font-normal whitespace-nowrap">{h.isPlaceholder ? null : <table.FlexRender header={h} />}</th>)}</tr>
              ))}
            </thead>
            <tbody>
              {table.getRowModel().rows.map((r) => (
                <tr key={r.id} className="cursor-pointer border-b border-line transition-colors last:border-0 hover:bg-surface-2/60" onClick={() => router.push(`/decision/?id=${r.original.id}&site=${site}`)}>
                  {r.getAllCells().map((c) => <td key={c.id} className="h-11 px-4 whitespace-nowrap"><table.FlexRender cell={c} /></td>)}
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
    </div>
  );
}

/**
 * G3: a sorted slot's items in the declared order, one numbered row each. Items are not variants:
 * the learner orders them per visitor, so there is no traffic share to set.
 */
function ItemsTable({ items }: { items: ItemRow[] }) {
  const cols = ['#', 'Item', 'Shown', 'Avg position', 'Acted on', 'Rate'];
  return (
    <Card className="overflow-auto">
      <table className="w-full text-[13px]" data-testid="items-table">
        <thead className="border-b border-line text-left text-[12px] text-muted">
          <tr>{cols.map((c) => <th key={c} className="h-10 px-4 font-normal">{c}</th>)}</tr>
        </thead>
        <tbody>
          {items.map((i, n) => (
            <tr key={i.key} className="border-b border-line last:border-0">
              <td className="h-11 w-10 px-4 text-muted tabular">{n + 1}</td>
              <td className="px-4 py-2 font-mono text-[12px]">{i.key}</td>
              <td className="px-4 py-2 tabular">{int(i.shown)}</td>
              <td className="px-4 py-2 tabular">{i.mean_position === null ? '—' : i.mean_position.toFixed(1)}</td>
              <td className="px-4 py-2 tabular">{int(i.acted)}</td>
              <td className="px-4 py-2 tabular">{pct(i.rate, 0)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </Card>
  );
}

function Workbench() {
  const site = useParam('site')!;
  const slot = useParam('slot')!;
  const q = useQuery({ queryKey: ['slot', site, slot], queryFn: () => api.get<SlotDetail>(`/sites/${site}/slots/${slot}`), refetchInterval: 15_000 });
  if (q.isLoading || !q.data) return <Loading />;
  const s = q.data;
  const shown = s.variants.filter((v) => v.status !== 'draft' && v.status !== 'rejected');
  const series = s.variants.map((v, index) => ({ key: v.key, index })).filter((x) => s.curve.some((p) => p.variants[x.key]));
  // A sorted slot's variants are its items: a numbered list, no cards and no share to set.
  const sorted = s.kind === 'sorted';
  const items: ItemRow[] = s.items ?? shown.map((v) => ({ key: v.key, shown: 0, mean_position: null, seen: 0, acted: 0, rate: null }));
  return (
    <div className="grid gap-10 xl:grid-cols-[1fr_280px]">
      <div className="min-w-0 space-y-12">
        {sorted ? (
          <section>
            <SectionTitle icon={ListOrdered} help="Items in the order declared. The learner reorders them for each visitor, so there is no share to set.">Items</SectionTitle>
            <ItemsTable items={items} />
          </section>
        ) : (
          <section>
            <SectionTitle icon={GitBranch} help={<><p>Per-variant lift compares a variant&apos;s visitors with the holdout. The bandit chose who saw it by context, so it describes, it does not prove.</p><p>The slot&apos;s overall lift, below, is the causal number.</p></>}>Variants</SectionTitle>
            <div className="flex flex-wrap gap-3">
              {shown.map((v) => <VariantCard key={v.id} v={v} index={s.variants.indexOf(v)} champion={v.key === s.champion && v.status === 'approved'} site={site} slot={slot} fields={s.kind === 'copy' ? s.fields : null} />)}
              <Link href={`/drafts/?site=${site}&slot=${slot}`} className="flex min-h-24 w-full shrink-0 flex-col items-center justify-center gap-2 rounded-lg border border-dashed border-line-strong text-center text-[13px] text-muted transition-colors hover:border-ink hover:text-ink sm:min-h-40 sm:w-40">
                <FilePlus2 className="size-5" aria-hidden />
                Next draft
              </Link>
            </div>
          </section>
        )}
        <section>
          <SectionTitle icon={TrendingUp} help="Cumulative lift against the holdout, one line per variant. Zero is the holdout; the wash is the 95% confidence sequence." hint={<LiftPill lift={s.lift} small />}>Lift over time</SectionTitle>
          <Card className="p-4 sm:p-5">
            <LiftCurve curve={s.curve} series={series} />
          </Card>
        </section>
        <section>
          <SectionTitle icon={LayoutList}>Decisions</SectionTitle>
          <DecisionsTable site={site} slot={slot} variants={s.variants} />
        </section>
      </div>
      <aside className="text-[13px]">
        <Card className="divide-y divide-line xl:sticky xl:top-32">
          <div className="space-y-2.5 p-4 text-[13px] text-ink-2">
            <SectionTitle>Slot</SectionTitle>
            <SlotFact icon={Target} tip="The goal this slot is judged on">Metric <span className="font-mono text-[12.5px] text-ink">{s.metric}</span></SlotFact>
            <SlotFact icon={Timer} tip="How long after a decision a goal still counts">Window {Math.round(s.attribution_window_secs / 60)} min</SlotFact>
            <SlotFact icon={Magnet} tip="How long a visitor keeps the same version">Sticky {s.decision_ttl_secs} s</SlotFact>
            <SlotFact icon={Activity} tip="The model version deciding now">Model <span className="font-mono text-[12.5px] text-ink">v{s.model_version}</span></SlotFact>
            <SlotFact icon={Users} tip="Closed decisions: personalised, then holdout"><span className="tabular">{int(s.treated.n)} / {int(s.holdout.n)}</span></SlotFact>
            {(s.pending?.decisions ?? 0) > 0 && (
              <div className="tabular text-muted" data-testid="slot-pending">{inWindow(s.pending!.decisions)}, counted when it closes</div>
            )}
          </div>
          <div className="p-4">
            <Audiences site={site} slot={slot} variants={s.variants} audiences={s.audiences} />
          </div>
        </Card>
      </aside>
    </div>
  );
}

function Page() {
  const site = useParam('site');
  const slot = useParam('slot');
  return (
    <Shell
      site={site}
      title={slot ?? 'Slot'}
      mono
      crumbs={<Crumb href={`/site/?id=${site}`}>Live</Crumb>}
    >
      {site && slot ? <Workbench /> : <Empty>No slot given.</Empty>}
    </Shell>
  );
}

export default function SlotPage() {
  return (
    <Suspense>
      <Page />
    </Suspense>
  );
}

/** Credited conversions: whole under last touch, a sum of shares under multi-touch. */
function credited(c: number) {
  return Number.isInteger(c) ? int(c) : c.toFixed(1);
}

function SlotFact({ icon: I, tip, children }: { icon: typeof Target; tip: string; children: React.ReactNode }) {
  return (
    <Tip label={tip} className="flex">
      <div tabIndex={0} className="flex items-center gap-2.5 rounded-sm">
        <I className="size-3.5 shrink-0 text-muted" aria-hidden />
        <span>{children}</span>
      </div>
    </Tip>
  );
}
