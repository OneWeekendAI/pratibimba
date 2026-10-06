// The two calls this package makes to Pratibimba Core, typed.

export interface Ctx {
  returning: boolean;
  mobile: boolean;
  from_ad: boolean;
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

export function isDecision(d: Decision | SlotError): d is Decision {
  return typeof (d as Decision).decision_id === 'string';
}

export async function decideBatch(
  opts: { core: string; site: string; fetch?: Fetch; timeoutMs?: number },
  subject: string,
  ctx: Ctx,
  slots: string[],
): Promise<(Decision | SlotError)[]> {
  const f = opts.fetch ?? fetch;
  const res = await f(`${opts.core.replace(/\/$/, '')}/decide/batch`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ site: opts.site, subject, slots, ctx }),
    signal: AbortSignal.timeout(opts.timeoutMs ?? DEFAULT_TIMEOUT_MS),
    cache: 'no-store',
  });
  if (!res.ok) throw new Error(`Pratibimba decide failed: ${res.status}`);
  const body = (await res.json()) as { decisions: (Decision | SlotError)[] };
  return body.decisions;
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
): Promise<void> {
  const f = opts.fetch ?? fetch;
  const res = await f(`${opts.core.replace(/\/$/, '')}/e`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ site: opts.site, subject, events }),
    cache: 'no-store',
  });
  if (!res.ok) throw new Error(`Pratibimba events failed: ${res.status}`);
}
