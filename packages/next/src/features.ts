// Declared features on the server path: the twin of the browser SDK's window.__prb_features.
// The owner's callback names the visitor's features; this file sends, as `ctx.features`, only
// the names the site file declares (an undeclared name makes Core refuse the whole decision),
// only boolean values, and a feature tagged pii only when the visitor is identified (`full`,
// or `consent_aware` with consent), so it never leaves the server otherwise.
//
// The declared names and the privacy mode come from Core's manifest, fetched once and kept for
// a minute (Core's own cache header). A manifest that cannot be read sends no features.

export interface DeclaredFeature {
  key: string;
  pii: boolean;
}

export interface SiteFeatures {
  privacy_mode: string;
  features: DeclaredFeature[];
}

/** What the owner's `features` callback returns: name -> on. Anything not boolean is dropped. */
export type FeatureValues = Record<string, unknown>;

/** How long one fetched manifest answers for this server. */
export const MANIFEST_TTL_MS = 60_000;

/** Core's "identified": `full`, or `consent_aware` with consent. */
export function identified(mode: string, consent: boolean | undefined): boolean {
  return mode === 'full' || (mode === 'consent_aware' && consent === true);
}

/** The features to send, or `undefined` when none may go. */
export function sendable(site: SiteFeatures, values: FeatureValues | undefined, consent: boolean | undefined): Record<string, boolean> | undefined {
  if (!values) return undefined;
  const out: Record<string, boolean> = {};
  let any = false;
  for (const f of site.features) {
    const v = values[f.key];
    if (typeof v !== 'boolean') continue;
    if (f.pii && !identified(site.privacy_mode, consent)) continue;
    out[f.key] = v;
    any = true;
  }
  return any ? out : undefined;
}

/** The site's declared features and privacy mode, from Core's manifest, cached. */
export function manifestReader(opts: { core: string; site: string; fetch?: typeof fetch; timeoutMs?: number; now?: () => number }) {
  let cached: { at: number; value: Promise<SiteFeatures | null> } | null = null;
  const now = opts.now ?? Date.now;
  async function load(): Promise<SiteFeatures | null> {
    try {
      const f = opts.fetch ?? fetch;
      const res = await f(`${opts.core.replace(/\/$/, '')}/manifest/${opts.site}`, {
        signal: AbortSignal.timeout(opts.timeoutMs ?? 300),
      });
      if (!res.ok) return null;
      const m = (await res.json()) as { privacy_mode?: unknown; features?: unknown };
      if (typeof m.privacy_mode !== 'string') return null;
      const features = Array.isArray(m.features)
        ? m.features.filter((x): x is DeclaredFeature => !!x && typeof x.key === 'string' && typeof x.pii === 'boolean')
        : [];
      return { privacy_mode: m.privacy_mode, features };
    } catch {
      return null;
    }
  }
  return function read(): Promise<SiteFeatures | null> {
    if (!cached || now() - cached.at >= MANIFEST_TTL_MS) {
      const value = load();
      cached = { at: now(), value };
      // A failed read is not kept: the next request tries again.
      void value.then((v) => {
        if (v === null && cached?.value === value) cached = null;
      });
    }
    return cached.value;
  };
}

/**
 * Whether the site may keep device storage for this visitor (G1): Core's "identified", the same
 * rule the browser SDK uses for its `_prb_subject` cookie. A manifest that cannot be read says no.
 */
export async function mayRemember(read: () => Promise<SiteFeatures | null>, consent: boolean | undefined): Promise<boolean> {
  const site = await read();
  return site !== null && identified(site.privacy_mode, consent);
}

/** `ctx.features` for one request: the callback's values filtered against the manifest. */
export async function featuresFor(
  read: () => Promise<SiteFeatures | null>,
  values: () => FeatureValues | undefined | Promise<FeatureValues | undefined>,
  consent: boolean | undefined,
): Promise<Record<string, boolean> | undefined> {
  let v: FeatureValues | undefined;
  try {
    v = await values();
  } catch (e) {
    console.warn(`[pratibimba] features callback failed; sending none: ${(e as Error).message}`);
    return undefined;
  }
  if (!v || Object.keys(v).length === 0) return undefined;
  const site = await read();
  return site ? sendable(site, v, consent) : undefined;
}
