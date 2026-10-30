'use client';
// Charts follow the house dataviz rules: one axis, 2px lines, band washes at ~12%, a legend
// for two or more series, values in text ink, the variant's colour fixed by its order.
import { ArrowDownRight, ArrowUpRight, Equal } from 'lucide-react';
import { Tip } from '@/components/ui';
import type { Interval } from '@/lib/api';
import { pp, seriesVar } from '@/lib/format';

export function Sparkline({ values, label }: { values: number[]; label: string }) {
  const max = Math.max(1, ...values);
  const w = 160;
  const h = 28;
  const step = w / Math.max(1, values.length - 1);
  const d = values.map((v, i) => `${i === 0 ? 'M' : 'L'}${(i * step).toFixed(1)},${(h - 2 - (v / max) * (h - 4)).toFixed(1)}`).join(' ');
  return (
    <svg role="img" aria-label={label} viewBox={`0 0 ${w} ${h}`} className="h-7 w-full" preserveAspectRatio="none">
      <path d={d} fill="none" stroke="var(--ink-2)" strokeWidth={1.25} strokeLinejoin="round" strokeLinecap="round" vectorEffect="non-scaling-stroke" />
      {values.length > 0 && <circle cx={(values.length - 1) * step} cy={h - 2 - (values[values.length - 1] / max) * (h - 4)} r={2.5} fill="var(--ink)" />}
    </svg>
  );
}

/** A stacked share bar with 2px surface gaps between segments. */
export function TrafficBar({ parts }: { parts: { key: string; share: number; index: number }[] }) {
  const total = parts.reduce((a, p) => a + p.share, 0);
  if (total <= 0) return <div className="h-1.5 rounded-full bg-surface-3" title="no traffic yet" />;
  return (
    <div className="flex h-1.5 w-full gap-[2px] overflow-hidden rounded-full" role="img" aria-label={parts.map((p) => `${p.key} ${Math.round(p.share * 100)}%`).join(', ')}>
      {parts.filter((p) => p.share > 0).map((p) => (
        <div key={p.key} title={`${p.key}: ${Math.round((p.share / total) * 100)}%`} style={{ width: `${(p.share / total) * 100}%`, background: seriesVar(p.index) }} />
      ))}
    </div>
  );
}

/** The lift interval: a direction icon and words, never colour alone; the range in a tooltip and in muted text. */
export function LiftPill({ lift, small }: { lift: Interval | null; small?: boolean }) {
  if (!lift) return <span className="text-xs text-muted">no closed windows yet</span>;
  const tone = lift.lo > 0 ? 'text-good-ink' : lift.hi < 0 ? 'text-pause-ink' : 'text-ink-2';
  const Icon = lift.lo > 0 ? ArrowUpRight : lift.hi < 0 ? ArrowDownRight : Equal;
  const word = lift.lo > 0 ? 'better than holdout' : lift.hi < 0 ? 'worse than holdout' : 'not yet clear';
  return (
    <Tip label={<>95% confidence sequence {pp(lift.lo)} to {pp(lift.hi)}. {word[0].toUpperCase() + word.slice(1)}.</>}>
      <span tabIndex={0} className={`tabular inline-flex items-center gap-1.5 whitespace-nowrap ${small ? 'text-xs' : 'text-[13px]'}`}>
        <span className={`inline-flex items-center gap-0.5 font-semibold ${tone}`}>
          <Icon className="size-3.5" aria-label={word} />
          {pp(lift.estimate)}
        </span>
        <span className="text-muted">{pp(lift.lo, 1).replace(' pp', '')} to {pp(lift.hi)}</span>
      </span>
    </Tip>
  );
}
