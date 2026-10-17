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
  del: (p: string) => call<void>('DELETE', p),
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
  /** Conversions credited (a sum of shares under multi-touch; equal to conversions under last touch). */
  credit: number;
  /** Revenue credited, in the site's currency (F1). */
  revenue?: number;
  conversion_rate?: Interval | null;
  revenue_per_visitor?: Interval | null;
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
  /** The kill switch: set while every slot on the site serves only its baseline. */
  paused: { at: string; by: string } | null;
  /** One line per installed de-anonymiser plugin. */
  warnings: string[];
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
  /** The audience that gates this variant, or null for everyone. */
  audience?: number | null;
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
  audiences: AudienceRow[];
  /** A sorted slot's per-item results (G3); null for other kinds. */
  items: ItemRow[] | null;
}
/** One item of a sorted slot, over treated closed exposed decisions, counted as the cascade learns. */
export interface ItemRow {
  key: string;
  /** Rendered within the first `visible` positions. */
  shown: number;
  mean_position: number | null;
  /** Taught from: at or above the acted-on card, or the visible set when nothing was acted on. */
  seen: number;
  /** The topmost card acted on. */
  acted: number;
  /** acted / seen. */
  rate: number | null;
}
/** A promoted audience: a segment that can gate a variant. */
export interface AudienceRow {
  id: number;
  name: string;
  predicate: Record<string, unknown>;
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
  revenue_lift?: Interval | null;
  goal?: 'conversion' | 'revenue';
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
    currency?: string;
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
  /** Set when an agent wrote the draft over MCP: the tool call behind it. */
  agent: null | {
    name: string;
    justification: string | null;
    on_behalf_of: string | null;
    token: string;
    call_id: unknown;
    at: string;
    audit_id: number;
    tool: string;
  };
  champion: { key: string; payload: string | null; is_baseline: boolean; copy?: Record<string, string> | null } | null;
  /** A copy slot (F4): its declared fields, and this draft's text by field. */
  fields?: { name: string; max: number; required: boolean }[] | null;
  copy?: Record<string, string> | null;
  checks: Check[];
  comments: { action: string; actor: string; comment: string | null; at: string }[];
}
/** G2: a share change an agent proposed for a live variant; it waits for a person. */
export interface ShareRequest {
  id: number;
  slot: string;
  variant: string;
  variant_id: number;
  reason: string;
  traffic_share: number;
  /** The share it would replace; null means uncapped. */
  current: number | null;
  at: string;
  agent: { name: string; justification?: string | null; on_behalf_of?: string | null; token?: string; audit_id?: number; tool?: string };
}
export interface Approvals {
  drafts: ApprovalCard[];
  share_requests: ShareRequest[];
  max_share: number;
  default_share: number;
  recent: { action: string; actor: string; slot: string; variant: string; comment: string | null; traffic_share: number | null; at: string }[];
}
export interface ProbeStep {
  id: 'framework' | 'package' | 'reach' | 'slot' | 'agent' | 'live';
  state: 'choice' | 'done' | 'pending' | 'failed' | 'later';
  detail: string;
}
export interface McpCall {
  id: number;
  tool: string;
  agent: string;
  on_behalf_of: string | null;
  justification: string | null;
  outcome: 'ok' | 'error' | 'refused';
  error: string | null;
  call_id: unknown;
  token: string;
  at: string;
}
export interface McpTail {
  site: string;
  endpoint: string;
  tokens: { name: string; created_at: string; revoked_at: string | null; last_used: string | null; calls: number }[];
  calls: McpCall[];
}
export interface McpCallDetail extends McpCall {
  site: string;
  arguments: Record<string, unknown>;
  variants: { id: number; slot: string; key: string; status: string }[];
}

export type WebhookEvent = 'decision' | 'exposure' | 'conversion' | 'variant.approved' | 'variant.retired' | 'variant.share_changed' | 'auto_rollback';
export interface WebhookSubscription {
  id: number;
  url: string;
  events: WebhookEvent[];
  created_at: string;
  counts: { pending: number; delivered: number; failed: number };
  last_delivered_at: string | null;
}
export interface WebhookList {
  subscriptions: WebhookSubscription[];
  event_types: WebhookEvent[];
}
export interface WebhookCreated {
  id: number;
  url: string;
  events: WebhookEvent[];
  secret: string;
}
export interface WebhookDelivery {
  id: number;
  event: WebhookEvent;
  idempotency_key: string;
  status: 'pending' | 'delivered' | 'failed';
  attempts: number;
  last_status: number | null;
  last_error: string | null;
  next_attempt_at: string | null;
  delivered_at: string | null;
  created_at: string;
}
