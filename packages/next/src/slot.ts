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
import { featuresFor, manifestReader, mayRemember, type FeatureValues } from './features.ts';

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
  /**
   * Where the browser fetches uploaded images (S11), without the site: default `/_prb/assets`,
   * which the route file forwards to Core. Point it at a CDN in front of Core if you have one.
   */
  assetsPath?: string;
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

/** An image a copy slot's image field shows (S11): a plain `<img>`'s attributes. */
export interface CopyImage {
  src: string;
  alt: string;
  width?: number;
  height?: number;
}

/** A copy field's value: a line of text, or an image. */
export type CopyValue = string | CopyImage;

export interface CopyProps<T extends Record<string, CopyValue> = Record<string, string>> {
  /** The copy slot's key, as declared in the site file (`kind: "copy"`). */
  id: string;
  /**
   * The page's own value for every field: the baseline, and the value of any field a variant
   * leaves out. A text field's is a string; an image field's is `{ src, alt, width?, height? }`.
   */
  fallback: T;
  /**
   * Renders the fields. Each text value is a plain string; render it as a React text child
   * (escaped), never as HTML. Each image is `{ src, alt, width?, height? }`; render it with
   * `<prb.Image image={t.banner} />` or your own `<img>`.
   */
  children: (text: T) => ReactNode;
}

const ASSET_ID = /^[0-9a-f]{64}$/;
const SAFE_URL = /^(https:\/\/[^/\s"'<>`\\]|\/(?!\/))[^\s"'<>`\\]*$/;

/**
 * A decided image value as `<img>` attributes, or `null` when it is not one Core would have
 * accepted (no alt, an address that is not https or root-relative, an asset id that is not a
 * SHA-256). `assets` is where the browser fetches uploads, e.g. `/_prb/assets/<site>`.
 */
export function imageOf(v: unknown, assets: string): CopyImage | null {
  if (!v || typeof v !== 'object') return null;
  const o = v as Record<string, unknown>;
  if (typeof o.alt !== 'string' || !o.alt) return null;
  let src: string;
  if (typeof o.asset === 'string' && ASSET_ID.test(o.asset)) src = `${assets.replace(/\/$/, '')}/${o.asset}`;
  else if (typeof o.url === 'string' && SAFE_URL.test(o.url)) src = o.url;
  else return null;
  const out: CopyImage = { src, alt: o.alt };
  for (const k of ['width', 'height'] as const) if (typeof o[k] === 'number' && (o[k] as number) > 0) out[k] = Math.floor(o[k] as number);
  return out;
}

/**
 * What a copy decision shows: the page's own value for each field, overridden by the decided
 * variant's value of the same kind (text for a text field, a safe image for an image field).
 * Anything else is ignored.
 */
export function copyText<T extends Record<string, CopyValue>>(fallback: T, copy: unknown, assets = '/_prb/assets'): T {
  const out = { ...fallback } as Record<string, CopyValue>;
  if (copy && typeof copy === 'object') {
    for (const [k, v] of Object.entries(copy as Record<string, unknown>)) {
      if (!(k in out)) continue;
      if (typeof out[k] === 'string') {
        if (typeof v === 'string') out[k] = v;
      } else {
        const img = imageOf(v, assets);
        if (img) out[k] = img;
      }
    }
  }
  return out as T;
}

/** `<prb.Image image={t.banner} className="hero" />`: an image field as a plain `<img>`. */
export function PrbImage({ image, ...rest }: { image: CopyImage } & Record<string, unknown>) {
  const { src, alt, width, height } = image;
  return createElement('img', { src, alt, ...(width ? { width } : {}), ...(height ? { height } : {}), ...rest });
}

/**
 * The children in the decided order, each keyed child wrapped in a `display: contents` element
 * marked `data-prb-item`. Keyed children fill the keyed children's places in the decided order;
 * one the order leaves out (gated for this visitor) follows them in page order. A child without a
 * key keeps its place, as in the edge rewriter and the browser SDK. Returns the keys of the
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
  let n = 0;
  const nodes = all.map((c) => (keyOf(c) === null ? c : wrap(placed[n++])));
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
        remember: await mayRemember(readManifest, v!.ctx.consent),
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
        remember: await mayRemember(readManifest, v!.ctx.consent),
        eventsPath: config.eventsPath ?? '/_prb/e',
        items: shown,
      }),
    );
  }

  const assets = `${(config.assetsPath ?? '/_prb/assets').replace(/\/$/, '')}/${config.site}`;

  /** A copy slot (F4): `children` gets the decided fields: text as plain strings, images as `<img>` attributes. */
  async function Copy<T extends Record<string, CopyValue>>({ id, fallback, children }: CopyProps<T>) {
    const d = await decide(id);
    if (!d) return createElement('div', { 'data-prb-slot': id, style: { display: 'contents' } }, children({ ...fallback }));
    const v = await visitor();
    return createElement(
      'div',
      { 'data-prb-slot': id, 'data-prb-variant': d.variant, 'data-prb-decision': d.decision_id, style: { display: 'contents' } },
      children(copyText(fallback, d.copy, assets)),
      createElement(PrbBeacon, {
        site: config.site,
        slot: id,
        decision: d.decision_id,
        subject: v!.subject,
        consent: v!.ctx.consent === true,
        remember: await mayRemember(readManifest, v!.ctx.consent),
        eventsPath: config.eventsPath ?? '/_prb/e',
      }),
    );
  }

  return { Slot, Sorted, Copy, Image: PrbImage, decide, visitor };
}
