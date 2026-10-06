// How the proxy hands the visitor to the render: two request headers. The render never
// reads cookies itself, because a cookie the proxy just set is not on the request yet.

import type { Ctx } from './core.ts';

export const SUBJECT_COOKIE = '_prb_subject';
/** A session cookie holding the visit's context, so it does not change between pages. */
export const SESSION_COOKIE = '_prb_s';
export const SUBJECT_HEADER = 'x-prb-subject';
export const CTX_HEADER = 'x-prb-ctx';
export const SDK_HEADER = 'x-prb-sdk';
export const VERSION = '0.1.0';

/** `r1m0a0`: returning, mobile, from_ad. */
export function encodeCtx(c: Ctx): string {
  return `r${+c.returning}m${+c.mobile}a${+c.from_ad}`;
}

export function decodeCtx(s: string | null | undefined): Ctx | null {
  const m = /^r([01])m([01])a([01])$/.exec(s ?? '');
  if (!m) return null;
  return { returning: m[1] === '1', mobile: m[2] === '1', from_ad: m[3] === '1' };
}

const MOBILE = /Mobi|Android|iPhone|iPad|iPod/i;
const AD_PARAMS = ['utm_source', 'utm_medium', 'utm_campaign', 'gclid', 'fbclid', 'msclkid'];

/** The context at the start of a visit. `returning` needs a subject cookie that predates it. */
export function contextFor(url: URL, userAgent: string, hadSubject: boolean): Ctx {
  return {
    returning: hadSubject,
    mobile: MOBILE.test(userAgent),
    from_ad: AD_PARAMS.some((p) => url.searchParams.has(p)),
  };
}

/** A subject id: printable ASCII, at most 256 characters, as Core requires. */
export function validSubject(s: string | undefined): s is string {
  return !!s && s.length <= 256 && /^[\x21-\x7e]+$/.test(s);
}
