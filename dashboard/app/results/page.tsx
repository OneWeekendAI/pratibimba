'use client';
// Findings: a reading view of the live state, not a weekly report.
import Link from 'next/link';
import { useQuery } from '@tanstack/react-query';
import { Suspense } from 'react';
import { LiftCurve, LiftPill } from '@/components/charts';
import { Shell } from '@/components/shell';
import { Card, Chip, Empty, Loading, Prose, SectionTitle } from '@/components/ui';
import { api, type Results } from '@/lib/api';
import { ago, compact, inWindow, int, pp } from '@/lib/format';
import { useParam } from '@/lib/page';

function money(i: { estimate: number } | null | undefined, currency = 'USD') {
  if (!i) return '—';
  return new Intl.NumberFormat(undefined, { style: 'currency', currency, maximumFractionDigits: 2 }).format(i.estimate);
}

function View() {
  const site = useParam('site')!;
  const q = useQuery({ queryKey: ['results', site], queryFn: () => api.get<Results>(`/sites/${site}/results`), refetchInterval: 30_000 });
  if (q.isLoading || !q.data) return <Loading />;
  const { results: r, curves, learnings, decisions_total } = q.data;
  // Ranked by lift contribution: the slot's lift times its share of treated visitors.
  const ranked = [...r.slots].sort((a, b) => (b.lift?.estimate ?? -Infinity) * b.treated.n - (a.lift?.estimate ?? -Infinity) * a.treated.n);
  const headline = r.lift;
  return (
    <div className="space-y-8">
      <Card className="p-6" data-testid="headline">
        <div className="text-xs text-ink-2">Every slot’s policy against the {Math.round(r.holdout_fraction * 100)}% holdout, all time</div>
        <div className="mt-2 text-[52px] leading-none font-semibold tracking-tight tabular">{headline ? pp(headline.estimate) : '—'}</div>
        <p className="mt-3 max-w-2xl text-[15px] text-ink-2">
          {headline
            ? <>conversion vs holdout, 95% confidence sequence {pp(headline.lo)} to {pp(headline.hi)}, over {compact(r.treated.n + r.holdout.n)} closed decisions ({compact(decisions_total)} in all). {headline.lo > 0 ? 'Personalisation is beating showing everyone the baseline.' : headline.hi < 0 ? 'Personalisation is losing to the baseline.' : 'Not yet distinguishable from the baseline; the band narrows as windows close.'}</>
            : 'No attribution window has closed yet.'}
          {(r.pending?.decisions ?? 0) > 0 && <> {inWindow(r.pending!.decisions)}.</>}
        </p>
        <p className="mt-2 text-xs text-muted">A confidence sequence stays valid however often you look, so this number is as honest at 3 pm as at 9 am.</p>
      </Card>
      <section>
        <SectionTitle hint="ranked by lift × traffic">Slots</SectionTitle>
        {ranked.length === 0 && <Empty>No slots.</Empty>}
        <div className="grid items-start gap-4 lg:grid-cols-2">
          {ranked.map((s) => {
            const idx = new Map(s.variants.map((v, i) => [v.key, i]));
            const series = s.variants.map((v) => ({ key: v.key, index: idx.get(v.key)! })).filter((x) => (curves[s.key] ?? []).some((p) => p.variants[x.key]));
            return (
              <Card key={s.key} className="p-4" data-testid={`result-${s.key}`}>
                <div className="mb-1 flex items-baseline justify-between">
                  <Link href={`/slot/?site=${site}&slot=${s.key}`} className="font-mono font-semibold hover:underline">{s.key}</Link>
                  <span className="text-xs text-muted tabular">{int(s.treated.n)} policy · {int(s.holdout.n)} holdout</span>
                </div>
                <div className="mb-3"><LiftPill lift={s.lift} /></div>
                {(s.pending?.decisions ?? 0) > 0 && (
                  <p className="mb-3 text-xs text-ink-2 tabular" data-testid={`pending-${s.key}`}>
                    {inWindow(s.pending!.decisions)} ({int(s.pending!.exposures)} shown, {int(s.pending!.conversions)} converted so far), counted when it closes
                  </p>
                )}
                {(s.treated.revenue ?? 0) + (s.holdout.revenue ?? 0) > 0 && (
                  <p className="mb-3 text-xs text-ink-2 tabular" data-testid={`revenue-${s.key}`}>
                    Revenue per visitor {money(s.treated.revenue_per_visitor, r.currency)} vs {money(s.holdout.revenue_per_visitor, r.currency)} holdout
                    {s.revenue_lift && <> · lift {money(s.revenue_lift, r.currency)} ({money({ estimate: s.revenue_lift.lo }, r.currency)} to {money({ estimate: s.revenue_lift.hi }, r.currency)})</>}
                    {s.goal === 'revenue' && <> · optimising revenue</>}
                  </p>
                )}
                {/* No closed window (or no holdout) yet: the pill above already says so in one line. */}
                {series.length > 0 && <LiftCurve curve={curves[s.key] ?? []} series={series} height={160} />}
              </Card>
            );
          })}
        </div>
      </section>
      <section>
        <SectionTitle hint="written when evidence crosses a threshold, not on a schedule">Learnings</SectionTitle>
        {learnings.length === 0 ? (
          <Empty>Nothing durable yet. A sentence lands here when a variant clearly beats the holdout or is paused by auto-rollback.</Empty>
        ) : (
          <Card className="divide-y divide-line">
            {learnings.map((l) => (
              <div key={l.id} className="flex gap-3 px-4 py-3 text-[13.5px]" data-testid="learning">
                <span className="pt-0.5">{l.kind === 'rollback' ? <Chip tone="paused">rollback</Chip> : <Chip tone="live">winning</Chip>}</span>
                <p className="flex-1 text-ink"><Prose text={l.sentence} /></p>
                <span className="shrink-0 text-xs text-muted">{ago(l.at)}</span>
              </div>
            ))}
          </Card>
        )}
      </section>
    </div>
  );
}

function Page() {
  const site = useParam('site');
  return (
    <Shell site={site} title={<><div className="text-xs text-muted">{site}</div><h1 className="font-serif text-3xl">Results</h1></>}>
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
