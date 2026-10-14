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
//   <prb.Sorted id="grid">{products.map((p) => <Card key={p.slug} p={p} />)}</prb.Sorted>

import { postEvents, type Ctx, type Decision, type Fetch } from './core.ts';
import { createProxy, type ProxyOptions } from './proxy.ts';
import { createRoute } from './route.ts';
import { createSlot, type SlotConfig, type SortedProps } from './slot.ts';
import { addToCartEvent, purchaseEvent, type Item, type Order } from './commerce.ts';

export type { Ctx, Decision, SlotConfig, SortedProps, ProxyOptions, Item, Order };
export { createProxy, createRoute, createSlot };
export { encodeCtx, decodeCtx, contextFor, SUBJECT_COOKIE, SESSION_COOKIE, SDK_HEADER, VERSION } from './context.ts';
export { TZ_COOKIE, GEO_TZ_HEADERS, validTimeZone, visitorTimeZone } from './timezone.ts';
export { CONSENT_COOKIE, visitorConsent, type ConsentOption } from './consent.ts';
export { MANIFEST_TTL_MS, type FeatureValues } from './features.ts';

export interface PratibimbaConfig extends SlotConfig, ProxyOptions {}

export function createPratibimba(config: PratibimbaConfig) {
  const { Slot, Sorted, decide, visitor } = createSlot(config);
  return {
    Slot,
    /** A sorted slot: keyed children rendered in the decided order (see `SortedProps`). */
    Sorted,
    /** The decision for a slot in this request, for slots that are not a `<Slot>`. */
    decide,
    /** Post a goal (e.g. `purchase`) for this request's visitor from server code. */
    async track(name: string, opts: { slot?: string; decisionId?: string } = {}) {
      const v = await visitor();
      if (!v) return;
      await postEvents(config, v.subject, [{ id: crypto.randomUUID(), name, slot: opts.slot, decision_id: opts.decisionId }], v.ctx.consent);
    },
    /** Post `add_to_cart` with the item for this request's visitor. Its price is not revenue. */
    async addToCart(item: Item) {
      const ev = addToCartEvent(item);
      const v = await visitor();
      if (!v) return;
      await postEvents(config, v.subject, [ev], v.ctx.consent);
    },
    /**
     * Post `purchase` with the order for this request's visitor. Core shares its value among the
     * decisions in each slot's window and stores one row per `order_id`, so calling this twice
     * for one order counts it once. Throws a TypeError for a malformed order.
     */
    async purchase(order: Order) {
      const ev = purchaseEvent(order);
      const v = await visitor();
      if (!v) return;
      await postEvents(config, v.subject, [ev], v.ctx.consent);
    },
    proxy: createProxy(config),
    route: createRoute({ core: config.core, fetch: config.fetch as Fetch | undefined }),
  };
}
