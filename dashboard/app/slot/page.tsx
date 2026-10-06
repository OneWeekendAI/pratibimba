'use client';
// Sheet 02, the slot workbench: the variant strip, the lift curve, the decisions table.
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import { useQuery } from '@tanstack/react-query';
import { createColumnHelper, tableFeatures, useTable } from '@tanstack/react-table';
import { Suspense, useMemo, useState } from 'react';
import { LiftCurve, LiftPill } from '@/components/charts';
import { Preview } from '@/components/preview';
import { Shell } from '@/components/shell';
import { Card, Chip, Empty, Loading, SectionTitle, inputClass, type ChipTone } from '@/components/ui';
import { api, type DecisionRow, type SlotDetail, type VariantRow } from '@/lib/api';
import { ago, int, pct, pp, seriesVar, shortId, when } from '@/lib/format';
import { useParam } from '@/lib/page';

const STATUS: Record<VariantRow['status'], [ChipTone, string]> = {
  approved: ['live', 'live'],
  paused: ['paused', 'paused by rollback'],
  draft: ['warn', 'draft'],
  rejected: ['off', 'rejected'],
  retired: ['off', 'retired'],
};

function VariantCard({ v, index, champion }: { v: VariantRow; index: number; champion: boolean }) {
  const [tone, label] = STATUS[v.status];
  return (
    <Card className="relative w-64 shrink-0 overflow-hidden" data-testid={`variant-${v.key}`}>
      {champion && <div className="absolute top-0 right-0 rounded-bl-md bg-accent px-2 py-0.5 text-[10.5px] font-semibold text-accent-ink">champion</div>}
      <div className="h-1" style={{ background: seriesVar(index) }} />
      <div className="p-3">
        <div className="mb-2 flex items-center gap-2">
          <span className="font-mono text-[13px] font-semibold">{v.key}</span>
          {v.is_baseline && <span className="text-[11px] text-muted">baseline</span>}
        </div>
        <Preview html={v.is_baseline ? null : v.payload} label={`preview of ${v.key}`} height={96} scale={0.62} />
        <div className="mt-3 flex items-center justify-between text-xs">
          <Chip tone={tone}>{label}</Chip>
          <span className="tabular text-ink-2">{pct(v.share, 0)} of traffic</span>
        </div>
        <div className="mt-2"><LiftPill lift={v.lift} small /></div>
        <div className="mt-1 text-[11px] text-muted tabular">
          {int(v.counts.n)} closed · {int(v.counts.conversions)} converted
          {v.traffic_share !== null && ` · planned share ${Math.round(v.traffic_share * 100)}%`}
        </div>
        {v.pause_reason && (
          <div className="mt-2 rounded-md bg-surface-2 p-2 text-[11.5px] leading-snug text-ink-2" data-testid="death-certificate">
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
        helper.accessor('created_at', { header: 'When', cell: (c) => <span title={when(c.getValue())}>{ago(c.getValue())}</span> }),
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
    [],
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
        <span className="self-center text-xs text-muted">{q.data ? `${int(q.data.length)} shown (newest 500)` : ''}</span>
      </div>
      <Card className={`max-h-[460px] overflow-auto transition-opacity ${q.isFetching ? 'opacity-60' : ''}`} data-testid="decisions-panel">
        {data.length === 0 && !q.isLoading ? (
          <div className="p-6 text-center text-[13px] text-muted">No decisions match.</div>
        ) : (
          <table className="w-full text-[13px]" data-testid="decisions-table">
            <thead className="sticky top-0 border-b border-line bg-surface text-left text-xs text-muted">
              {table.getHeaderGroups().map((g) => (
                <tr key={g.id}>{g.headers.map((h) => <th key={h.id} className="px-4 py-2 font-medium">{h.isPlaceholder ? null : <table.FlexRender header={h} />}</th>)}</tr>
              ))}
            </thead>
            <tbody>
              {table.getRowModel().rows.map((r) => (
                <tr key={r.id} className="cursor-pointer border-b border-line last:border-0 hover:bg-surface-2/60" onClick={() => router.push(`/decision/?id=${r.original.id}&site=${site}`)}>
                  {r.getAllCells().map((c) => <td key={c.id} className="px-4 py-2"><table.FlexRender cell={c} /></td>)}
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
    </div>
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
  const live = s.variants.filter((v) => v.status === 'approved');
  return (
    <div className="grid gap-8 xl:grid-cols-[1fr_260px]">
      <div className="min-w-0 space-y-8">
        <section>
          <SectionTitle hint={<Link className="text-accent" href={`/approvals/?site=${site}`}>Write a draft →</Link>}>Variants</SectionTitle>
          <div className="flex flex-wrap gap-3">
            {shown.map((v) => <VariantCard key={v.id} v={v} index={s.variants.indexOf(v)} champion={v.key === s.champion && v.status === 'approved'} />)}
            <Link href={`/approvals/?site=${site}&slot=${slot}`} className="grid min-h-40 w-40 shrink-0 place-items-center rounded-lg border border-dashed border-line-strong text-center text-xs text-muted hover:border-accent hover:text-accent">
              + next draft
            </Link>
          </div>
          <p className="mt-2 text-xs text-muted">Per-variant lift compares the variant’s visitors with the holdout. The bandit chose who saw it by context, so it is descriptive; the slot’s overall lift below is the causal number.</p>
        </section>
        <section>
          <SectionTitle hint={<span>slot lift <LiftPill lift={s.lift} small /></span>}>Lift over time</SectionTitle>
          <Card className="p-4">
            <LiftCurve curve={s.curve} series={series} />
          </Card>
        </section>
        <section>
          <SectionTitle>Decisions</SectionTitle>
          <DecisionsTable site={site} slot={slot} variants={s.variants} />
        </section>
      </div>
      <aside className="space-y-4 text-[13px]">
        <Card className="p-4">
          <SectionTitle>Audiences</SectionTitle>
          <p className="text-ink-2">No audiences gate this slot yet, so every approved variant is eligible for every visitor{live.some((v) => v.traffic_share !== null) ? ', within its planned share' : ''}.</p>
          <ul className="mt-3 space-y-1.5 text-xs text-ink-2">
            {live.map((v) => (
              <li key={v.id}>
                <span className="font-mono text-ink">{v.key}</span>: {v.traffic_share !== null ? `at most ${Math.round(v.traffic_share * 100)}% of visitors` : 'all visitors'}
              </li>
            ))}
          </ul>
          <p className="mt-3 text-xs text-muted">Audiences arrive with the audience editor (Phase 1b).</p>
        </Card>
        <Card className="space-y-1.5 p-4 text-xs text-ink-2">
          <SectionTitle>Slot</SectionTitle>
          <div>Metric <b className="font-mono text-ink">{s.metric}</b></div>
          <div>Attribution window {Math.round(s.attribution_window_secs / 60)} min</div>
          <div>Sticky for {s.decision_ttl_secs} s per visitor</div>
          <div>Model version <span className="font-mono text-ink">v{s.model_version}</span></div>
          <div className="tabular">Policy {int(s.treated.n)} · holdout {int(s.holdout.n)} closed decisions</div>
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
      title={
        <>
          <div className="text-xs text-muted"><Link href={`/site/?id=${site}`} className="hover:underline">{site}</Link> / slot</div>
          <h1 className="font-mono text-2xl font-semibold">{slot}</h1>
        </>
      }
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
