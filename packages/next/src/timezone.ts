// The visitor's time zone, for a server render that has no browser clock. Core turns an IANA
// zone name into the local-time features (hour, night, weekend, business hours) at its own
// clock, so the name is all a server-side SDK sends.
//
// Where the name comes from, in order (backend.md, "Local time as a context signal": the
// browser's own zone, when it sent one, is preferred over the IP's):
// 1. The first-party cookie `_prb_tz`, which the browser half (the beacon) sets from
//    `Intl.DateTimeFormat().resolvedOptions().timeZone`. It names the device's zone, which
//    is right where IP geolocation is not (mobile carriers, VPNs). The first page of a visit
//    has no cookie yet.
// 2. The edge's IP geolocation header. Vercel sets `x-vercel-ip-timezone`, Cloudflare sets
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

/** The visitor's zone: the browser's cookie, else the first valid geo header, else `undefined`. */
export function visitorTimeZone(headers: HeaderBag, cookie: string | undefined, names: readonly string[] = GEO_TZ_HEADERS): string | undefined {
  const c = decode(cookie);
  if (validTimeZone(c)) return c;
  for (const n of names) {
    const v = headers.get(n);
    if (validTimeZone(v)) return v;
  }
  return undefined;
}

/** Browser side: keep the zone in the first-party session cookie the server reads next time. */
export function rememberTimeZone(doc: Document = document): void {
  try {
    const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    if (!validTimeZone(zone)) return;
    const pair = `${TZ_COOKIE}=${encodeURIComponent(zone)}`;
    if (doc.cookie.split('; ').includes(pair)) return;
    const secure = doc.location?.protocol === 'https:' ? '; Secure' : '';
    doc.cookie = `${pair}; Path=/; SameSite=Lax${secure}`;
  } catch {
    // No cookies (sandboxed frame, privacy mode): the server falls back to the geo header.
  }
}
