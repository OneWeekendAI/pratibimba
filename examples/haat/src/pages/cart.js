import { h, byId, cart, art, inr, esc, products, strip, deliveryBy, crossPool } from '../lib.js';
import { emptyCart } from './states.js';

export const FREE_OVER = 499;
export const FEE = 40;
export const feeFor = (sub) => (sub >= FREE_OVER || sub === 0 ? 0 : FEE);

export function cartPage() {
  const inCart = new Set(Object.keys(cart.items()));
  // Cross-sell renders once from the cart on arrival; later edits only touch the lines.
  const cats = new Set([...inCart].map((id) => byId.get(id).category));
  const avail = crossPool.filter((p) => !inCart.has(p.id)); // the declared cross-sell pool (site.json), minus what is in the cart
  const cross = [...avail.filter((p) => cats.has(p.category)), ...avail.filter((p) => !cats.has(p.category))].slice(0, 8);
  const el = h(`<div class="cartpg">
    <div id="top"></div>
    <div data-slot="cart-cross-sell" data-testid="cross-sell">${strip(inCart.size ? 'Add-ons you might like' : 'Popular right now', cross, '', 'data-prb-slot="cart-cross-sell"')}</div>
  </div>`);
  const top = el.querySelector('#top');
  function paint() {
    const entries = Object.entries(cart.items());
    if (!entries.length) { top.replaceChildren(h('<div><h1>Your cart</h1><div id="lines" data-testid="lines"></div></div>')); top.firstElementChild.querySelector('#lines').append(emptyCart()); return; }
    const sub = cart.total();
    const mrp = entries.reduce((a, [id, q]) => a + byId.get(id).mrp * q, 0);
    const fee = feeFor(sub);
    const left = Math.max(0, FREE_OVER - sub);
    const pct = Math.min(100, Math.round((sub / FREE_OVER) * 100));
    const n = cart.count();
    top.replaceChildren(h(`<div class="cart-grid">
      <section class="cart-main">
        <div class="cart-head"><h1>Your cart</h1><span class="muted">${n} ${n === 1 ? 'item' : 'items'}</span></div>
        <div id="lines" data-testid="lines">${entries.map(([id, q]) => { const p = byId.get(id); return `<div class="line" data-line="${id}">
          <a class="line-img" href="#/product/${id}">${art(p)}</a>
          <div class="line-info"><a class="line-title" href="#/product/${id}">${esc(p.title)}</a>
            <div class="line-price"><b>${inr(p.price)}</b> <s>${inr(p.mrp)}</s></div>
            <div class="line-deliv">${p.stock <= 4 ? `<span class="low">Only ${p.stock} left</span> ` : ''}Free delivery by ${deliveryBy(p.stock % 3 + 1)}</div></div>
          <div class="line-ctl"><div class="qty" role="group" aria-label="Quantity"><button data-dec="${id}" aria-label="Fewer">&minus;</button><span data-testid="line-qty" aria-live="polite">${q}</span><button data-inc="${id}" aria-label="More">+</button></div>
            <button class="link-btn" data-rm="${id}">Remove</button></div>
          <div class="line-sub" data-testid="line-sub">${inr(p.price * q)}</div></div>`; }).join('')}</div>
      </section>
      <aside class="sumbox" data-testid="summary">
        <div class="free ${left ? '' : 'full'}" data-testid="free-bar">
          <p data-testid="free-msg">${left ? `Add <b>${inr(left)}</b> more for free delivery` : '<b>You get free delivery</b> on this order'}</p>
          <div class="bar-track" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${pct}" aria-label="Progress to free delivery"><i style="width:${pct}%"></i></div>
        </div>
        <dl class="sum-rows">
          <div><dt>Items (${n})</dt><dd>${inr(mrp)}</dd></div>
          ${mrp > sub ? `<div class="save"><dt>Discount</dt><dd>&minus;${inr(mrp - sub)}</dd></div>` : ''}
          <div><dt>Delivery</dt><dd>${fee ? inr(fee) : 'Free'}</dd></div>
          <div class="sum-total"><dt>Total</dt><dd data-testid="cart-total">${inr(sub + fee)}</dd></div>
        </dl>
        ${mrp > sub ? `<p class="sum-save">You save ${inr(mrp - sub)} on this order</p>` : ''}
        <a class="btn block" href="#/checkout" data-testid="to-checkout">Proceed to checkout</a>
        <p class="sum-note">Cash on delivery and UPI accepted. 7-day easy returns.</p>
      </aside>
    </div>`));
  }
  const onClick = (e) => {
    const t = e.target.closest('[data-inc],[data-dec],[data-rm]');
    if (!t) return;
    const key = t.dataset.inc ? 'inc' : t.dataset.dec ? 'dec' : 'rm';
    const id = t.dataset[key];
    const q = cart.items()[id] || 0;
    cart.set(id, key === 'inc' ? q + 1 : key === 'dec' ? q - 1 : 0);
    // keep keyboard focus on the same stepper button after the repaint
    const again = el.querySelector(`[data-${key}="${id}"]`) || el.querySelector('[data-testid=to-checkout]');
    again?.focus();
  };
  const onCart = () => paint();
  el.addEventListener('click', onClick);
  window.addEventListener('haat:cart', onCart);
  paint();
  return { el, cleanup: () => window.removeEventListener('haat:cart', onCart) };
}
