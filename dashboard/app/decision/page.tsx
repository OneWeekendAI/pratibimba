'use client';
// One decision, one URL, one plain-English sentence; then the evidence for it.
import Link from 'next/link';
import { useQuery } from '@tanstack/react-query';
import { Suspense, useState } from 'react';
import { Preview } from '@/components/preview';
import { Shell } from '@/components/shell';
import { Button, Card, Chip, Empty, Loading, Mono, Prose, SectionTitle } from '@/components/ui';
import { api, ApiError, type DecisionDetail } from '@/lib/api';
import { pct, when } from '@/lib/format';
import { useParam } from '@/lib/page';

function ScoreBars({ d }: { d: DecisionDetail }) {
  const max = Math.max(...d.scores.map((s) => s.score), 0.0001);
  return (
    <div className="space-y-2.5" role="table" aria-label="Arm scores">
      {d.scores.map((s) => {
        const shown = s.variant === d.variant;
        return (
          <div key={s.variant_id} role="row" className="grid grid-cols-[120px_1fr_56px] items-center gap-3">
            <span role="cell" className={`truncate font-mono text-[12.5px] ${shown ? 'font-semibold text-ink' : 'text-ink-2'}`}>{s.variant}</span>
            <div role="cell" className="h-3 rounded-r bg-surface-2" title={`${s.variant}: ${s.score.toFixed(3)}`}>
              <div className="h-3 rounded-r" style={{ width: `${(s.score / max) * 100}%`, maxWidth: '100%', background: shown ? 'var(--accent)' : 'var(--axis)' }} />
            </div>
            <span role="cell" className="tabular text-right text-[12.5px]">{s.score.toFixed(3)}</span>
          </div>
        );
      })}
      <p className="pt-1 text-xs text-muted">One Thompson draw per arm with seed <Mono>{d.seed}</Mono>; the highest score wins{d.in_holdout ? ' (a holdout visitor gets the baseline regardless)' : ''}.</p>
    </div>
  );
}

function Counterfactual({ d }: { d: DecisionDetail }) {
  const [open, setOpen] = useState(false);
  const c = d.counterfactual;
  if (!c) return null;
  const question = c.kind === 'baseline' ? 'What would they have seen under the baseline?' : 'What would the policy have shown them?';
  return (
    <Card className="p-4">
      <SectionTitle>Counterfactual</SectionTitle>
      {!open ? (
        <Button variant="ghost" onClick={() => setOpen(true)}>{question}</Button>
      ) : (
        <div data-testid="counterfactual">
          <p className="mb-3 text-[13px] text-ink-2">
            Same seed, holdout flag flipped: <b className="font-mono text-ink">{c.variant}</b>
            {c.kind === 'baseline' ? ' (the baseline).' : ' (the policy’s pick).'} The model at v{d.policy_version} expected{' '}
            <b className="text-ink">{pct(c.inferred_rate)}</b> conversion for this visitor there, against <b className="text-ink">{pct(c.shown_inferred_rate)}</b> for <span className="font-mono">{d.variant}</span>.
          </p>
          <div className="grid gap-3 md:grid-cols-2">
            <div><div className="mb-1 text-xs text-muted">Shown: {d.variant}</div><Preview html={d.is_baseline ? null : d.payload} label={`shown ${d.variant}`} /></div>
            <div><div className="mb-1 text-xs text-muted">Counterfactual: {c.variant}</div><Preview html={c.is_baseline ? null : c.payload} label={`counterfactual ${c.variant}`} /></div>
          </div>
        </div>
      )}
    </Card>
  );
}

