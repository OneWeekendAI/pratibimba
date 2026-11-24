import { h, cart, orders, inr, esc } from '../lib.js';

export function checkout() {
  if (!cart.count()) {
    const last = orders.all().at(-1);
    return { el: h(`<div><h1>Checkout</h1><p class="empty" data-testid="cart-empty">Nothing to check out. <a href="#/">Keep browsing</a></p>${last ? `<p>Last order: <code>${esc(last.id)}</code></p>` : ''}</div>`) };
  }
  const el = h(`<div><h1>Checkout</h1>
    <p>Total payable: <b>${inr(cart.total())}</b> (demo: no payment is taken)</p>
    <form class="form" data-testid="checkout-form">
      <label>Name<input type="text" name="name" required value="Asha Verma" /></label>
      <label>Delivery address<textarea name="addr" required rows="3">12 Market Road, Pune</textarea></label>
      <button class="btn" data-testid="place-order">Place order</button>
    </form></div>`);
  el.querySelector('form').addEventListener('submit', (e) => {
    e.preventDefault();
    const f = new FormData(e.target);
    const o = orders.place({ name: f.get('name'), addr: f.get('addr') });
    el.replaceChildren(h(`<div class="ok" data-testid="order-done"><h1>Thank you, order placed</h1>
      <p>Order id: <code data-testid="order-id">${esc(o.id)}</code></p><p>Total ${inr(o.total)}. We will deliver in about 3 days.</p>
      <a class="btn" href="#/">Continue shopping</a></div>`));
  });
  return { el };
}
