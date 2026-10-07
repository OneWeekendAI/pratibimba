// The proxy (Next.js 16 renamed middleware to proxy.ts). It runs before the render and does
// three things: makes sure the visitor has a subject id, fixes the visit's context, and passes
// both to the render as request headers. It never calls Core: the slots on the page decide.

import { NextResponse, type NextRequest } from 'next/server.js';
import {
  CTX_HEADER,
  SDK_HEADER,
  SESSION_COOKIE,
  SUBJECT_COOKIE,
  SUBJECT_HEADER,
  VERSION,
  contextFor,
  decodeCtx,
  encodeCtx,
  validSubject,
} from './context.ts';
import { GEO_TZ_HEADERS, TZ_COOKIE, TZ_HEADER, visitorTimeZone } from './timezone.ts';
import { CONSENT_HEADER, visitorConsent, type ConsentOption } from './consent.ts';

export interface ProxyOptions {
  /**
   * Keep the subject in a one-year first-party cookie, which makes `returning` meaningful.
   * Collect consent first. Default: a session cookie, so `returning` is always false.
   */
  persist?: boolean;
  /**
   * The edge geolocation headers trusted to name the visitor's time zone when the browser's
   * `_prb_tz` cookie names none, first match wins.
   * Default: Vercel's, Cloudflare's and CloudFront's. Pass `[]` when the app does not run
   * behind one of them, so a client-sent copy is ignored and only the browser's cookie counts.
   */
  timeZoneHeaders?: readonly string[];
  /**
   * The visitor's consent, sent to Core as `ctx.consent`. A boolean, or a function of the
   * request's cookies; `undefined` from it falls through to the cookies consent.ts reads
   * (`_prb_consent`, TCF's `euconsent-v2`, OneTrust's `OptanonConsent`). Default: those cookies.
   */
  consent?: ConsentOption;
}

const YEAR = 60 * 60 * 24 * 365;

export function createProxy(opts: ProxyOptions = {}) {
  return function proxy(request: NextRequest): NextResponse {
    const existing = request.cookies.get(SUBJECT_COOKIE)?.value;
    const subject = validSubject(existing) ? existing : crypto.randomUUID();
    const session = decodeCtx(request.cookies.get(SESSION_COOKIE)?.value);
    const ctx = session ?? contextFor(request.nextUrl, request.headers.get('user-agent') ?? '', opts.persist === true && validSubject(existing));

    const headers = new Headers(request.headers);
    headers.set(SUBJECT_HEADER, subject);
    headers.set(CTX_HEADER, encodeCtx(ctx));
    // The local clock is not part of the visit's fixed context: it is read on every request.
    const tz = visitorTimeZone(request.headers, request.cookies.get(TZ_COOKIE)?.value, opts.timeZoneHeaders ?? GEO_TZ_HEADERS);
    if (tz) headers.set(TZ_HEADER, tz);
    else headers.delete(TZ_HEADER);
    // Read on every request too: a choice made on one page counts from the next.
    const jar = { get: (n: string) => request.cookies.get(n)?.value };
    headers.set(CONSENT_HEADER, visitorConsent(jar, opts.consent) ? '1' : '0');
    const res = NextResponse.next({ request: { headers } });

    const secure = request.nextUrl.protocol === 'https:';
    res.cookies.set(SUBJECT_COOKIE, subject, {
      path: '/',
      httpOnly: true,
      sameSite: 'lax',
      secure,
      ...(opts.persist ? { maxAge: YEAR } : {}),
    });
    if (!session) {
      res.cookies.set(SESSION_COOKIE, encodeCtx(ctx), { path: '/', httpOnly: true, sameSite: 'lax', secure });
    }
    // The install wizard's probe looks for this.
    res.headers.set(SDK_HEADER, `next/${VERSION}`);
    return res;
  };
}
