'use client';
// Charts follow the house dataviz rules: one axis, 2px lines, band washes at ~12%, a legend
// for two or more series, values in text ink, the variant's colour fixed by its order.
import { useMemo } from 'react';
import { Area, CartesianGrid, Cell, ComposedChart, ErrorBar, Line, ReferenceLine, ResponsiveContainer, Scatter, Tooltip, XAxis, YAxis } from 'recharts';
import type { CurvePoint, Interval } from '@/lib/api';
import { niceTicks, pp, seriesVar } from '@/lib/format';

export function Sparkline({ values, label }: { values: number[]; label: string }) {
  const max = Math.max(1, ...values);
  const w = 160;
  const h = 28;
  const step = w / Math.max(1, values.length - 1);
  const d = values.map((v, i) => `${i === 0 ? 'M' : 'L'}${(i * step).toFixed(1)},${(h - 2 - (v / max) * (h - 4)).toFixed(1)}`).join(' ');
  return (
    <svg role="img" aria-label={label} viewBox={`0 0 ${w} ${h}`} className="h-7 w-full" preserveAspectRatio="none">
      <path d={d} fill="none" stroke="var(--muted)" strokeWidth={1.5} strokeLinejoin="round" strokeLinecap="round" vectorEffect="non-scaling-stroke" />
      {values.length > 0 && <circle cx={(values.length - 1) * step} cy={h - 2 - (values[values.length - 1] / max) * (h - 4)} r={2.5} fill="var(--accent)" />}
    </svg>
  );
}

/** A stacked share bar with 2px surface gaps between segments. */
export function TrafficBar({ parts }: { parts: { key: string; share: number; index: number }[] }) {
  const total = parts.reduce((a, p) => a + p.share, 0);
  if (total <= 0) return <div className="h-2 rounded-full bg-surface-2" title="no traffic yet" />;
  return (
    <div className="flex h-2 w-full gap-[2px] overflow-hidden rounded-full" role="img" aria-label={parts.map((p) => `${p.key} ${Math.round(p.share * 100)}%`).join(', ')}>
      {parts.filter((p) => p.share > 0).map((p) => (
        <div key={p.key} title={`${p.key}: ${Math.round((p.share / total) * 100)}%`} style={{ width: `${(p.share / total) * 100}%`, background: seriesVar(p.index) }} />
      ))}
    </div>
  );
}

interface Series {
  key: string;
  index: number;
}

/**
 * Cumulative lift against the holdout over time, one line per variant with its confidence
 * sequence as a wash. Zero is the holdout: a variant above zero beats showing nothing new.
 */
