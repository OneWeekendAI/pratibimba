// `<Slot>`: a server component that decides during the server render, so the HTML that
// leaves the server already holds the chosen variant. No hide, no swap, no flicker.
//
// Every <Slot> in one render shares one `/decide/batch` call: slots ask for their decision,
// and the batch goes out once the render has visited them all (the next macrotask).

import { Children, createElement, cache, isValidElement, type ReactNode } from 'react';
import { PrbBeacon } from './beacon.ts';
import { decideBatch, isDecision, type Ctx, type Decision, type Fetch } from './core.ts';
import { CTX_HEADER, SUBJECT_HEADER, decodeCtx } from './context.ts';
import { TZ_HEADER, validTimeZone } from './timezone.ts';
import { CONSENT_HEADER } from './consent.ts';
import { featuresFor, manifestReader, type FeatureValues } from './features.ts';

type HeaderReader = Headers | { get(name: string): string | null };

export interface Visitor {
  subject: string;
  ctx: Ctx;
}

export interface SlotConfig {
  core: string;
  site: string;
  fetch?: Fetch;
  timeoutMs?: number;
  /** Where the browser posts events; the route file mounts it. */
  eventsPath?: string;
  /** Reads the request headers. Default: Next.js's `headers()`. */
  headers?: () => Promise<HeaderReader>;
  /** One object per request; slots that share it share a batch. Default: React's `cache`. */
  requestScope?: () => object;
  /**
   * The visitor's declared features for this request, name -> on (the server twin of the
   * browser's `window.__prb_features`). Called once per render with the request headers; it
   * may also use Next's `cookies()` or your session. Sent as `ctx.features`: only names the
   * site file declares, only booleans, and a pii one only when the visitor is identified
   * (`full`, or `consent_aware` with consent).
   */
  features?: (headers: HeaderReader) => FeatureValues | undefined | Promise<FeatureValues | undefined>;
}

export interface SlotProps {
  /** The slot key, as declared in the site file. */
  id: string;
  /** Components for variants that live in your repo, by variant key. */
  variants?: Record<string, ReactNode>;
  /** The baseline: what the page shows when no variant replaces it. */
  children?: ReactNode;
}

export interface SortedProps {
  /** The sorted slot's key, as declared in the site file. */
  id: string;
  /**
   * The items, in the page's own (default) order, each with a React `key` equal to its item
   * key in the site file. Children without a key render where they stand.
   */
  children?: ReactNode;
}

export interface CopyProps<F extends string = string> {
  /** The copy slot's key, as declared in the site file (`kind: "copy"`). */
  id: string;
  /** The page's own text for every field: the baseline, and the value of any field a variant leaves out. */
  fallback: Record<F, string>;
  /**
   * Renders the text. Each value is a plain string; render it as a React text child (escaped),
   * never as HTML.
   */
  children: (text: Record<F, string>) => ReactNode;
}

/**
 * The text a copy decision shows: the page's own for each field, overridden by the decided
 * variant's string fields. Non-string values (none come from Core) are ignored.
 */
export function copyText<F extends string>(fallback: Record<F, string>, copy: unknown): Record<F, string> {
  const out = { ...fallback };
  if (copy && typeof copy === 'object') {
    for (const [k, v] of Object.entries(copy as Record<string, unknown>)) if (typeof v === 'string' && k in out) (out as Record<string, string>)[k] = v;
  }
  return out;
}

/**
 * The children in the decided order, each keyed child wrapped in a `display: contents` element
 * marked `data-prb-item`. Keyed children move, in order, to where the first one stood; one the
 * order leaves out (gated for this visitor) follows them in page order. Returns the keys of the
 * ordered items as rendered.
 */
export function arrange(children: ReactNode, order: string[] | undefined): { nodes: ReactNode[]; shown: string[] } {
  const all: ReactNode[] = [];
  Children.forEach(children, (c) => all.push(c));
  const keyOf = (c: ReactNode) => (isValidElement(c) && c.key != null ? String(c.key) : null);
  const items = all.filter((c) => keyOf(c) !== null);
  const byKey = new Map<string, ReactNode>();
  for (const c of items) if (!byKey.has(keyOf(c)!)) byKey.set(keyOf(c)!, c);
  const placed: ReactNode[] = [];
  const shown: string[] = [];
  for (const k of order ?? []) {
    const c = byKey.get(k);
    if (c !== undefined && !placed.includes(c)) {
      placed.push(c);
      shown.push(k);
    }
  }
  for (const c of items) if (!placed.includes(c)) placed.push(c);
  const wrap = (c: ReactNode) => createElement('div', { key: keyOf(c)!, 'data-prb-item': keyOf(c)!, style: { display: 'contents' } }, c);
  const nodes: ReactNode[] = [];
  let done = false;
  for (const c of all) {
    if (keyOf(c) === null) nodes.push(c);
    else if (!done) {
      nodes.push(...placed.map(wrap));
      done = true;
    }
  }
  return { nodes, shown };
}

type Pending = { slot: string; resolve: (d: Decision | null) => void };

class Batch {
  private pending: Pending[] = [];
  private timer: ReturnType<typeof setTimeout> | null = null;
  private config: SlotConfig;
  private visitor: Visitor;
  private ctx: Promise<Ctx>;
  constructor(config: SlotConfig, visitor: Visitor, ctx: Promise<Ctx>) {
    this.config = config;
    this.visitor = visitor;
    this.ctx = ctx;
  }

