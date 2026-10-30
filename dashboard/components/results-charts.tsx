'use client';
// The two Results charts, plain SVG-free HTML so they theme with the tokens and need no Recharts:
// a two-bar comparison for the site, and a ranked dot-and-range list per slot. One idea each;
// the leader in the accent, everything else grey.
import { useState } from 'react';
import { ChevronDown } from 'lucide-react';
import { Tip } from '@/components/ui';
import type { Interval } from '@/lib/api';
import { human, signed1000 } from '@/lib/plain';

const LEAD = 'var(--s1)';
// The zero line: the comparison group, who always see the original.
const ZERO = 'color-mix(in srgb, var(--muted) 60%, transparent)';

/** With personalisation vs the comparison group: two thin bars on one 0-based scale. */
export function RateBars({ treated, holdout, rangeT, rangeH, unitMany }: { treated: number; holdout: number; rangeT?: Interval | null; rangeH?: Interval | null; unitMany: string }) {
  const max = Math.max(treated, holdout, 1e-9);
  const rows = [
    { key: 'with', label: 'With personalisation', v: treated, r: rangeT, color: LEAD, ink: 'text-ink font-medium' },
    { key: 'without', label: 'Original for everyone', v: holdout, r: rangeH, color: 'var(--axis)', ink: 'text-ink-2' },
  ];
  return (
    <figure className="m-0" data-testid="rate-bars">
      <figcaption className="mb-3 text-[12px] text-muted">Out of every 100 visits, how many led to {unitMany}</figcaption>
      <div className="space-y-3">
        {rows.map((r) => (
          <Tip key={r.key} className="block" label={<>{r.label}: {(r.v * 100).toFixed(1)} in 100{r.r ? <>, likely between {(r.r.lo * 100).toFixed(1)} and {(r.r.hi * 100).toFixed(1)}</> : null}.</>}>
            <div tabIndex={0} className="rounded-sm">
              <div className="mb-1 flex items-baseline justify-between gap-3 text-[13px]">
                <span className={r.ink}>{r.label}</span>
                <span className={`tabular ${r.ink}`}>{(r.v * 100).toFixed(1)}</span>
              </div>
              <div className="h-2 w-full rounded-full bg-surface-2">
                <div className="h-2 rounded-full" style={{ width: `${Math.max(1, (r.v / max) * 100)}%`, background: r.color }} />
              </div>
            </div>
          </Tip>
        ))}
      </div>
    </figure>
  );
}

export interface RankRow {
  key: string;
  lift: Interval;
  isOriginal: boolean;
  n: number;
  conversions: number;
  audience?: string | null;
}

/**
 * Each version against the original, per 1,000 visits: a dot for the best guess, a bar for the
 * likely range, a hairline at zero. Ranked best first; the top three show, the rest fold away.
 */
export function VersionRanks({ rows, unitMany, testid }: { rows: RankRow[]; unitMany: string; testid?: string }) {
  const [all, setAll] = useState(false);
  const sorted = [...rows].sort((a, b) => b.lift.estimate - a.lift.estimate);
  const lo = Math.min(0, ...sorted.map((r) => r.lift.lo));
  const hi = Math.max(0, ...sorted.map((r) => r.lift.hi));
  const pad = (hi - lo) * 0.04 || 0.01;
  const x = (v: number) => ((v - (lo - pad)) / (hi - lo + 2 * pad)) * 100;
  const TOP = 3;
  const shown = all || sorted.length <= TOP + 1 ? sorted : sorted.slice(0, TOP);
  const hidden = sorted.length - shown.length;
  return (
    <figure className="m-0" data-testid={testid}>
      <div className="space-y-0.5" role="list" aria-label={`Each version against the comparison group, extra ${unitMany} per 1,000 visits`}>
        {shown.map((r, i) => {
          const lead = i === 0;
          return (
            <Tip
              key={r.key}
              className="block"
              width={280}
              label={
                <>
                  {human(r.key)}{r.isOriginal ? ' (the original)' : ''}: {signed1000(r.lift.estimate)} {unitMany} per 1,000 visits, likely between {signed1000(r.lift.lo)} and {signed1000(r.lift.hi)}.
                  {' '}Shown {r.n.toLocaleString('en-US')} times, {r.conversions.toLocaleString('en-US')} {unitMany}.
                </>
              }
            >
              <div tabIndex={0} role="listitem" data-testid="version-row" data-key={r.key} className="grid grid-cols-[minmax(0,8.5rem)_minmax(0,1fr)_3rem] items-center gap-3 rounded-sm py-1 sm:grid-cols-[minmax(0,11rem)_minmax(0,1fr)_3.25rem]">
                <span className={`text-[13px] leading-tight break-words ${lead ? 'font-medium text-ink' : 'text-ink-2'}`}>
                  {human(r.key)}
                  {r.isOriginal && <span className="whitespace-nowrap text-muted"> · original</span>}
                </span>
                <span className="relative block h-5" aria-hidden>
                  <span className="absolute inset-y-0 w-px" style={{ left: `${x(0)}%`, background: ZERO }} />
                  <span
                    className="absolute top-1/2 h-1.5 -translate-y-1/2 rounded-full"
                    style={{ left: `${x(r.lift.lo)}%`, width: `${Math.max(0.5, x(r.lift.hi) - x(r.lift.lo))}%`, background: lead ? `color-mix(in srgb, ${LEAD} 28%, transparent)` : 'var(--surface-3)' }}
                  />
                  <span
                    className="absolute top-1/2 size-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full"
                    style={{ left: `${x(r.lift.estimate)}%`, background: lead ? LEAD : 'var(--muted)', boxShadow: '0 0 0 2px var(--surface)' }}
                  />
                </span>
                <span className={`text-right text-[13px] tabular ${lead ? 'font-medium text-ink' : 'text-ink-2'}`}>{signed1000(r.lift.estimate)}</span>
              </div>
            </Tip>
          );
        })}
      </div>
      <div className="mt-2 flex flex-wrap items-center justify-between gap-x-4 gap-y-1 text-[12px] text-muted">
        <span className="inline-flex items-center gap-3">
          <span className="inline-flex items-center gap-1.5"><span className="size-2 rounded-full bg-[color:var(--muted)]" />best guess</span>
          <span className="inline-flex items-center gap-1.5"><span className="h-1.5 w-4 rounded-full bg-surface-3" />likely range</span>
          <span className="inline-flex items-center gap-1.5"><span className="h-3 w-px" style={{ background: ZERO }} />comparison group</span>
        </span>
        {(hidden > 0 || all) && sorted.length > TOP + 1 && (
          <button type="button" onClick={() => setAll((a) => !a)} aria-expanded={all} className="inline-flex items-center gap-1 rounded-sm text-ink-2 hover:text-ink">
            {all ? 'Show top 3' : `Show ${hidden} more`}
            <ChevronDown className={`size-3.5 transition-transform ${all ? 'rotate-180' : ''}`} aria-hidden />
          </button>
        )}
      </div>
    </figure>
  );
}
