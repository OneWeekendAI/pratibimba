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

export interface ProxyOptions {
  /**
   * Keep the subject in a one-year first-party cookie, which makes `returning` meaningful.
   * Collect consent first. Default: a session cookie, so `returning` is always false.
   */
  persist?: boolean;
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
