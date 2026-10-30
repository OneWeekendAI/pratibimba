// Plain-language readings of the Results numbers, for someone who has never seen a confidence
// interval. Every number here is derived from the API response; nothing is assumed.
import type { Interval } from './api';

/** What one conversion is called, from the slot's metric name. */
export function unit(metric: string | null | undefined): { one: string; many: string } {
  const m = (metric ?? '').toLowerCase().replace(/[\s-]+/g, '_');
  if (m === 'add_to_cart' || m === 'atc' || m === 'cart') return { one: 'add-to-cart', many: 'add-to-carts' };
  if (m === 'click' || m === 'clicks' || m === 'ctr') return { one: 'click', many: 'clicks' };
  if (/order|purchase|checkout|buy/.test(m)) return { one: 'order', many: 'orders' };
  if (/sign_?up|register|subscribe/.test(m)) return { one: 'sign-up', many: 'sign-ups' };
  return { one: 'conversion', many: 'conversions' };
}

/** "hero_copy" -> "Hero copy"; "welcome_back" -> "Welcome back". */
export function human(key: string) {
  const s = key.replace(/[_-]+/g, ' ').trim();
  return s ? s[0].toUpperCase() + s.slice(1) : key;
}

/** A rate difference as a whole count per 1,000 visits: 0.0576 -> 58. */
export function per1000(x: number) {
  return Math.round(x * 1000);
}

export type Sureness = 'sure-up' | 'leaning-up' | 'flat' | 'unclear' | 'leaning-down' | 'sure-down';

/**
 * How sure the range lets us be. "Sure" means the whole 95% range is on one side of zero.
 * "Fairly sure" means at least 80% of the range's width is on that side. "No difference" means
 * the estimate is under a tenth of the range's half-width: if there is an effect, it is small.
 */
export function sureness(i: Interval): Sureness {
  if (i.lo > 0) return 'sure-up';
  if (i.hi < 0) return 'sure-down';
  const width = i.hi - i.lo;
  if (!(width > 0)) return 'unclear';
  const half = width / 2;
  if (Math.abs(i.estimate) < half / 10) return 'flat';
  const above = i.hi / width;
  if (above >= 0.8) return 'leaning-up';
  if (above <= 0.2) return 'leaning-down';
  return 'unclear';
}

export const SURE_LABEL: Record<Sureness, string> = {
  'sure-up': 'Sure',
  'leaning-up': 'Fairly sure',
  flat: 'No clear difference',
  unclear: 'Too early to tell',
  'leaning-down': 'Fairly sure',
  'sure-down': 'Sure',
};

/**
 * Roughly how many more visits until the range clears zero, if the effect stays where it is.
 * A range narrows about as one over the square root of the visits, so the total needed is the
 * current total times (half-width / |estimate|) squared. The range used here (a confidence
 * sequence) narrows a little slower than that, so treat the answer as a floor. Null when the
 * estimate is too close to zero to project, or the range has already cleared zero.
 */
export function visitsToBeSure(i: Interval, visits: number): number | null {
  if (i.lo > 0 || i.hi < 0 || visits <= 0) return null;
  const half = (i.hi - i.lo) / 2;
  const e = Math.abs(i.estimate);
  if (e < half / 10) return null;
  const more = visits * ((half / e) ** 2 - 1);
  return more > 0 ? roundish(more) : null;
}

/** Two significant figures: 17,834 -> 18,000; 7,912 -> 7,900. */
export function roundish(n: number) {
  if (n < 100) return Math.ceil(n / 10) * 10;
  const mag = 10 ** (Math.floor(Math.log10(n)) - 1);
  return Math.round(n / mag) * mag;
}

/** "about 58 more add-to-carts" / "about 5 fewer clicks" / "about the same number of clicks". */
export function moreOrFewer(x: number, u: { one: string; many: string }) {
  const n = per1000(x);
  if (n === 0) return `about the same number of ${u.many}`;
  const abs = Math.abs(n);
  return `about ${abs.toLocaleString('en-US')} ${n > 0 ? 'more' : 'fewer'} ${abs === 1 ? u.one : u.many}`;
}

/** "+293" / "−12" / "0", per 1,000 visits. */
export function signed1000(x: number) {
  const n = per1000(x);
  return n === 0 ? '0' : `${n > 0 ? '+' : '−'}${Math.abs(n).toLocaleString('en-US')}`;
}
