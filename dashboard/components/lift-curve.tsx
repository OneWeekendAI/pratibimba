'use client';
// The lift curve, the one Recharts view. Pages load it with next/dynamic, so Recharts stays out
// of every page that only shows a lift pill.
import { useMemo } from 'react';
import { Area, CartesianGrid, Cell, ComposedChart, ErrorBar, Line, ReferenceLine, ResponsiveContainer, Scatter, Tooltip, XAxis, YAxis } from 'recharts';
import type { CurvePoint } from '@/lib/api';
import { niceTicks, pp, seriesVar } from '@/lib/format';

interface Series {
  key: string;
  index: number;
}

/**
 * Cumulative lift against the holdout over time, one line per variant with its confidence
 * sequence as a wash. Zero is the holdout: a variant above zero beats showing nothing new.
 */
export function LiftCurve({ curve, series, height = 260, highlight, yLabel = 'pp vs holdout' }: { curve: CurvePoint[]; series: Series[]; height?: number; highlight?: string; yLabel?: string }) {
  // With a highlight, one line carries the accent and the rest sit behind it in grey: one idea,
  // not a legend to decode.
  const hl = highlight && series.some((s) => s.key === highlight) ? highlight : undefined;
  const ordered = hl ? [...series.filter((s) => s.key !== hl), ...series.filter((s) => s.key === hl)] : series;
  const stroke = (s: Series) => (hl ? (s.key === hl ? 'var(--s1)' : 'var(--axis)') : seriesVar(s.index));
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
  if (curve.length === 1) return <LiftDots point={last} series={series} height={height} domain={domain} yLabel={yLabel} />;
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
              label={{ value: yLabel, angle: -90, position: 'insideLeft', fill: 'var(--muted)', fontSize: 11, dx: 6, dy: 40 }}
            />
            <ReferenceLine y={0} stroke="var(--muted)" strokeWidth={1} strokeDasharray="3 3" />
            {/* Many overlapping washes read as noise: past three series, lines only. */}
            {(hl ? ordered.filter((s) => s.key === hl) : series.length <= 3 ? series : []).map((s) => (
              <Area key={`${s.key}_band`} dataKey={`${s.key}_band`} stroke="none" fill={stroke(s)} fillOpacity={0.12} isAnimationActive={false} connectNulls />
            ))}
            {ordered.map((s) => (
              <Line key={s.key} dataKey={s.key} stroke={stroke(s)} strokeWidth={hl && s.key !== hl ? 1.25 : 2} dot={false} isAnimationActive={false} connectNulls />
            ))}
            <Tooltip
              cursor={{ stroke: 'var(--axis)', strokeWidth: 1 }}
              content={({ active, payload, label }) =>
                active && payload ? (
                  <div className="rounded-md bg-surface px-3 py-2 text-xs shadow-pop">
                    <div className="mb-1 text-muted">{new Date(label as number).toISOString().slice(0, 16).replace('T', ' ')} UTC</div>
                    {series.map((s) => {
                      const row = payload[0]?.payload as Record<string, unknown>;
                      const v = row?.[s.key] as number | null;
                      const b = row?.[`${s.key}_band`] as [number, number] | null;
                      return (
                        <div key={s.key} className="flex items-center gap-2 tabular">
                          <span className="inline-block h-0.5 w-3" style={{ background: stroke(s) }} />
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
      <div className="mt-3 flex flex-wrap gap-x-5 gap-y-1 text-[12px] text-ink-2">
        {hl && series.length > 1 && (
          <span className="inline-flex items-center gap-1.5 text-muted">
            <span className="inline-block h-0.5 w-4" style={{ background: 'var(--axis)' }} /> other versions
          </span>
        )}
        {(hl ? series.filter((s) => s.key === hl) : series).map((s) => {
          const i = last.variants[s.key];
          return (
            <span key={s.key} className="inline-flex items-center gap-1.5">
              <span className="inline-block h-0.5 w-4" style={{ background: stroke(s) }} />
              {s.key}
              {i && <span className="tabular text-muted">{pp(i.estimate)}</span>}
            </span>
          );
        })}
        <span className="inline-flex items-center gap-1.5 text-muted">
          <span className="inline-block w-4 border-t border-dashed border-muted" /> {yLabel === 'pp vs holdout' ? 'holdout (zero)' : 'comparison group (zero)'}
        </span>
      </div>
    </div>
  );
}

/** A one-point curve: each variant's lift so far against the holdout, with its 95% sequence. */
function LiftDots({ point, series, height, domain, yLabel }: { point: CurvePoint; series: Series[]; height: number; domain: { range: [number, number]; ticks: number[] }; yLabel: string }) {
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
            <XAxis dataKey="key" type="category" padding={{ left: 56, right: 56 }} stroke="var(--axis)" tick={{ fill: 'var(--muted)', fontSize: 11 }} tickLine={false} />
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
              label={{ value: yLabel, angle: -90, position: 'insideLeft', fill: 'var(--muted)', fontSize: 11, dx: 6, dy: 40 }}
            />
            <ReferenceLine y={0} stroke="var(--muted)" strokeWidth={1} strokeDasharray="3 3" />
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

