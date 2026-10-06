// @pratibimba/next: decide during the server render, so the first byte carries the variant.
//
//   // lib/prb.ts
//   export const prb = createPratibimba({ core: process.env.PRB_CORE_URL!, site: 'shop' });
//   // proxy.ts
//   export const proxy = prb.proxy;
//   // app/_prb/[...path]/route.ts
//   export const { GET, POST } = prb.route;
//   // any server component
//   <prb.Slot id="hero" variants={{ gift: <GiftHero /> }}><Hero /></prb.Slot>

import { postEvents, type Ctx, type Decision, type Fetch } from './core.ts';
import { createProxy, type ProxyOptions } from './proxy.ts';
import { createRoute } from './route.ts';
import { createSlot, type SlotConfig } from './slot.ts';

export type { Ctx, Decision, SlotConfig, ProxyOptions };
export { createProxy, createRoute, createSlot };
export { encodeCtx, decodeCtx, contextFor, SUBJECT_COOKIE, SESSION_COOKIE, SDK_HEADER, VERSION } from './context.ts';
export { TZ_COOKIE, GEO_TZ_HEADERS, validTimeZone, visitorTimeZone } from './timezone.ts';

export interface PratibimbaConfig extends SlotConfig, ProxyOptions {}

export function createPratibimba(config: PratibimbaConfig) {
  const { Slot, decide, visitor } = createSlot(config);
  return {
    Slot,
    /** The decision for a slot in this request, for slots that are not a `<Slot>`. */
    decide,
    /** Post a goal (e.g. `purchase`) for this request's visitor from server code. */
    async track(name: string, opts: { slot?: string; decisionId?: string } = {}) {
      const v = await visitor();
      if (!v) return;
      await postEvents(config, v.subject, [{ id: crypto.randomUUID(), name, slot: opts.slot, decision_id: opts.decisionId }]);
    },
    proxy: createProxy(config),
    route: createRoute({ core: config.core, fetch: config.fetch as Fetch | undefined }),
  };
}
