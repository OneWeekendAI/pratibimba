// @pratibimba/edge: put this in front of any origin. HTML responses have their marked slots
// rewritten as they stream, so the first byte the browser sees already carries the variant.
//
//   const prb = createEdge({ core: 'https://core.internal', site: 'shop' });
//   export default { fetch: (req) => prb.handleRequest(req) };          // Cloudflare
//   Deno.serve((req) => prb.handleRequest(req));                         // Deno Deploy
//   export const GET = (req) => prb.handleRequest(req);                  // Vercel Edge

import { BEACON_JS } from './beacon.ts';
import { beforeBodyEnd, rewriter, type Replacement } from './markers.ts';
import { GEO_TZ_HEADERS, TZ_COOKIE, validTimeZone, visitorTimeZone } from './timezone.ts';
import { CONSENT_COOKIE, visitorConsent, type ConsentOption } from './consent.ts';
import { MANIFEST_TTL_MS, featuresFor, manifestReader, mayRemember, type FeatureValues } from './features.ts';
import { addToCartEvent, purchaseEvent, type Item, type Order } from './commerce.ts';

export { rewriter, beforeBodyEnd, BEACON_JS };
export { TZ_COOKIE, GEO_TZ_HEADERS, validTimeZone, visitorTimeZone, CONSENT_COOKIE, visitorConsent };
export { MANIFEST_TTL_MS };
export type { ConsentOption, FeatureValues, Item, Order };
export const VERSION = '0.1.0';
export const SUBJECT_COOKIE = '_prb_subject';
export const SESSION_COOKIE = '_prb_s';

export interface EdgeOptions {
  /** Pratibimba Core's base URL. The browser never sees it. */
  core: string;
  site: string;
  /** Fetches the page from the origin. Default: `fetch(request)`. */
  origin?: (request: Request) => Promise<Response>;
  /** One-year subject cookie (collect consent first). Default: a session cookie. */
  persist?: boolean;
  /** How long one slot waits for Core before keeping its baseline. Default 300 ms. */
  timeoutMs?: number;
  /**
   * How long a decide that missed its page may keep running, so its late answer is read and its
   * keep-alive connection goes back to the pool. Only a Core that never answers is cut off.
   * Default 10 s.
   */
  hardTimeoutMs?: number;
  fetch?: typeof fetch;
  /**
   * Geolocation headers trusted to name the visitor's time zone when the browser's `_prb_tz`
   * cookie names none, first match wins. Default:
   * Vercel's, Cloudflare's and CloudFront's. Pass `[]` off those platforms, so a client-sent
   * copy is ignored. Cloudflare's `request.cf.timezone` is always read: a client cannot set it.
   */
  timeZoneHeaders?: readonly string[];
  /**
   * The visitor's consent, sent to Core as `ctx.consent`. A boolean, or a function of the
   * request's cookies; `undefined` from it falls through to the cookies consent.ts reads
   * (`_prb_consent`, TCF's `euconsent-v2`, OneTrust's `OptanonConsent`). Default: those cookies.
   */
  consent?: ConsentOption;
  /**
   * The visitor's declared features for this request, name -> on (the server twin of the
   * browser's `window.__prb_features`), read from the request: your session cookie, say. Sent
   * as `ctx.features`: only names the site file declares (from Core's manifest, cached for a
   * minute), only booleans, and a pii one only when the visitor is identified (`full`, or
   * `consent_aware` with consent).
   */
  features?: (request: Request) => FeatureValues | undefined | Promise<FeatureValues | undefined>;
}

interface Ctx {
  returning: boolean;
  mobile: boolean;
  from_ad: boolean;
  /** The visitor's IANA zone; Core derives the local hour, night, weekend and business hours. */
  tz?: string;
  /** Whether the visitor consented to being remembered (see consent.ts). */
  consent?: boolean;
  /** Declared features, name -> on (see features.ts). */
  features?: Record<string, boolean>;
}

const MOBILE = /Mobi|Android|iPhone|iPad|iPod/i;
const AD_PARAMS = ['utm_source', 'utm_medium', 'utm_campaign', 'gclid', 'fbclid', 'msclkid'];
const PUBLIC: { method: string; pattern: RegExp }[] = [
  { method: 'GET', pattern: /^manifest\/[a-z0-9_-]{1,64}(\.js)?$/ },
  { method: 'GET', pattern: /^sdk\.js$/ },
  { method: 'POST', pattern: /^e$/ },
  { method: 'POST', pattern: /^decide$/ },
  { method: 'POST', pattern: /^decide\/batch$/ },
];

