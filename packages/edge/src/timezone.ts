// The visitor's time zone, for an edge rewrite that has no browser clock. A copy of
// @pratibimba/next's timezone.ts (the two packages share no runtime code), plus Cloudflare's
// `request.cf.timezone`, which a Worker reads with no transform rule. Core turns an IANA
// zone name into the local-time features (hour, night, weekend, business hours) at its own
// clock, so the name is all a server-side SDK sends.
//
// Where the name comes from, in order:
// 1. The edge's IP geolocation header. Vercel sets `x-vercel-ip-timezone`, Cloudflare sets
//    `cf-timezone` (with the "visitor location headers" managed transform), CloudFront sets
//    `cloudfront-viewer-time-zone`. Each platform strips a client-sent copy; off those
//    platforms a geo header is just something the visitor sent, so `timeZoneHeaders: []`
//    switches them off. A forged zone only mislabels the forger's own hour.
// 2. A first-party session cookie, `_prb_tz`, that the browser half (beacon.js) sets from
//    `Intl.DateTimeFormat().resolvedOptions().timeZone`. The first page of a visit off a geo
//    edge has no zone; later pages do.

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

/** The visitor's zone: the first valid geo header, else the cookie, else `undefined`. */
export function visitorTimeZone(headers: HeaderBag, cookie: string | undefined, names: readonly string[] = GEO_TZ_HEADERS): string | undefined {
  for (const n of names) {
    const v = headers.get(n);
    if (validTimeZone(v)) return v;
  }
  const c = decode(cookie);
  return validTimeZone(c) ? c : undefined;
}
