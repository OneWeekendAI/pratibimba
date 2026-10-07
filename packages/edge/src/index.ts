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

export { rewriter, beforeBodyEnd, BEACON_JS };
export { TZ_COOKIE, GEO_TZ_HEADERS, validTimeZone, visitorTimeZone, CONSENT_COOKIE, visitorConsent };
export type { ConsentOption };
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
  fetch?: typeof fetch;
  /**
   * Geolocation headers trusted to name the visitor's time zone, first match wins. Default:
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
}

interface Ctx {
  returning: boolean;
  mobile: boolean;
  from_ad: boolean;
  /** The visitor's IANA zone; Core derives the local hour, night, weekend and business hours. */
  tz?: string;
  /** Whether the visitor consented to being remembered (see consent.ts). */
  consent?: boolean;
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

function attr(json: unknown) {
  return JSON.stringify(json).replace(/&/g, '&amp;').replace(/'/g, '&#39;').replace(/</g, '&lt;');
}

export function createEdge(opts: EdgeOptions) {
  const f = opts.fetch ?? fetch;
  const core = opts.core.replace(/\/$/, '');

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
    const tz = typeof cf === 'string' && validTimeZone(cf) ? cf : visitorTimeZone(request.headers, jar.get(TZ_COOKIE), opts.timeZoneHeaders ?? GEO_TZ_HEADERS);
    const consent = visitorConsent(jar, opts.consent);
    const decideCtx: Ctx = { ...ctx, ...(tz ? { tz } : {}), consent };

    const decided: { slot: string; id: string }[] = [];
    async function decide(slot: string): Promise<Replacement> {
      const r = await f(`${core}/decide`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ site: opts.site, slot, subject, ctx: decideCtx }),
        signal: AbortSignal.timeout(opts.timeoutMs ?? 300),
      });
      if (!r.ok) return { html: null };
      const d = (await r.json()) as { decision_id: string; payload: string | null };
      decided.push({ slot, id: d.decision_id });
      return { html: d.payload };
    }

    // The beacon tag goes in last, once every slot above it has been decided.
    const beacon = () => `<script src="/_prb/beacon.js" data-prb='${attr({ site: opts.site, subject, consent, decisions: decided })}'></script>`;
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

  return { handleRequest };
}