function Detail() {
  const id = useParam('id')!;
  const q = useQuery({ queryKey: ['decision', id], queryFn: () => api.get<DecisionDetail>(`/decisions/${id}`) });
  if (q.error instanceof ApiError && q.error.status === 404) return <Empty>No decision {id}.</Empty>;
  if (q.isLoading || !q.data) return <Loading />;
  const d = q.data;
  const named = { bias: 'Bias (always 1)', returning: 'Returning visitor', mobile: 'On mobile', from_ad: 'Arrived from an ad' } as Record<string, string>;
  return (
    <div className="max-w-4xl space-y-6">
      <p className="font-serif text-[21px] leading-relaxed text-ink" data-testid="why"><Prose text={d.sentence} /></p>
      <div className="flex flex-wrap gap-2">
        {d.in_holdout ? <Chip tone="off">holdout</Chip> : <Chip tone="accent">policy</Chip>}
        {d.replay_matches === true && <Chip tone="live" title="Re-running the policy on the stored model version, seed and pool gives these exact scores">replay reproduces the scores</Chip>}
        {d.replay_matches === false && <Chip tone="paused">replay does not match</Chip>}
        <Chip tone="off">{d.policy === 'lints' ? 'LinTS' : d.policy} v{d.policy_version}</Chip>
      </div>
      <div className="grid gap-4 md:grid-cols-2">
        <Card className="p-4">
          <SectionTitle>What the bandit saw</SectionTitle>
          <table className="w-full text-[13px]">
            <tbody>
              {Object.entries(d.features).map(([k, v]) => (
                <tr key={k} className="border-b border-line last:border-0">
                  <td className="py-1.5 text-ink-2">{named[k] ?? k}</td>
                  <td className="py-1.5 font-mono text-xs text-muted">{k}</td>
                  <td className="tabular py-1.5 text-right">{v}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="mt-3 text-xs text-muted">Audiences: none matched; this site has none yet.</p>
        </Card>
        <Card className="p-4">
          <SectionTitle>Arm scores</SectionTitle>
          <ScoreBars d={d} />
        </Card>
      </div>
      <Counterfactual d={d} />
      {d.review.length > 0 && (
        <Card className="p-4">
          <SectionTitle>At the approval gate</SectionTitle>
          <ul className="space-y-1.5 text-[13px]" data-testid="review">
            {d.review.map((r, i) => (
              <li key={i}>
                <b className="font-medium">{r.actor}</b>{' '}
                <span className="text-muted">{r.action === 'approve' ? 'approved' : r.action === 'reject' ? 'rejected' : 'commented'} <span className="font-mono">{d.variant}</span> {when(r.at)}</span>
                {r.comment && <span className="text-ink-2">: “{r.comment}”</span>}
              </li>
            ))}
          </ul>
        </Card>
      )}
      <Card className="p-4">
        <SectionTitle>Timeline</SectionTitle>
        <ol className="space-y-2 text-[13px]" data-testid="timeline">
          <li className="flex gap-3"><span className="w-44 shrink-0 text-xs text-muted tabular">{when(d.created_at)}</span>decided <span className="font-mono">{d.variant}</span></li>
          {d.timeline.map((e, i) => (
            <li key={i} className="flex gap-3"><span className="w-44 shrink-0 text-xs text-muted tabular">{when(e.at)}</span><span className="font-mono">{e.name}</span></li>
          ))}
          {d.outcome !== null && (
            <li className="flex gap-3"><span className="w-44 shrink-0 text-xs text-muted tabular">{when(d.outcome_at ?? d.settled_at)}</span>{d.outcome === 1 ? `credited as a conversion (${d.metric})` : 'settled as a miss'}</li>
          )}
          {d.outcome === null && <li className="flex gap-3 text-muted"><span className="w-44 shrink-0 text-xs tabular">{when(d.window_closes_at)}</span>attribution window closes</li>}
        </ol>
      </Card>
    </div>
  );
}

function Page() {
  const id = useParam('id');
  const site = useParam('site');
  return (
    <Shell site={site} title={<><div className="text-xs text-muted">{site && <Link href={`/site/?id=${site}`} className="hover:underline">{site}</Link>} / decision</div><h1 className="font-mono text-xl">{id}</h1></>}>
      {id ? <Detail /> : <Empty>No decision given.</Empty>}
    </Shell>
  );
}

export default function DecisionPage() {
  return (
    <Suspense>
      <Page />
    </Suspense>
  );
}
