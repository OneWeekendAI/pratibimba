// The two calls this package makes to Pratibimba Core, typed.

export interface Ctx {
  returning: boolean;
  mobile: boolean;
  from_ad: boolean;
  /** The visitor's IANA time zone, when the proxy could name it (see timezone.ts). */
  tz?: string;
  /** Whether the visitor consented to being remembered (see consent.ts). */
  consent?: boolean;
  /** Declared features, name -> on (see features.ts). */
  features?: Record<string, boolean>;
}

export interface Decision {
  decision_id: string;
  slot: string;
  variant: string;
  /** HTML to render; `null` means the page's own content (the baseline). */
  payload: string | null;
  explanation: {
    policy: string;
    policy_version: number;
    features: Record<string, number>;
    scores: Record<string, number>;
    seed: string;
    in_holdout: boolean;
  };
}

export interface SlotError {
  slot: string;
  error: string;
}

export type Fetch = typeof fetch;

/** How long a server render waits for Core before it renders the baseline. */
export const DEFAULT_TIMEOUT_MS = 300;

/**
 * How long a request that missed its render may keep running. A late answer is read to the end
 * so its keep-alive connection goes back to the pool; only a Core that never answers is cut off.
 */
export const HARD_TIMEOUT_MS = 10_000;

export function isDecision(d: Decision | SlotError): d is Decision {
  return typeof (d as Decision).decision_id === 'string';
}

/**
 * Waits up to `ms` for `work`, then rejects; `work` itself keeps running.
 *
 * Why not abort at the deadline: aborting an in-flight fetch destroys its keep-alive socket, so
 * the next render pays a fresh TCP+TLS handshake, misses its deadline too, and the pool never
 * warms. Measured against the live endpoint from India: 15 renders at 300 ms opened 22
 * connections and served 0 decisions. The work is bounded by its own (hard) signal instead.
 */
export function deadline<T>(work: Promise<T>, ms: number, what: string): Promise<T> {
  work.catch(() => {}); // a late failure is nobody's: the render already moved on
  let timer: ReturnType<typeof setTimeout> | undefined;
  const late = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${what} timed out after ${ms} ms`)), ms);
  });
  return Promise.race([work, late]).finally(() => clearTimeout(timer));
}

export async function decideBatch(
  opts: { core: string; site: string; fetch?: Fetch; timeoutMs?: number; hardTimeoutMs?: number },
  subject: string,
  ctx: Ctx,
  slots: string[],
): Promise<(Decision | SlotError)[]> {
  const f = opts.fetch ?? fetch;
  const ms = opts.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const work = (async () => {
    const res = await f(`${opts.core.replace(/\/$/, '')}/decide/batch`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ site: opts.site, subject, slots, ctx }),
      signal: AbortSignal.timeout(Math.max(ms, opts.hardTimeoutMs ?? HARD_TIMEOUT_MS)),
      cache: 'no-store',
    });
    if (!res.ok) throw new Error(`Pratibimba decide failed: ${res.status}`);
    const body = (await res.json()) as { decisions: (Decision | SlotError)[] };
    return body.decisions;
  })();
  return deadline(work, ms, 'Pratibimba decide');
}

export interface EventIn {
  id: string;
  name: string;
  slot?: string;
  decision_id?: string;
}

export async function postEvents(
  opts: { core: string; site: string; fetch?: Fetch },
  subject: string,
  events: EventIn[],
  consent?: boolean,
): Promise<void> {
  const f = opts.fetch ?? fetch;
  const res = await f(`${opts.core.replace(/\/$/, '')}/e`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ site: opts.site, subject, events, consent: consent === true }),
    cache: 'no-store',
  });
  if (!res.ok) throw new Error(`Pratibimba events failed: ${res.status}`);
}
