// Whether the visitor consented to being remembered, for an edge rewrite. A copy of
// @pratibimba/next's consent.ts (the two packages share no runtime code). Core reads it only
// on a `consent_aware` site: with consent the subject is stored as is; without it, as a daily
// pseudonym. The browser SDK reads the same choice from the page; a server reads what reached
// it as cookies, in this order (first one that answers wins):
//
// 1. The `consent` option: a boolean, or a function of the request's cookies (your own CMP).
//    The server-side twin of the browser tag's `data-cookie="on"`.
// 2. `_prb_consent`, a first-party cookie your banner sets: `granted`/`1` or `denied`/`0`.
// 3. IAB TCF v2, the `euconsent-v2` cookie many CMPs keep: purpose 1 ("store and/or access
//    information on a device").
// 4. OneTrust's `OptanonConsent` cookie: its performance group, C0002.
//
// Google Consent Mode lives in the page's dataLayer and never reaches the server; a site that
// relies on it alone sets `_prb_consent` from its consent callback, or passes `consent`.
// No answer means no consent.

export const CONSENT_COOKIE = '_prb_consent';

export type CookieJar = { get(name: string): string | undefined };
export type ConsentOption = boolean | ((cookies: CookieJar) => boolean | undefined);

function decode(v: string): string {
  try {
    return decodeURIComponent(v);
  } catch {
    return v;
  }
}

/** Purpose 1 from a TCF v2 TC string's core segment, or `undefined` when it cannot be read. */
export function tcfPurposeOne(tc: string | undefined): boolean | undefined {
  const core = tc?.split('.')[0];
  if (!core || !/^[A-Za-z0-9_-]+$/.test(core)) return undefined;
  const ALPHA = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
  let bits = '';
  for (const ch of core) bits += ALPHA.indexOf(ch).toString(2).padStart(6, '0');
  // Version (6 bits) must be 2. PurposesConsent starts at bit 152: version 6, created 36,
  // last updated 36, CMP id 12, CMP version 12, screen 6, language 12, vendor list version 12,
  // policy version 6, service specific 1, non-standard texts 1, special feature opt-ins 12.
  if (bits.length < 176 || parseInt(bits.slice(0, 6), 2) !== 2) return undefined;
  return bits[152] === '1';
}

/** OneTrust's performance group from its `OptanonConsent` cookie (`...&groups=C0001:1,C0002:0&...`). */
export function oneTrustPerformance(cookie: string | undefined): boolean | undefined {
  if (!cookie) return undefined;
  const groups = new URLSearchParams(decode(cookie)).get('groups');
  if (groups === null) return undefined;
  const g = groups.split(',').find((p) => p.split(':')[0] === 'C0002');
  return g ? g.split(':')[1] === '1' : false;
}

/** The visitor's consent, from the option and the cookies above. */
export function visitorConsent(cookies: CookieJar, option?: ConsentOption): boolean {
  const own = typeof option === 'function' ? option(cookies) : option;
  if (typeof own === 'boolean') return own;
  const mine = cookies.get(CONSENT_COOKIE);
  if (mine !== undefined) {
    const v = decode(mine);
    if (v === 'granted' || v === '1') return true;
    if (v === 'denied' || v === '0') return false;
  }
  const tcf = tcfPurposeOne(cookies.get('euconsent-v2'));
  if (tcf !== undefined) return tcf;
  return oneTrustPerformance(cookies.get('OptanonConsent')) ?? false;
}
