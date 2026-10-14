'use client';
// Reports that a server-rendered slot was actually shown (once it hydrates) and clicks inside
// it. A decision nobody saw must teach the model nothing, so server rendering alone is not an
// exposure. Renders nothing.

import { useEffect } from 'react';
import { rememberTimeZone } from './timezone.ts';

export interface BeaconProps {
  site: string;
  slot: string;
  decision: string;
  subject: string;
  eventsPath: string;
  /** The consent the server decided with; events carry the same, so Core stores one subject. */
  consent?: boolean;
  /** A sorted slot's item keys as rendered: the exposure carries them, and a click inside an
   * element marked `data-prb-item` names that item (the signal Core's cascade learns from). */
  items?: string[];
}

const reported = new Set<string>();

function send(p: BeaconProps, name: string, items?: string[]) {
  const body = JSON.stringify({
    site: p.site,
    subject: p.subject,
    events: [{ id: crypto.randomUUID(), name, slot: p.slot, decision_id: p.decision, ...(items?.length ? { items: items.slice(0, 100).map((id) => ({ id })) } : {}) }],
    // The browser's offset, so insights put this visit at its local hour.
    tz_offset: -new Date().getTimezoneOffset(),
    consent: p.consent === true,
  });
  void fetch(p.eventsPath, { method: 'POST', headers: { 'content-type': 'application/json' }, body, keepalive: true }).catch(() => {});
}

export function PrbBeacon(p: BeaconProps) {
  useEffect(() => {
    rememberTimeZone(document, p.consent === true);
    // Strict Mode runs effects twice in development; one exposure per decision per page.
    if (!reported.has(p.decision)) {
      reported.add(p.decision);
      send(p, 'exposure', p.items);
    }
    const el = document.querySelector(`[data-prb-decision="${p.decision}"]`);
    if (!el) return;
    const onClick = (e: Event) => {
      const item = p.items ? (e.target as Element | null)?.closest?.('[data-prb-item]') : null;
      send(p, 'click', item && el.contains(item) ? [item.getAttribute('data-prb-item')!] : undefined);
    };
    el.addEventListener('click', onClick);
    return () => el.removeEventListener('click', onClick);
  }, [p.decision]);
  return null;
}

/** For tests: forget which decisions were reported. */
export function __resetBeacon() {
  reported.clear();
}