function cookies(request: Request): Map<string, string> {
  const out = new Map<string, string>();
  for (const part of (request.headers.get('cookie') ?? '').split(';')) {
    const i = part.indexOf('=');
    if (i > 0) out.set(part.slice(0, i).trim(), part.slice(i + 1).trim());
  }
  return out;
}

function encodeCtx(c: Ctx) {
  return `r${+c.returning}m${+c.mobile}a${+c.from_ad}`;
}

function decodeCtx(s: string | undefined): Ctx | null {
  const m = /^r([01])m([01])a([01])$/.exec(s ?? '');
  return m ? { returning: m[1] === '1', mobile: m[2] === '1', from_ad: m[3] === '1' } : null;
}

const validSubject = (s: string | undefined): s is string => !!s && s.length <= 256 && /^[\x21-\x7e]+$/.test(s);

/** Waits up to `ms` for `work`, then rejects; `work` itself keeps running to the end. */
function deadline<T>(work: Promise<T>, ms: number): Promise<T> {
  work.catch(() => {}); // a late failure is nobody's: the page already moved on
  let timer: ReturnType<typeof setTimeout> | undefined;
  const late = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`Pratibimba decide timed out after ${ms} ms`)), ms);
  });
  return Promise.race([work, late]).finally(() => clearTimeout(timer));
}

