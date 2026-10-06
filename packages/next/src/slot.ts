// `<Slot>`: a server component that decides during the server render, so the HTML that
// leaves the server already holds the chosen variant. No hide, no swap, no flicker.
//
// Every <Slot> in one render shares one `/decide/batch` call: slots ask for their decision,
// and the batch goes out once the render has visited them all (the next macrotask).

import { createElement, cache, type ReactNode } from 'react';
import { PrbBeacon } from './beacon.ts';
import { decideBatch, isDecision, type Ctx, type Decision, type Fetch } from './core.ts';
import { CTX_HEADER, SUBJECT_HEADER, decodeCtx } from './context.ts';

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
  headers?: () => Promise<Headers | { get(name: string): string | null }>;
  /** One object per request; slots that share it share a batch. Default: React's `cache`. */
  requestScope?: () => object;
}

export interface SlotProps {
  /** The slot key, as declared in the site file. */
  id: string;
  /** Components for variants that live in your repo, by variant key. */
  variants?: Record<string, ReactNode>;
  /** The baseline: what the page shows when no variant replaces it. */
  children?: ReactNode;
}

type Pending = { slot: string; resolve: (d: Decision | null) => void };

class Batch {
  private pending: Pending[] = [];
  private timer: ReturnType<typeof setTimeout> | null = null;
  private config: SlotConfig;
  private visitor: Visitor;
  constructor(config: SlotConfig, visitor: Visitor) {
    this.config = config;
    this.visitor = visitor;
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
      const ds = await decideBatch(this.config, this.visitor.subject, this.visitor.ctx, slots);
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

  async function visitor(): Promise<Visitor | null> {
    const h = await (config.headers ?? defaultHeaders)();
    const subject = h.get(SUBJECT_HEADER);
    const ctx = decodeCtx(h.get(CTX_HEADER));
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
      b = new Batch(config, v);
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
        eventsPath: config.eventsPath ?? '/_prb/e',
      }),
    );
  }

  return { Slot, decide, visitor };
}
