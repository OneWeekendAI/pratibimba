import type { Interval } from './api';

/** Percentage points, signed: 0.034 -> "+3.4 pp". */
export function pp(x: number, digits = 1) {
  const v = x * 100;
  return `${v >= 0 ? '+' : '−'}${Math.abs(v).toFixed(digits)} pp`;
}

export function pct(x: number | null | undefined, digits = 1) {
  return x === null || x === undefined ? '—' : `${(x * 100).toFixed(digits)}%`;
}

export function int(n: number) {
  return n.toLocaleString('en-US');
}

/** Decisions whose attribution window is still open: real traffic, not yet in any rate (S8). */
export function inWindow(n: number) {
  return `${int(n)} ${n === 1 ? 'decision' : 'decisions'} still in their window`;
}

export function compact(n: number) {
  return Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 }).format(n);
}

/** "−0.9 to +1.2 pp" */
export function band(i: Interval) {
  return `${pp(i.lo)} to ${pp(i.hi)}`;
}

/** What the interval says, in words. */
export function verdict(i: Interval | null): 'better' | 'worse' | 'unclear' | 'none' {
  if (!i) return 'none';
  if (i.lo > 0) return 'better';
  if (i.hi < 0) return 'worse';
  return 'unclear';
}

export function when(iso: string | null | undefined) {
  if (!iso) return '—';
  const d = new Date(iso);
  return d.toISOString().replace('T', ' ').slice(0, 16) + ' UTC';
}

export function ago(iso: string, now = Date.now()) {
  const s = Math.max(0, Math.round((now - new Date(iso).getTime()) / 1000));
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  return `${Math.round(s / 86400)}d ago`;
}

export function shortId(id: string) {
  return id.length > 12 ? `${id.slice(0, 8)}…` : id;
}

/** Fixed categorical slot per variant, by its order in the slot (never by rank). */
export function seriesVar(index: number) {
  return `var(--s${(index % 8) + 1})`;
}

/**
 * Axis ticks on round values: a step of 1, 2 or 5 times a power of ten, about `target` of
 * them, covering [lo, hi] and always including zero when the range spans it.
 */
export function niceTicks(lo: number, hi: number, target = 5): number[] {
  if (!(hi > lo)) return [lo];
  const raw = (hi - lo) / Math.max(1, target);
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 5, 10].map((m) => m * mag).find((s) => s >= raw) ?? 10 * mag;
  const first = Math.floor(lo / step) * step;
  const out: number[] = [];
  for (let v = first; v < hi + step * 0.5; v += step) out.push(Number(v.toFixed(10)) + 0);
  return out;
}
