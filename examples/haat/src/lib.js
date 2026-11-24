import catalogue from './data/catalogue.json';

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

// ---- generated flat illustrations: deterministic from category + seed ----
const PAL = [['#e9c9a8', '#b4532f'], ['#d9e1ea', '#2f3d7a'], ['#efe0b8', '#8a6a1f'], ['#d6e4d2', '#3d6b4a'], ['#f0d3cf', '#a63d40'], ['#dcd3ea', '#5a4a8c']];
function mulberry(a) {
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
export function art(p) {
  const r = mulberry(p.seed);
  const [bg, fg] = PAL[Math.floor(r() * PAL.length)];
  const light = '#fbf6ee';
  const j = () => Math.round(r() * 20 - 10);
  let g;
  switch (p.category) {
    case 'electronics': g = `<rect x="55" y="40" width="90" height="120" rx="14" fill="${fg}"/><rect x="64" y="52" width="72" height="80" rx="6" fill="${light}"/><circle cx="100" cy="146" r="6" fill="${light}"/>`; break;
    case 'kitchen': g = `<path d="M50 80h100l-10 70a10 10 0 0 1-10 8H70a10 10 0 0 1-10-8z" fill="${fg}"/><rect x="45" y="68" width="110" height="14" rx="6" fill="${light}"/><path d="M150 95h22a8 8 0 0 1 0 28h-16" fill="none" stroke="${fg}" stroke-width="8"/>`; break;
    case 'books': g = `<rect x="52" y="48" width="96" height="108" rx="6" fill="${fg}"/><rect x="62" y="48" width="8" height="108" fill="${light}" opacity=".5"/><rect x="82" y="72" width="52" height="8" rx="3" fill="${light}"/><rect x="82" y="90" width="38" height="8" rx="3" fill="${light}"/>`; break;
    case 'fashion': g = `<path d="M70 50l30 14 30-14 28 24-20 16v66H62V90L42 74z" fill="${fg}"/><path d="M85 52l15 18 15-18" fill="none" stroke="${light}" stroke-width="5"/>`; break;
    case 'home': g = `<rect x="48" y="118" width="104" height="36" rx="8" fill="${fg}"/><rect x="58" y="88" width="84" height="38" rx="14" fill="${light}" stroke="${fg}" stroke-width="6"/><rect x="62" y="152" width="8" height="12" fill="${fg}"/><rect x="130" y="152" width="8" height="12" fill="${fg}"/>`; break;
    case 'toys': g = `<rect x="50" y="100" width="56" height="56" rx="6" fill="${fg}"/><circle cx="130" cy="124" r="30" fill="${light}" stroke="${fg}" stroke-width="8"/><path d="M60 100l20-34 20 34z" fill="${light}" stroke="${fg}" stroke-width="6"/>`; break;
    case 'grocery': g = `<path d="M62 70h76l8 86H54z" fill="${fg}"/><path d="M80 70a20 20 0 0 1 40 0" fill="none" stroke="${fg}" stroke-width="7"/><circle cx="100" cy="116" r="18" fill="${light}"/>`; break;
    default: g = `<rect x="76" y="40" width="48" height="26" rx="6" fill="${light}" stroke="${fg}" stroke-width="6"/><rect x="62" y="66" width="76" height="92" rx="16" fill="${fg}"/><ellipse cx="100" cy="112" rx="14" ry="22" fill="${light}"/>`;
  }
  const dots = `<circle cx="${30 + j()}" cy="${34 + j()}" r="${6 + Math.floor(r() * 8)}" fill="${fg}" opacity=".12"/><circle cx="${170 + j()}" cy="${170 + j()}" r="${10 + Math.floor(r() * 10)}" fill="${fg}" opacity=".12"/>`;
  return `<svg class="art" viewBox="0 0 200 200" role="img" aria-label="${esc(p.title)}"><rect width="200" height="200" fill="${bg}"/>${dots}${g}</svg>`;
}

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

export function card(p) {
  return `<article class="card" data-id="${p.id}">
    <a class="card-img" href="#/product/${p.id}">${art(p)}</a>
    <a class="card-title" href="#/product/${p.id}">${esc(p.title)}</a>
    <div class="rating">${p.rating.toFixed(1)} &#9733; <span>(${p.reviews.toLocaleString('en-IN')})</span></div>
    <div class="price"><b>${inr(p.price)}</b> <s>${inr(p.mrp)}</s> <em>${off(p)}% off</em></div>
    <button class="btn small" data-add="${p.id}">Add to cart</button>
  </article>`;
}
export const strip = (title, list, extra = '') => `<section class="strip" ${extra}><h2>${title}</h2><div class="row">${list.map(card).join('')}</div></section>`;
