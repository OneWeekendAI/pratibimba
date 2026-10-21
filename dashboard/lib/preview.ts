// The Preview room's visitor model: a few plain toggles, turned into the decide context the
// site's /preview reads. Pure, so the rules are testable without a page.

export type Toggle = 'returning' | 'mobile' | 'from_ad' | 'evening';

export const TOGGLES: { key: Toggle; label: string }[] = [
  { key: 'returning', label: 'Returning' },
  { key: 'mobile', label: 'On phone' },
  { key: 'from_ad', label: 'From an ad' },
  { key: 'evening', label: 'Evening' },
];

export type Visitor = Record<Toggle, boolean>;

export const FIRST_VISIT: Visitor = { returning: false, mobile: false, from_ad: false, evening: false };

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
