// Commerce events (F1): the events `addToCart` and `purchase` post, checked as Core checks them,
// so a malformed order fails in the developer's code rather than as a 400 nobody reads.
// Core derives a purchase's event id from `order_id`: one order is one row however often it is
// sent (a reloaded thank-you page, the browser and the server both reporting it).
// The same file is in @pratibimba/next (src/commerce.ts); keep the two copies identical.

export interface Item {
  id: string;
  /** Unit price in the currency's major unit. */
  price?: number;
  quantity?: number;
  [key: string]: unknown;
}

export interface Order {
  /** Your order id: the idempotency key. */
  order_id: string;
  /** The order total in the currency's major unit (49.99, not 4999). */
  value: number;
  /** ISO 4217, e.g. 'USD' or 'INR'. */
  currency: string;
  items?: Item[];
}

export interface CommerceEvent {
  id: string;
  name: 'purchase' | 'add_to_cart';
  order_id?: string;
  value?: number;
  currency?: string;
  items?: Item[];
}

function bad(msg: string): never {
  throw new TypeError(`[pratibimba] ${msg}`);
}

function checkItems(items: unknown): Item[] | undefined {
  if (items === undefined) return undefined;
  if (!Array.isArray(items)) bad('items must be an array of {id, price?, quantity?}');
  for (const it of items) {
    // Core counts bytes (UTF-8), so a 200-character id of accented letters can be too long.
    if (!it || typeof it.id !== 'string' || !it.id || new TextEncoder().encode(it.id).length > 256) bad('each item needs a string id (1-256 bytes)');
    if (it.price !== undefined && !(typeof it.price === 'number' && Number.isFinite(it.price) && it.price >= 0)) bad('item price must be a number >= 0');
    if (it.quantity !== undefined && !(Number.isInteger(it.quantity) && it.quantity >= 1)) bad('item quantity must be a whole number >= 1');
  }
  return items as Item[];
}

export function purchaseEvent(order: Order): CommerceEvent {
  const o = (order ?? {}) as Partial<Order>;
  if (typeof o.order_id !== 'string' || !o.order_id || o.order_id.length > 256) bad('purchase needs an order_id (1-256 chars)');
  if (!(typeof o.value === 'number' && Number.isFinite(o.value) && o.value >= 0)) bad('purchase value must be a number >= 0, in major units (49.99)');
  if (typeof o.currency !== 'string' || !/^[A-Za-z]{3}$/.test(o.currency)) bad('purchase currency must be an ISO 4217 code, e.g. USD');
  const items = checkItems(o.items);
  return {
    id: crypto.randomUUID(),
    name: 'purchase',
    order_id: o.order_id,
    value: o.value,
    currency: o.currency.toUpperCase(),
    ...(items ? { items } : {}),
  };
}

export function addToCartEvent(item: Item): CommerceEvent {
  return { id: crypto.randomUUID(), name: 'add_to_cart', items: checkItems([item]) };
}
