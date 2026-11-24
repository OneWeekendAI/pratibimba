import { h, byId, cart, art, inr, esc, products, strip } from '../lib.js';

export function cartPage() {
  const inCart = new Set(Object.keys(cart.items()));
  // Cross-sell renders once from the cart on arrival; later edits only touch the lines.
  const cats = new Set([...inCart].map((id) => byId.get(id).category));
  const cross = products.filter((p) => !inCart.has(p.id) && (cats.size ? cats.has(p.category) : true) && p.price < 1500).sort((a, b) => b.rating - a.rating).slice(0, 8);
  const el = h(`<div>
    <h1>Your cart</h1>
    <div id="lines" data-testid="lines"></div>
    <div data-slot="cart-cross-sell" data-testid="cross-sell">${strip('Add-ons you might like', cross)}</div>
  </div>`);
  const lines = el.querySelector('#lines');
  function paint() {
    const items = Object.entries(cart.items());
    if (!items.length) { lines.innerHTML = '<p class="empty" data-testid="cart-empty">Your cart is empty. <a href="#/">Keep browsing</a></p>'; return; }
    lines.innerHTML = items.map(([id, q]) => { const p = byId.get(id); return `<div class="line" data-line="${id}">
      <a href="#/product/${id}">${art(p)}</a><div><a href="#/product/${id}"><b>${esc(p.title)}</b></a><div>${inr(p.price)}</div></div>
      <div class="qty"><button data-dec="${id}" aria-label="Fewer">-</button><span>${q}</span><button data-inc="${id}" aria-label="More">+</button></div>
      <button class="btn small alt" data-rm="${id}">Remove</button></div>`; }).join('') +
      `<div class="summary">Total <b data-testid="cart-total">${inr(cart.total())}</b></div><div style="text-align:right"><a class="btn" href="#/checkout" data-testid="to-checkout">Checkout</a></div>`;
  }
  const onClick = (e) => {
    const t = e.target.closest('[data-inc],[data-dec],[data-rm]');
    if (!t) return;
    const id = t.dataset.inc || t.dataset.dec || t.dataset.rm;
    const q = cart.items()[id] || 0;
    cart.set(id, t.dataset.inc ? q + 1 : t.dataset.dec ? q - 1 : 0);
  };
  const onCart = () => paint();
  el.addEventListener('click', onClick);
  window.addEventListener('haat:cart', onCart);
  paint();
  return { el, cleanup: () => window.removeEventListener('haat:cart', onCart) };
}
