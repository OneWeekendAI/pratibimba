'use client';
// Reports that a server-rendered slot was actually shown (once it hydrates) and clicks inside
// it. A decision nobody saw must teach the model nothing, so server rendering alone is not an
// exposure. Renders nothing.

import { useEffect } from 'react';

export interface BeaconProps {
  site: string;
  slot: string;
  decision: string;
  subject: string;
  eventsPath: string;
}

const reported = new Set<string>();

function send(p: BeaconProps, name: string) {
  const body = JSON.stringify({
    site: p.site,
    subject: p.subject,
    events: [{ id: crypto.randomUUID(), name, slot: p.slot, decision_id: p.decision }],
  });
  void fetch(p.eventsPath, { method: 'POST', headers: { 'content-type': 'application/json' }, body, keepalive: true }).catch(() => {});
}

export function PrbBeacon(p: BeaconProps) {
  useEffect(() => {
    // Strict Mode runs effects twice in development; one exposure per decision per page.
    if (!reported.has(p.decision)) {
      reported.add(p.decision);
      send(p, 'exposure');
    }
    const el = document.querySelector(`[data-prb-decision="${p.decision}"]`);
    if (!el) return;
    const onClick = () => send(p, 'click');
    el.addEventListener('click', onClick);
    return () => el.removeEventListener('click', onClick);
  }, [p.decision]);
  return null;
}

/** For tests: forget which decisions were reported. */
export function __resetBeacon() {
  reported.clear();
}