export function LiftCurve({ curve, series, height = 260 }: { curve: CurvePoint[]; series: Series[]; height?: number }) {
  const data = useMemo(
    () =>
      curve.map((p) => {
        const row: Record<string, number | string | [number, number] | null> = { t: new Date(p.t).getTime() };
        for (const s of series) {
          const i = p.variants[s.key];
          row[s.key] = i ? i.estimate * 100 : null;
          row[`${s.key}_band`] = i ? [i.lo * 100, i.hi * 100] : null;
        }
        return row;
      }),
    [curve, series],
  );
  // The first hours carry huge bands; scale to the later half so the readable part is not
  // flattened, and let the early overflow clip at the edge.
  const domain = useMemo(() => {
    const tail = curve.slice(Math.floor(curve.length / 2));
    let lo = 0;
    let hi = 0;
    for (const p of tail) for (const s of series) { const i = p.variants[s.key]; if (i) { lo = Math.min(lo, i.lo * 100); hi = Math.max(hi, i.hi * 100); } }
    const pad = Math.max(1, (hi - lo) * 0.15);
    // Round values on the axis: the domain snaps out to the tick step.
    const ticks = niceTicks(lo - pad, hi + pad, 5);
    return { range: [ticks[0], ticks[ticks.length - 1]] as [number, number], ticks };
  }, [curve, series]);
  const dayTicks = useMemo(() => {
    if (curve.length === 0) return [];
    const first = new Date(curve[0].t).getTime();
    const last = new Date(curve[curve.length - 1].t).getTime();
    const out: number[] = [];
    const d = new Date(first);
    d.setUTCHours(0, 0, 0, 0);
    for (let t = d.getTime() + 86_400_000; t <= last; t += 86_400_000) out.push(t);
    return out.length >= 2 ? out : undefined;
  }, [curve]);
  // Nothing to draw (no closed window yet, or no holdout to compare with): one short line, not
  // an empty frame of axes.
  if (curve.length === 0 || series.length === 0) {
    return <p className="text-[13px] text-muted" data-testid="curve-empty">{curve.length === 0 ? 'No closed windows yet.' : 'No holdout to compare with yet.'}</p>;
  }
  const last = curve[curve.length - 1];
  // One hour of closed windows is one point, and a line needs two: draw each variant's lift so
  // far as a dot with its confidence sequence as an error bar, instead of an invisible line.
  if (curve.length === 1) return <LiftDots point={last} series={series} height={height} domain={domain} />;
  return (
    <div>
      <div style={{ height }} role="img" aria-label="Cumulative lift against the holdout, per variant, with 95% confidence sequences">
        <ResponsiveContainer width="100%" height="100%">
          <ComposedChart data={data} margin={{ top: 8, right: 16, bottom: 0, left: 0 }}>
            <CartesianGrid stroke="var(--grid)" vertical={false} />
            <XAxis
              dataKey="t"
              type="number"
              domain={['dataMin', 'dataMax']}
              scale="time"
              ticks={dayTicks}
              minTickGap={24}
              tickFormatter={(t) => new Date(t).toISOString().slice(5, dayTicks ? 10 : 16).replace('T', ' ')}
              stroke="var(--axis)"
              tick={{ fill: 'var(--muted)', fontSize: 11 }}
              tickLine={false}
            />
            <YAxis
              domain={domain.range}
              ticks={domain.ticks}
              interval={0}
              allowDataOverflow
              stroke="var(--axis)"
              tick={{ fill: 'var(--muted)', fontSize: 11 }}
              tickLine={false}
              axisLine={false}
              width={48}
              tickFormatter={(v: number) => `${v > 0 ? '+' : v < 0 ? '−' : ''}${Math.abs(v)}`}
              label={{ value: 'pp vs holdout', angle: -90, position: 'insideLeft', fill: 'var(--muted)', fontSize: 11, dx: 6, dy: 40 }}
            />
            <ReferenceLine y={0} stroke="var(--ink-2)" strokeWidth={1} />
            {series.map((s) => (
              <Area key={`${s.key}_band`} dataKey={`${s.key}_band`} stroke="none" fill={seriesVar(s.index)} fillOpacity={0.12} isAnimationActive={false} connectNulls />
            ))}
            {series.map((s) => (
              <Line key={s.key} dataKey={s.key} stroke={seriesVar(s.index)} strokeWidth={2} dot={false} isAnimationActive={false} connectNulls />
            ))}
            <Tooltip
              cursor={{ stroke: 'var(--axis)', strokeWidth: 1 }}
              content={({ active, payload, label }) =>
                active && payload ? (
                  <div className="rounded-md border border-line bg-surface px-3 py-2 text-xs shadow-sm">
                    <div className="mb-1 text-muted">{new Date(label as number).toISOString().slice(0, 16).replace('T', ' ')} UTC</div>
                    {series.map((s) => {
                      const row = payload[0]?.payload as Record<string, unknown>;
                      const v = row?.[s.key] as number | null;
                      const b = row?.[`${s.key}_band`] as [number, number] | null;
                      return (
                        <div key={s.key} className="flex items-center gap-2 tabular">
                          <span className="inline-block h-0.5 w-3" style={{ background: seriesVar(s.index) }} />
                          <b className="text-ink">{v === null || v === undefined ? '—' : `${v >= 0 ? '+' : ''}${v.toFixed(1)} pp`}</b>
                          <span className="text-muted">{s.key}</span>
                          {b && <span className="text-muted">({b[0].toFixed(1)} to {b[1].toFixed(1)})</span>}
                        </div>
                      );
                    })}
                  </div>
                ) : null
              }
            />
          </ComposedChart>
        </ResponsiveContainer>
      </div>
      <div className="mt-2 flex flex-wrap gap-x-5 gap-y-1 text-xs text-ink-2">
        {series.map((s) => {
          const i = last.variants[s.key];
          return (
            <span key={s.key} className="inline-flex items-center gap-1.5">
              <span className="inline-block h-0.5 w-4" style={{ background: seriesVar(s.index) }} />
              {s.key}
              {i && <span className="tabular text-muted">{pp(i.estimate)}</span>}
            </span>
          );
        })}
        <span className="inline-flex items-center gap-1.5 text-muted">
          <span className="inline-block h-px w-4 bg-ink-2" /> holdout (zero)
        </span>
      </div>
    </div>
  );
}

