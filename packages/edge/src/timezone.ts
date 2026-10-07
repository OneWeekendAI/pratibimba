// The visitor's time zone, for an edge rewrite that has no browser clock. A copy of
// @pratibimba/next's timezone.ts (the two packages share no runtime code), plus Cloudflare's
// `request.cf.timezone`, which a Worker reads with no transform rule. Core turns an IANA
// zone name into the local-time features (hour, night, weekend, business hours) at its own
// clock, so the name is all a server-side SDK sends.
//
// Where the name comes from, in order (backend.md, "Local time as a context signal": the
// browser's own zone, when it sent one, is preferred over the IP's):
// 1. The first-party cookie `_prb_tz`, which the browser half (beacon.js) sets from
//    `Intl.DateTimeFormat().resolvedOptions().timeZone`. It names the device's zone, which
//    is right where IP geolocation is not (mobile carriers, VPNs). The first page of a visit
//    has no cookie yet.
// 2. Cloudflare's `request.cf.timezone` (index.ts), which a client cannot set.
// 3. The edge's IP geolocation header. Vercel sets `x-vercel-ip-timezone`, Cloudflare sets
//    `cf-timezone` (with the "visitor location headers" managed transform), CloudFront sets
//    `cloudfront-viewer-time-zone`. Each platform strips a client-sent copy; off those
//    platforms a geo header is just something the visitor sent, so `timeZoneHeaders: []`
//    switches them off. A forged zone (header or cookie) only mislabels the forger's own hour.

export const TZ_COOKIE = '_prb_tz';
export const TZ_HEADER = 'x-prb-tz';
export const GEO_TZ_HEADERS = ['x-vercel-ip-timezone', 'cf-timezone', 'cloudfront-viewer-time-zone'];

type HeaderBag = { get(name: string): string | null };

/** An IANA zone name the runtime's tz database knows. */
export function validTimeZone(s: string | null | undefined): s is string {
  if (!s || s.length > 64 || !/^[A-Za-z][A-Za-z0-9_+\-/]*$/.test(s)) return false;
  try {
    new Intl.DateTimeFormat('en-US', { timeZone: s });
    return true;
  } catch {
    return false;
  }
}

function decode(v: string | undefined): string | undefined {
  if (v === undefined) return undefined;
  try {
    return decodeURIComponent(v);
  } catch {
    return undefined;
  }
}

/**
 * The visitor's zone: the browser's cookie, else the platform's own zone (`request.cf`), else
 * the first valid geo header, else `undefined`.
 */
export function visitorTimeZone(headers: HeaderBag, cookie: string | undefined, names: readonly string[] = GEO_TZ_HEADERS, platform?: unknown): string | undefined {
  const c = decode(cookie);
  if (validTimeZone(c)) return c;
  if (typeof platform === 'string' && validTimeZone(platform)) return platform;
  for (const n of names) {
    const v = headers.get(n);
    if (validTimeZone(v)) return v;
  }
  return undefined;
}
