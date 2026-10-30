// The Preview room's visitor model: a few plain toggles, turned into the decide context the
// site's /preview reads. Pure, so the rules are testable without a page.

export type Toggle = 'returning' | 'mobile' | 'from_ad' | 'evening';

export const TOGGLES: { key: Toggle; label: string }[] = [
  { key: 'returning', label: 'Returning' },
  { key: 'mobile', label: 'On phone' },
  { key: 'from_ad', label: 'From an ad' },
  { key: 'evening', label: 'Evening' },
];

/** The toggles, and the synthetic history (a persona key from `/api/personas`) templates fill from. */
export type Visitor = Record<Toggle, boolean> & { persona: string };

export const FIRST_VISIT: Visitor = { returning: false, mobile: false, from_ad: false, evening: false, persona: 'first_visit' };

/**
 * Minutes east of UTC that put the visitor's local clock at `hour:minute` right now, inside the
 * range decide accepts (-720 to 840). The span is 26 hours, so every local time has one.
 */
export function offsetForLocal(now: Date, hour: number, minute = 0): number {
  let off = hour * 60 + minute - (now.getUTCHours() * 60 + now.getUTCMinutes());
  while (off < -720) off += 1440;
  while (off > 840) off -= 1440;
  return off;
}

/** An evening visitor is at 19:30 local; anyone else at 11:00, so a preview never depends on when you look. */
export function ctxOf(v: Visitor, now: Date = new Date()) {
  return {
    returning: v.returning,
    mobile: v.mobile,
    from_ad: v.from_ad,
    tz_offset: v.evening ? offsetForLocal(now, 19, 30) : offsetForLocal(now, 11, 0),
  };
}

export interface RoomVersion {
  id?: number;
  key: string;
  status: string;
  baseline: boolean;
  why: string;
  /** P1: the persona lacks a fact this version requires; it gets the page's own instead. */
  closed?: boolean;
  /** P1: a template, filled from the persona's history. */
  filled?: boolean;
}
export interface RoomSlot {
  slot: string;
  kind: string;
  index: number;
  changed: boolean;
  on_page: boolean;
  versions: RoomVersion[];
  kept_out: { key: string; why: string }[];
}
export interface RoomRect {
  top: number;
  left: number;
  width: number;
  height: number;
  bottom: number;
}

/**
 * The visitor an audience's predicate describes, built on `base`: the room's four toggles cover
 * returning, mobile, from_ad and a local-hour window (the evening visitor is at 19:30, anyone else
 * at 11:00). Null when the predicate needs something the toggles cannot say.
 */
export function visitorFor(predicate: Record<string, unknown> | null | undefined, base: Visitor): Visitor | null {
  const v = { ...base };
  for (const [k, val] of Object.entries(predicate ?? {})) {
    if (k === 'returning' || k === 'mobile' || k === 'from_ad') {
      if (typeof val !== 'boolean') return null;
      v[k] = val;
    } else if (k === 'hour_local' && Array.isArray(val) && val.length === 2 && val.every((x) => typeof x === 'number')) {
      const [a, b] = val as number[];
      const within = (h: number) => (a <= b ? h >= a && h <= b : h >= a || h <= b);
      if (within(19.5)) v.evening = true;
      else if (within(11)) v.evening = false;
      else return null;
    } else return null;
  }
  return v;
}

export const sameVisitor = (a: Visitor, b: Visitor) => a.persona === b.persona && TOGGLES.every((t) => a[t.key] === b[t.key]);

export type CardStatus = 'original' | 'live' | 'draft' | 'paused';

/** One design of a slot, as the explorer lists it. */
export interface VariantCard {
  key: string;
  id?: number;
  status: CardStatus;
  baseline: boolean;
  /** The audience it is made for; null when it is open to every visitor. */
  audience: string | null;
  /** The toggle the audience turns on, for its icon. */
  audienceKey: Toggle | null;
  share: number | null;
  why: string | null;
  /** P1: this visitor's history lacks what the design requires, so they get the page's own. */
  closed: boolean;
  /** A visitor who gets this design (to show it on the page and in its thumbnail); null when none can. */
  visitor: Visitor | null;
  /** The same, built on a first visit: a thumbnail does not change when the room's visitor does. */
  thumbVisitor: Visitor | null;
}

interface DetailLike {
  kind: string;
  variants: { id: number; key: string; status: string; is_baseline: boolean; traffic_share: number | null; audience?: number | null }[];
  audiences: { id: number; name: string; predicate: Record<string, unknown> }[];
}

const RANK: Record<CardStatus, number> = { original: 0, live: 1, draft: 2, paused: 3 };

/**
 * Every design of a slot: the page's own first, then live, drafts and paused ones. `room` is what
 * the page's preview lists for the current visitor; `detail` is the API's slot, which also knows
 * the designs made for other audiences and the paused ones. Either may be missing.
 */
export function cardsOf(room: RoomSlot | null, detail: DetailLike | null, visitor: Visitor): VariantCard[] {
  const inRoom = (key: string) => room?.versions.find((v) => v.key === key) ?? null;
  // A sorted slot's designs are one list in the default order; a picker asks the visitor.
  if (room && (room.kind === 'sorted' || room.kind === 'picker')) {
    return room.versions.map((v) => ({ key: v.key, id: v.id, status: 'original', baseline: true, audience: null, audienceKey: null, share: null, why: v.why, closed: false, visitor, thumbVisitor: FIRST_VISIT }));
  }
  if (!detail) {
    return (room?.versions ?? []).map((v) => ({
      key: v.key, id: v.id, status: v.baseline ? 'original' : v.status === 'draft' ? 'draft' : 'live', baseline: v.baseline,
      audience: null, audienceKey: null, share: null, why: v.why, closed: !!v.closed, visitor, thumbVisitor: null,
    }));
  }
  const cards: VariantCard[] = [];
  for (const v of detail.variants) {
    if (v.status !== 'approved' && v.status !== 'draft' && v.status !== 'paused') continue;
    const aud = v.audience != null ? (detail.audiences.find((a) => a.id === v.audience) ?? null) : null;
    const status: CardStatus = v.is_baseline ? 'original' : v.status === 'approved' ? 'live' : v.status === 'draft' ? 'draft' : 'paused';
    const toggle = aud ? (Object.keys(aud.predicate).map((k) => (k === 'hour_local' ? 'evening' : k)).find((k) => TOGGLES.some((t) => t.key === k)) as Toggle | undefined) : undefined;
    const paused = status === 'paused';
    const here = inRoom(v.key);
    cards.push({
      key: v.key,
      id: v.id,
      status,
      baseline: v.is_baseline,
      audience: aud?.name ?? null,
      audienceKey: toggle ?? null,
      share: v.is_baseline ? null : v.traffic_share,
      why: here?.why ?? null,
      closed: !!here?.closed,
      visitor: paused ? null : here ? visitor : aud ? visitorFor(aud.predicate, visitor) : visitor,
      thumbVisitor: paused ? null : aud ? visitorFor(aud.predicate, FIRST_VISIT) : FIRST_VISIT,
    });
  }
  return cards.sort((a, b) => RANK[a.status] - RANK[b.status] || (a.id ?? 0) - (b.id ?? 0));
}