/** A one-point curve: each variant's lift so far against the holdout, with its 95% sequence. */
function LiftDots({ point, series, height, domain }: { point: CurvePoint; series: Series[]; height: number; domain: { range: [number, number]; ticks: number[] } }) {
  const rows = series.flatMap((s) => {
    const i = point.variants[s.key];
    return i ? [{ key: s.key, index: s.index, v: i.estimate * 100, err: [(i.estimate - i.lo) * 100, (i.hi - i.estimate) * 100] }] : [];
  });
  return (
    <div>
      <div style={{ height }} role="img" aria-label="Lift against the holdout so far, per variant, with 95% confidence sequences" data-testid="lift-dots">
        <ResponsiveContainer width="100%" height="100%">
          <ComposedChart data={rows} margin={{ top: 8, right: 16, bottom: 0, left: 0 }}>
            <CartesianGrid stroke="var(--grid)" vertical={false} />
            <XAxis dataKey="key" type="category" stroke="var(--axis)" tick={{ fill: 'var(--muted)', fontSize: 11 }} tickLine={false} />
            <YAxis
              domain={domain.range}
              ticks={domain.ticks}
              interval={0}
              allowDataOverflow
              stroke="var(--axis)"
              tick={{ fill: 'var(--muted)', fontSize: 11 }}
              tickLine={false}
              axisLine={false}
              width={48}
              tickFormatter={(v: number) => `${v > 0 ? '+' : v < 0 ? '−' : ''}${Math.abs(v)}`}
              label={{ value: 'pp vs holdout', angle: -90, position: 'insideLeft', fill: 'var(--muted)', fontSize: 11, dx: 6, dy: 40 }}
            />
            <ReferenceLine y={0} stroke="var(--ink-2)" strokeWidth={1} />
            <Scatter dataKey="v" isAnimationActive={false}>
              {rows.map((r) => <Cell key={r.key} fill={seriesVar(r.index)} />)}
              <ErrorBar dataKey="err" width={10} strokeWidth={1.5} stroke="var(--ink-2)" direction="y" />
            </Scatter>
          </ComposedChart>
        </ResponsiveContainer>
      </div>
      <p className="mt-2 text-xs text-muted">One hour of closed windows so far; the line over time starts with the second.</p>
    </div>
  );
}

/** The lift interval as a pill with words, never colour alone. */
export function LiftPill({ lift, small }: { lift: Interval | null; small?: boolean }) {
  if (!lift) return <span className="text-xs text-muted">no closed windows yet</span>;
  const tone = lift.lo > 0 ? 'text-good-ink' : lift.hi < 0 ? 'text-pause-ink' : 'text-ink-2';
  const mark = lift.lo > 0 ? '▲' : lift.hi < 0 ? '▼' : '≈';
  return (
    <span className={`tabular inline-flex items-baseline gap-1.5 ${small ? 'text-xs' : 'text-[13px]'}`}>
      <span className={`font-semibold ${tone}`}>
        {mark} {pp(lift.estimate)}
      </span>
      <span className="text-muted">
        95% CS {pp(lift.lo)} to {pp(lift.hi)}
      </span>
    </span>
  );
}
