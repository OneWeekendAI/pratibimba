// The dashboard's only way to Core: same-origin /api, with the HttpOnly session cookie.

export class Unauthorized extends Error {}

export interface ApiErrorBody {
  error: string;
  checks?: Check[];
}

export class ApiError extends Error {
  status: number;
  body: ApiErrorBody;
  constructor(status: number, body: ApiErrorBody) {
    super(body.error);
    this.status = status;
    this.body = body;
  }
}

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`/api${path}`, {
    method,
    credentials: 'same-origin',
    headers: body ? { 'content-type': 'application/json' } : {},
    body: body ? JSON.stringify(body) : undefined,
  });
  if (res.status === 401 || res.status === 503) {
    if (typeof window !== 'undefined' && !location.pathname.startsWith('/app/login')) {
      location.assign(`/app/login/?next=${encodeURIComponent(location.pathname + location.search)}`);
    }
    throw new Unauthorized(res.status === 503 ? 'admin API is off' : 'sign in');
  }
  if (res.status === 204) return undefined as T;
  const json = await res.json().catch(() => ({ error: res.statusText }));
  if (!res.ok) throw new ApiError(res.status, json);
  return json as T;
}

export const api = {
  get: <T>(p: string) => call<T>('GET', p),
  post: <T>(p: string, b: unknown) => call<T>('POST', p, b),
  login: (token: string) => call<void>('POST', '/session', { token }),
  logout: () => call<void>('DELETE', '/session'),
};

export interface Interval {
  estimate: number;
  lo: number;
  hi: number;
}
export interface Counts {
  n: number;
  conversions: number;
}
export interface Check {
  name: string;
  status: 'pass' | 'warn' | 'fail' | 'not_applicable';
  reason: string;
}
export interface SiteRow {
  id: string;
  origin: string;
  holdout_fraction: number;
  slots: number;
  pending: number;
}
export interface Overview {
  site: string;
  origin: string;
  holdout_fraction: number;
  now: string;
  decisions_last_minute: number;
  conversions_last_hour: number;
  lift_today: Interval | null;
  treated_today: Counts;
  holdout_today: Counts;
  sparkline: number[];
  stale: string | null;
  pending_approvals: number;
  slots: {
    key: string;
    metric: string;
    champion: string | null;
    lift_7d: Interval | null;
    treated: Counts;
    holdout: Counts;
    variants: number;
    traffic: { key: string; share: number; index: number }[];
  }[];
}
export interface VariantRow {
  id: number;
  key: string;
  status: 'draft' | 'approved' | 'paused' | 'rejected' | 'retired';
  is_baseline: boolean;
  traffic_share: number | null;
  counts: Counts;
  share: number;
  lift: Interval | null;
  payload: string | null;
  author: string;
  approved_by: string | null;
  approved_at: string | null;
  paused_at: string | null;
  pause_reason: null | {
    at: string;
    variant: { n: number; conversions: number; rate: number | null };
    holdout: { n: number; conversions: number; rate: number | null };
    lift: Interval;
    restored_to_version: number | null;
  };
}
export interface CurvePoint {
  t: string;
  policy: Interval | null;
  variants: Record<string, Interval | null>;
}
export interface SlotDetail {
  site: string;
  slot: string;
  id: number;
  selector: string;
  metric: string;
  attribution_window_secs: number;
  decision_ttl_secs: number;
  model_version: number;
  treated: Counts;
  holdout: Counts;
  lift: Interval | null;
  champion: string | null;
  variants: VariantRow[];
  curve: CurvePoint[];
  audiences: unknown[];
}
export interface DecisionRow {
  id: string;
  created_at: string;
  slot: string;
  variant: string;
  subject: string;
  in_holdout: boolean;
  exposed: boolean;
  outcome: number | null;
  settled: boolean;
  features: Record<string, number>;
}
export interface DecisionDetail {
  id: string;
  site: string;
  slot: string;
  metric: string;
  subject: string;
  variant: string;
  payload: string | null;
  is_baseline: boolean;
  created_at: string;
  exposed_at: string | null;
  outcome: number | null;
  outcome_at: string | null;
  settled_at: string | null;
  window_closes_at: string;
  in_holdout: boolean;
  policy: string;
  policy_version: number;
  seed: string;
  features: Record<string, number>;
  scores: { variant: string; variant_id: number; score: number }[];
  replay_matches: boolean | null;
  counterfactual: null | {
    kind: 'baseline' | 'policy';
    variant: string;
    payload: string | null;
    is_baseline: boolean;
    inferred_rate: number | null;
    shown_inferred_rate: number | null;
  };
  timeline: { at: string; name: string }[];
  review: { action: 'approve' | 'reject' | 'comment'; actor: string; comment: string | null; at: string }[];
  sentence: string;
}
export interface SlotResult {
  id: number;
  key: string;
  metric: string;
  treated: Counts;
  holdout: Counts;
  lift: Interval | null;
  variants: VariantRow[];
}
export interface Results {
  results: {
    site: string;
    alpha: number;
    holdout_fraction: number;
    treated: Counts;
    holdout: Counts;
    lift: Interval | null;
    slots: SlotResult[];
  };
  curves: Record<string, CurvePoint[]>;
  decisions_total: number;
  learnings: { id: number; kind: string; slot: string | null; variant: string | null; sentence: string; at: string }[];
}
export interface ApprovalCard {
  id: number;
  slot: string;
  key: string;
  html: string | null;
  author: string;
  created_at: string;
  champion: { key: string; payload: string | null; is_baseline: boolean } | null;
  checks: Check[];
  comments: { action: string; actor: string; comment: string | null; at: string }[];
}
export interface Approvals {
  drafts: ApprovalCard[];
  max_share: number;
  default_share: number;
  recent: { action: string; actor: string; slot: string; variant: string; comment: string | null; traffic_share: number | null; at: string }[];
}
export interface ProbeStep {
  id: 'framework' | 'package' | 'reach' | 'slot' | 'agent' | 'live';
  state: 'choice' | 'done' | 'pending' | 'failed' | 'later';
  detail: string;
}
