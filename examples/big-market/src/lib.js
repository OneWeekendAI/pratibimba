import catalogue from './data/catalogue.json';
import { art } from './art.js';

export const products = catalogue;
export const byId = new Map(products.map((p) => [p.id, p]));
export const CATEGORIES = ['electronics', 'kitchen', 'books', 'fashion', 'home', 'toys', 'grocery', 'beauty'];
export const cap = (s) => s[0].toUpperCase() + s.slice(1);
export const inr = (n) => '₹' + n.toLocaleString('en-IN');
export const off = (p) => Math.round((1 - p.price / p.mrp) * 100);

export function h(html) {
  const t = document.createElement('template');
  t.innerHTML = html.trim();
  return t.content.firstElementChild;
}
export const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);

export { art };

// ---- cart + orders (localStorage) ----
const CART_KEY = 'haat.cart';
const ORDERS_KEY = 'haat.orders';
const read = (k, d) => { try { return JSON.parse(localStorage.getItem(k)) ?? d; } catch { return d; } };
export const cart = {
  items: () => read(CART_KEY, {}),
  count() { return Object.values(this.items()).reduce((a, b) => a + b, 0); },
  total() { return Object.entries(this.items()).reduce((a, [id, q]) => a + byId.get(id).price * q, 0); },
  set(id, q) {
    const it = this.items();
    if (q <= 0) delete it[id]; else it[id] = q;
    localStorage.setItem(CART_KEY, JSON.stringify(it));
    window.dispatchEvent(new CustomEvent('haat:cart'));
  },
  add(id) { this.set(id, (this.items()[id] || 0) + 1); },
  clear() { localStorage.removeItem(CART_KEY); window.dispatchEvent(new CustomEvent('haat:cart')); },
};
export const orders = {
  all: () => read(ORDERS_KEY, []),
  place(info) {
    const rid = () => (crypto.randomUUID ? crypto.randomUUID() : 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => { const v = Math.random() * 16 | 0; return (c === 'x' ? v : (v & 3) | 8).toString(16); }));
    const order = { id: 'HAT-' + rid(), at: new Date().toISOString(), items: cart.items(), total: cart.total(), ...info };
    localStorage.setItem(ORDERS_KEY, JSON.stringify([...this.all(), order]));
    cart.clear();
    return order;
  },
};

const STAR = 'M10 1.5l2.5 5.4 5.9.7-4.3 4 1.1 5.8L10 14.6 4.8 17.4l1.1-5.8-4.3-4 5.9-.7z';
export const stars = (v) => `<span class="stars" style="--v:${v}" role="img" aria-label="${v.toFixed(1)} out of 5 stars"><svg viewBox="0 0 100 20" aria-hidden="true">${[0, 1, 2, 3, 4].map((i) => `<path transform="translate(${i * 20} 0)" d="${STAR}"/>`).join('')}</svg><span class="stars-fill"><svg viewBox="0 0 100 20" aria-hidden="true">${[0, 1, 2, 3, 4].map((i) => `<path transform="translate(${i * 20} 0)" d="${STAR}"/>`).join('')}</svg></span></span>`;
export function tagOf(p) {
  if (p.stock <= 4) return ['low', `Only ${p.stock} left`];
  if (p.tags.includes('new')) return ['new', 'New'];
  if (p.tags.includes('gift')) return ['gift', 'Great for gifting'];
  if (p.reviews > 2500 && p.rating >= 4.2) return ['best', 'Bestseller'];
  return null;
}
export function deliveryBy(days = 2) {
  const d = new Date(); d.setDate(d.getDate() + days);
  return d.toLocaleDateString('en-IN', { weekday: 'short' });
}
export function card(p) {
  const t = tagOf(p);
  return `<article class="card" data-id="${p.id}" data-prb-item="${p.id}">
    <a class="card-img" href="#/product/${p.id}">${art(p)}</a>
    <div class="card-tag">${t ? `<span class="tag tag-${t[0]}">${t[1]}</span>` : ''}</div>
    <a class="card-title" href="#/product/${p.id}">${esc(p.title)}</a>
    <div class="rating">${stars(p.rating)} <span>(${p.reviews.toLocaleString('en-IN')})</span></div>
    <div class="price"><b>${inr(p.price)}</b> <s>${inr(p.mrp)}</s> <em>${off(p)}% off</em></div>
    <div class="deliv">${p.price >= 499 ? 'Free delivery' : 'Delivery'} by ${deliveryBy(p.stock % 3 + 1)}</div>
    <button class="btn small" data-add="${p.id}">Add to cart</button>
  </article>`;
}
export const strip = (title, list, extra = '', rowAttrs = '') => `<section class="strip" ${extra}><h2>${title}</h2><div class="row" ${rowAttrs}>${list.map(card).join('')}</div></section>`;

// Cart cross-sell pool: the fixed set of add-ons the cross-sell slot may order (declared in site.json).
export const crossPool = [...products].filter((p) => p.price < 1500).sort((a, b) => b.rating - a.rating).slice(0, 16);

// ---- Pratibimba outcome events. Guarded: the page works the same when the SDK did not load. ----
export const prbTrack = {
  add(id, quantity = 1) {
    try { const p = byId.get(id); window.prb && window.prb.addToCart({ id, price: p.price, quantity }); } catch (e) { /* analytics never breaks the store */ }
  },
  purchase(order) {
    try {
      window.prb && window.prb.purchase({
        order_id: order.id, value: order.total, currency: 'INR',
        items: Object.entries(order.items).map(([id, quantity]) => ({ id, price: byId.get(id).price, quantity })),
      });
    } catch (e) { /* ditto */ }
  },
};
