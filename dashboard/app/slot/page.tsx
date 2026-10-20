'use client';
// Sheet 02, the slot workbench: the variant strip, the lift curve, the decisions table.
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import { useQuery } from '@tanstack/react-query';
import { createColumnHelper, tableFeatures, useTable } from '@tanstack/react-table';
import { Suspense, useMemo, useState } from 'react';
import { LiftCurve, LiftPill } from '@/components/charts';
import { Audiences } from '@/components/audiences';
import { Preview } from '@/components/preview';
import { ShareEditor } from '@/components/share-editor';
import { Shell } from '@/components/shell';
import { Card, Chip, Empty, Loading, SectionTitle, inputClass, type ChipTone } from '@/components/ui';
import { api, type DecisionRow, type ItemRow, type SlotDetail, type VariantRow } from '@/lib/api';
import { ago, inWindow, int, pct, pp, seriesVar, shortId, when } from '@/lib/format';
import { useParam } from '@/lib/page';

const STATUS: Record<VariantRow['status'], [ChipTone, string]> = {
  approved: ['live', 'live'],
  paused: ['paused', 'paused by rollback'],
  draft: ['warn', 'draft'],
  rejected: ['off', 'rejected'],
  retired: ['off', 'retired'],
};

/** A copy slot's variant (F4): its fields as labelled plain text, in the slot's declared order. */
function CopyFields({ payload, fields }: { payload: string | null; fields: SlotDetail['fields'] }) {
  let text: Record<string, string> = {};
  try {
    text = JSON.parse(payload ?? '{}') ?? {};
  } catch {}
  const names = fields?.map((f) => f.name) ?? Object.keys(text);
  return (
    <dl className="min-h-24 space-y-1.5 rounded-md border border-line bg-surface-2/40 px-3 py-2 text-[12.5px] leading-snug" data-testid="copy-fields">
      {names.map((n) => (
        <div key={n}>
          <dt className="font-mono text-[10.5px] text-muted">{n}</dt>
          <dd className="text-ink">{text[n] ?? <span className="text-muted">—</span>}</dd>
        </div>
      ))}
    </dl>
  );
}

function VariantCard({ v, index, champion, site, slot, fields }: { v: VariantRow; index: number; champion: boolean; site: string; slot: string; fields: SlotDetail['fields'] }) {
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
        {fields && !v.is_baseline ? (
          <CopyFields payload={v.payload} fields={fields} />
        ) : (
          <Preview html={v.is_baseline ? null : v.payload} label={`preview of ${v.key}`} height={96} scale={0.62} />
        )}
        <div className="mt-3 flex items-center justify-between text-xs">
          <Chip tone={tone}>{label}</Chip>
          <span className="tabular text-ink-2">{pct(v.share, 0)} of traffic</span>
        </div>
        <div className="mt-2"><LiftPill lift={v.lift} small /></div>
        <div className="mt-1 text-[11px] text-muted tabular">
          {int(v.counts.n)} closed · {credited(v.counts.credit ?? v.counts.conversions)} converted
          {(v.pending?.decisions ?? 0) > 0 && ` · ${int(v.pending!.decisions)} in window`}
          {v.traffic_share !== null && ` · planned share ${Math.round(v.traffic_share * 100)}%`}
        </div>
        {!v.is_baseline && (v.status === 'approved' || v.status === 'paused') && <ShareEditor site={site} slot={slot} v={v} />}
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

/**
 * G3: a sorted slot's items in the declared order, one numbered row each. Items are not variants:
 * the learner orders them per visitor, so there is no traffic share to set.
 */
function ItemsTable({ items }: { items: ItemRow[] }) {
  const cols = ['#', 'Item', 'Shown', 'Avg position', 'Acted on', 'Rate'];
  return (
    <Card className="overflow-auto">
      <table className="w-full text-[13px]" data-testid="items-table">
        <thead className="border-b border-line text-left text-xs text-muted">
          <tr>{cols.map((c) => <th key={c} className="px-4 py-2 font-medium">{c}</th>)}</tr>
        </thead>
        <tbody>
          {items.map((i, n) => (
            <tr key={i.key} className="border-b border-line last:border-0">
              <td className="w-10 px-4 py-2 text-muted tabular">{n + 1}</td>
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
    <div className="grid gap-8 xl:grid-cols-[1fr_260px]">
      <div className="min-w-0 space-y-8">
        {sorted ? (
          <section>
            <SectionTitle hint="in the order declared; the learner reorders them per visitor">Items</SectionTitle>
            <ItemsTable items={items} />
          </section>
        ) : (
          <section>
            <SectionTitle hint={<Link className="text-accent" href={`/approvals/?site=${site}`}>Write a draft →</Link>}>Variants</SectionTitle>
            <div className="flex flex-wrap gap-3">
              {shown.map((v) => <VariantCard key={v.id} v={v} index={s.variants.indexOf(v)} champion={v.key === s.champion && v.status === 'approved'} site={site} slot={slot} fields={s.kind === 'copy' ? s.fields : null} />)}
              <Link href={`/approvals/?site=${site}&slot=${slot}`} className="grid min-h-40 w-40 shrink-0 place-items-center rounded-lg border border-dashed border-line-strong text-center text-xs text-muted hover:border-accent hover:text-accent">
                + next draft
              </Link>
            </div>
            <p className="mt-2 text-xs text-muted">Per-variant lift compares the variant’s visitors with the holdout. The bandit chose who saw it by context, so it is descriptive; the slot’s overall lift below is the causal number.</p>
          </section>
        )}
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
          <Audiences site={site} slot={slot} variants={s.variants} audiences={s.audiences} />
        </Card>
        <Card className="space-y-1.5 p-4 text-xs text-ink-2">
          <SectionTitle>Slot</SectionTitle>
          <div>Metric <b className="font-mono text-ink">{s.metric}</b></div>
          <div>Attribution window {Math.round(s.attribution_window_secs / 60)} min</div>
          <div>Sticky for {s.decision_ttl_secs} s per visitor</div>
          <div>Model version <span className="font-mono text-ink">v{s.model_version}</span></div>
          <div className="tabular">Policy {int(s.treated.n)} · holdout {int(s.holdout.n)} closed decisions</div>
          {(s.pending?.decisions ?? 0) > 0 && (
            <div className="tabular" data-testid="slot-pending">{inWindow(s.pending!.decisions)}, counted when it closes</div>
          )}
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

/** Credited conversions: whole under last touch, a sum of shares under multi-touch. */
function credited(c: number) {
  return Number.isInteger(c) ? int(c) : c.toFixed(1);
}