  decide(slot: string): Promise<Decision | null> {
    return new Promise((resolve) => {
      this.pending.push({ slot, resolve });
      this.timer ??= setTimeout(() => void this.flush(), 0);
    });
  }

  private async flush() {
    const batch = this.pending;
    this.pending = [];
    this.timer = null;
    const slots = [...new Set(batch.map((p) => p.slot))];
    let results: Map<string, Decision> = new Map();
    try {
      const ds = await decideBatch(this.config, this.visitor.subject, await this.ctx, slots);
      results = new Map(ds.filter(isDecision).map((d) => [d.slot, d]));
    } catch (e) {
      // Core down or slow: every slot renders its baseline. The page never waits on us.
      console.warn(`[pratibimba] rendering baselines: ${(e as Error).message}`);
    }
    for (const p of batch) p.resolve(results.get(p.slot) ?? null);
  }
}

async function defaultHeaders() {
  const { headers } = await import('next/headers.js');
  return headers();
}

export function createSlot(config: SlotConfig) {
  const scope = config.requestScope ?? cache(() => ({}));
  const batches = new WeakMap<object, Batch>();
  const readManifest = manifestReader(config);

  /** The visit's context plus its declared features, for the one batch of this request. */
  async function decideCtx(v: Visitor): Promise<Ctx> {
    if (!config.features) return v.ctx;
    const h = await (config.headers ?? defaultHeaders)();
    const features = await featuresFor(readManifest, () => config.features!(h), v.ctx.consent);
    return features ? { ...v.ctx, features } : v.ctx;
  }

  async function visitor(): Promise<Visitor | null> {
    const h = await (config.headers ?? defaultHeaders)();
    const subject = h.get(SUBJECT_HEADER);
    const ctx = decodeCtx(h.get(CTX_HEADER));
    const tz = h.get(TZ_HEADER);
    if (ctx && validTimeZone(tz)) ctx.tz = tz;
    const consent = h.get(CONSENT_HEADER);
    if (ctx && (consent === '1' || consent === '0')) ctx.consent = consent === '1';
    return subject && ctx ? { subject, ctx } : null;
  }

  /** The decision for one slot in this request, or `null` (render the baseline). */
  async function decide(slot: string): Promise<Decision | null> {
    const v = await visitor();
    if (!v) {
      console.warn('[pratibimba] no visitor on this request: is proxy.ts installed and matching this path?');
      return null;
    }
    const key = scope();
    let b = batches.get(key);
    if (!b) {
      b = new Batch(config, v, decideCtx(v));
      batches.set(key, b);
    }
    return b.decide(slot);
  }

  async function Slot({ id, variants, children }: SlotProps) {
    const d = await decide(id);
    if (!d) return createElement('div', { 'data-prb-slot': id, style: { display: 'contents' } }, children);
    const v = await visitor();
    const own = variants?.[d.variant];
    const content =
      own !== undefined ? own : d.payload !== null ? createElement('div', { dangerouslySetInnerHTML: { __html: d.payload } }) : children;
    return createElement(
      'div',
      { 'data-prb-slot': id, 'data-prb-variant': d.variant, 'data-prb-decision': d.decision_id, style: { display: 'contents' } },
      content,
      createElement(PrbBeacon, {
        site: config.site,
        slot: id,
        decision: d.decision_id,
        subject: v!.subject,
        consent: v!.ctx.consent === true,
        eventsPath: config.eventsPath ?? '/_prb/e',
      }),
    );
  }

  /** A sorted slot (F2b): its keyed children in the order Core decided for this visitor. */
  async function Sorted({ id, children }: SortedProps) {
    const d = await decide(id);
    if (!d) return createElement('div', { 'data-prb-slot': id, style: { display: 'contents' } }, ...arrange(children, undefined).nodes);
    const v = await visitor();
    const { nodes, shown } = arrange(children, d.order);
    return createElement(
      'div',
      { 'data-prb-slot': id, 'data-prb-variant': d.variant, 'data-prb-decision': d.decision_id, style: { display: 'contents' } },
      ...nodes,
      createElement(PrbBeacon, {
        site: config.site,
        slot: id,
        decision: d.decision_id,
        subject: v!.subject,
        consent: v!.ctx.consent === true,
        eventsPath: config.eventsPath ?? '/_prb/e',
        items: shown,
      }),
    );
  }

  /** A copy slot (F4): `children` gets the decided text, field by field, as plain strings. */
  async function Copy<F extends string>({ id, fallback, children }: CopyProps<F>) {
    const d = await decide(id);
    if (!d) return createElement('div', { 'data-prb-slot': id, style: { display: 'contents' } }, children({ ...fallback }));
    const v = await visitor();
    return createElement(
      'div',
      { 'data-prb-slot': id, 'data-prb-variant': d.variant, 'data-prb-decision': d.decision_id, style: { display: 'contents' } },
      children(copyText(fallback, d.copy)),
      createElement(PrbBeacon, {
        site: config.site,
        slot: id,
        decision: d.decision_id,
        subject: v!.subject,
        consent: v!.ctx.consent === true,
        eventsPath: config.eventsPath ?? '/_prb/e',
      }),
    );
  }

  return { Slot, Sorted, Copy, decide, visitor };
}