function attr(json: unknown) {
  return JSON.stringify(json).replace(/&/g, '&amp;').replace(/'/g, '&#39;').replace(/</g, '&lt;');
}

export function createEdge(opts: EdgeOptions) {
  const f = opts.fetch ?? fetch;
  const core = opts.core.replace(/\/$/, '');
  const readManifest = manifestReader(opts);

  async function forward(request: Request, path: string): Promise<Response> {
    if (path === 'beacon.js') {
      return new Response(BEACON_JS, { headers: { 'content-type': 'application/javascript; charset=utf-8', 'cache-control': 'public, max-age=300' } });
    }
    if (!PUBLIC.some((p) => p.method === request.method && p.pattern.test(path))) {
      return new Response('{"error":"not found"}', { status: 404, headers: { 'content-type': 'application/json' } });
    }
    const up = await f(`${core}/${path}${new URL(request.url).search}`, {
      method: request.method,
      headers: request.method === 'POST' ? { 'content-type': 'application/json' } : {},
      body: request.method === 'POST' ? await request.text() : undefined,
    });
    const headers = new Headers();
    for (const h of ['content-type', 'cache-control']) {
      const v = up.headers.get(h);
      if (v) headers.set(h, v);
    }
    return new Response(up.body, { status: up.status, headers });
  }

  async function handleRequest(request: Request): Promise<Response> {
    const url = new URL(request.url);
    if (url.pathname.startsWith('/_prb/')) return forward(request, url.pathname.slice('/_prb/'.length));

    const res = await (opts.origin ?? ((r: Request) => f(r)))(request);
    if (!res.body || !(res.headers.get('content-type') ?? '').includes('text/html')) return res;

    const jar = cookies(request);
    const existing = jar.get(SUBJECT_COOKIE);
    const subject = validSubject(existing) ? existing : crypto.randomUUID();
    const session = decodeCtx(jar.get(SESSION_COOKIE));
    const ctx: Ctx = session ?? {
      returning: opts.persist === true && validSubject(existing),
      mobile: MOBILE.test(request.headers.get('user-agent') ?? ''),
      from_ad: AD_PARAMS.some((p) => url.searchParams.has(p)),
    };
    // The local clock is read on every request, never fixed in the session cookie.
    const cf = (request as Request & { cf?: { timezone?: unknown } }).cf?.timezone;
    const tz = visitorTimeZone(request.headers, jar.get(TZ_COOKIE), opts.timeZoneHeaders ?? GEO_TZ_HEADERS, cf);
    const consent = visitorConsent(jar, opts.consent);
    const baseCtx: Ctx = { ...ctx, ...(tz ? { tz } : {}), consent };
    // Read once per page, and only when the first slot needs it: the origin never waits.
    let ctxP: Promise<Ctx> | null = null;
    const decideCtx = () =>
      (ctxP ??= opts.features
        ? featuresFor(readManifest, () => opts.features!(request), consent).then((features) => (features ? { ...baseCtx, features } : baseCtx))
        : Promise.resolve(baseCtx));

    // The beacon's cookie rule (G1) needs the privacy mode: read from the (cached) manifest once
    // per page, after the first decision lands (a page with no decision has no beacon, and a
    // read raced against a slow Core would be aborted, costing its keep-alive socket).
    let remember = false;
    let rememberP: Promise<void> | null = null;
    const readRemember = () => (rememberP ??= mayRemember(readManifest, consent).then((v) => void (remember = v)));

    // `items`: a sorted slot's keys as the rewriter placed them, for the exposure (F2b, G3).
    const decided: { slot: string; id: string; items?: string[] }[] = [];
    async function decide(slot: string): Promise<Replacement> {
      const ms = opts.timeoutMs ?? 300;
      // Raced, not aborted: an abort destroys the keep-alive socket, so every later page would
      // pay a fresh handshake and miss its deadline too. The late answer is read and dropped.
      const work = (async () => {
        const r = await f(`${core}/decide`, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ site: opts.site, slot, subject, ctx: await decideCtx() }),
          signal: AbortSignal.timeout(Math.max(ms, opts.hardTimeoutMs ?? 10_000)),
        });
        if (!r.ok) return null;
        return (await r.json()) as { decision_id: string; payload: string | null; order?: unknown };
      })();
      const d = await deadline(work, ms);
      if (!d) return { html: null };
      await readRemember();
      const entry: (typeof decided)[number] = { slot, id: d.decision_id };
      decided.push(entry);
      const order = Array.isArray(d.order) && d.order.every((k) => typeof k === 'string') ? (d.order as string[]) : undefined;
      if (!order) return { html: d.payload };
      return { html: null, order, rendered: (keys) => void (entry.items = keys) };
    }

    // The beacon tag goes in last, once every slot above it has been decided.
    const beacon = () => `<script src="/_prb/beacon.js" data-prb='${attr({ site: opts.site, subject, consent, remember, decisions: decided })}'></script>`;
    const body = res.body
      .pipeThrough(new TextDecoderStream())
      .pipeThrough(rewriter(decide))
      .pipeThrough(beforeBodyEnd(() => (decided.length ? beacon() : '')))
      .pipeThrough(new TextEncoderStream());

    const headers = new Headers(res.headers);
    headers.delete('content-length');
    headers.delete('etag');
    // One visitor's page: a shared cache must never keep it.
    headers.set('cache-control', 'private, no-store');
    headers.set('x-prb-sdk', `edge/${VERSION}`);
    const secure = url.protocol === 'https:' ? '; Secure' : '';
    headers.append('set-cookie', `${SUBJECT_COOKIE}=${subject}; Path=/; HttpOnly; SameSite=Lax${opts.persist ? '; Max-Age=31536000' : ''}${secure}`);
    if (!session) headers.append('set-cookie', `${SESSION_COOKIE}=${encodeCtx(ctx)}; Path=/; HttpOnly; SameSite=Lax${secure}`);
    return new Response(body, { status: res.status, statusText: res.statusText, headers });
  }

  /** Post events for the visitor the request's subject cookie names; false when it names none. */
  async function post(request: Request, events: object[]): Promise<boolean> {
    const jar = cookies(request);
    const subject = jar.get(SUBJECT_COOKIE);
    if (!validSubject(subject)) return false;
    const res = await f(`${core}/e`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ site: opts.site, subject, events, consent: visitorConsent(jar, opts.consent) === true }),
    });
    if (!res.ok) throw new Error(`Pratibimba events failed: ${res.status}`);
    return true;
  }

  return {
    handleRequest,
    /** Post a goal (e.g. `signup`) for this request's visitor from edge code. */
    track: (request: Request, name: string, o: { slot?: string; decisionId?: string } = {}) =>
      post(request, [{ id: crypto.randomUUID(), name, slot: o.slot, decision_id: o.decisionId }]),
    /** Post `add_to_cart` with the item for this request's visitor. Its price is not revenue. */
    addToCart: async (request: Request, item: Item) => post(request, [addToCartEvent(item)]),
    /**
     * Post `purchase` with the order for this request's visitor. Core stores one row per
     * `order_id` and shares the value among the decisions in each slot's window. Throws a
     * TypeError for a malformed order.
     */
    purchase: async (request: Request, order: Order) => post(request, [purchaseEvent(order)]),
  };
}
