// G1: the `_prb_tz` cookie is device storage, so the beacon writes it only when the site may
// remember this visitor, exactly when the browser SDK writes `_prb_subject`: `full`, or
// `consent_aware` with consent; never `cookieless`. The slot works that out from Core's
// manifest and hands the beacon a `remember` flag. No readable manifest means no cookie.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { createSlot } from '../src/slot.ts';

function core(mode: string | null) {
  return (async (url: string, init: any) => {
    if (url.includes('/manifest/')) {
      if (mode === null) return new Response('down', { status: 503 });
      return Response.json({ site: 'shop', privacy_mode: mode, slots: [], features: [] });
    }
    const body = JSON.parse(init.body);
    const decisions = body.slots.map((slot: string) => ({ decision_id: `d-${slot}`, slot, variant: 'gift', payload: null, order: ['a', 'b'], explanation: {} }));
    return Response.json({ decisions });
  }) as unknown as typeof globalThis.fetch;
}

const kit = (mode: string | null, consent: '1' | '0') =>
  createSlot({
    core: 'http://core',
    site: 'shop',
    fetch: core(mode),
    headers: async () => new Headers({ 'x-prb-subject': 'v1', 'x-prb-ctx': 'r1m0a0', 'x-prb-consent': consent }),
    requestScope: () => ({}),
  });

/** The beacon element's props among a rendered slot's children. */
function beaconOf(el: any): any {
  const kids = [el.props.children].flat(Infinity);
  const b = kids.find((k: any) => k && typeof k === 'object' && 'decision' in (k.props ?? {}));
  assert.ok(b, 'no beacon rendered');
  return b.props;
}

const slotRemember = async (mode: string | null, consent: '1' | '0') =>
  beaconOf(await kit(mode, consent).Slot({ id: 'hero', children: createElement('h1', null, 'Espresso') })).remember;

test('tz_cookie_allowed_on_a_full_site_without_consent', async () => {
  assert.equal(await slotRemember('full', '0'), true);
});

test('tz_cookie_allowed_on_a_consent_aware_site_with_consent', async () => {
  assert.equal(await slotRemember('consent_aware', '1'), true);
});

test('tz_cookie_refused_on_a_consent_aware_site_without_consent', async () => {
  assert.equal(await slotRemember('consent_aware', '0'), false);
});

test('tz_cookie_refused_on_a_cookieless_site_even_with_consent', async () => {
  assert.equal(await slotRemember('cookieless', '1'), false);
});

test('tz_cookie_refused_when_the_manifest_cannot_be_read', async () => {
  assert.equal(await slotRemember(null, '1'), false);
});

test('sorted_slot_hands_the_beacon_the_same_rule', async () => {
  const items = [createElement('li', { key: 'a' }, 'A'), createElement('li', { key: 'b' }, 'B')];
  assert.equal(beaconOf(await kit('consent_aware', '0').Sorted({ id: 'list', children: items })).remember, false);
  assert.equal(beaconOf(await kit('consent_aware', '1').Sorted({ id: 'list', children: items })).remember, true);
});

test('copy_slot_hands_the_beacon_the_same_rule', async () => {
  const render = (t: Record<string, string>) => createElement('h1', null, t.headline);
  assert.equal(beaconOf(await kit('cookieless', '1').Copy({ id: 'hero', fallback: { headline: 'x' }, children: render })).remember, false);
  assert.equal(beaconOf(await kit('full', '0').Copy({ id: 'hero', fallback: { headline: 'x' }, children: render })).remember, true);
});
