import { h, byId, cart, orders, art, inr, esc, deliveryBy, prbTrack } from '../lib.js';
import { emptyCart } from './states.js';
import { feeFor } from './cart.js';

const DELIVERY = {
  standard: { label: 'Standard delivery', days: 3, sub: (s) => (feeFor(s) ? inr(feeFor(s)) : 'Free'), fee: (s) => feeFor(s) },
  express: { label: 'Express delivery', days: 1, sub: () => inr(99), fee: () => 99 },
};
const PAY = { upi: 'UPI', card: 'Debit or credit card', cod: 'Cash on delivery' };
const dayName = (days) => { const d = new Date(); d.setDate(d.getDate() + days); return d.toLocaleDateString('en-IN', { weekday: 'long', day: 'numeric', month: 'short' }); };

export function checkout() {
  if (!cart.count()) {
    const last = orders.all().at(-1);
    const el = h('<div class="co-empty"><h1>Checkout</h1></div>');
    el.append(emptyCart('cart-empty', 'There is nothing to check out yet.'));
    if (last) el.append(h(`<p class="state-last">Your last order was <code>${esc(last.id)}</code>. <a href="#/orders">View orders</a></p>`));
    return { el };
  }
  const entries = Object.entries(cart.items());
  const sub = cart.total();
  const el = h(`<div class="co">
    <ol class="steps" aria-label="Checkout steps"><li class="on"><i>1</i> Address</li><li class="on"><i>2</i> Delivery</li><li class="on"><i>3</i> Payment</li></ol>
    <h1>Checkout</h1>
    <div class="co-grid">
      <form class="co-main" id="coform" data-testid="checkout-form">
        <section class="co-card"><h2><i>1</i> Delivery address</h2>
          <div class="fields">
            <label>Full name<input type="text" name="name" required autocomplete="name" value="Asha Verma" /></label>
            <label>Mobile number<input type="tel" name="phone" required inputmode="numeric" pattern="[0-9]{10}" autocomplete="tel-national" value="9876543210" /></label>
            <label class="wide">Address<textarea name="addr" required rows="2" autocomplete="street-address">12 Market Road, Indiranagar</textarea></label>
            <label>City<input type="text" name="city" required value="Bengaluru" /></label>
            <label>PIN code<input type="text" name="pin" required inputmode="numeric" pattern="[0-9]{6}" autocomplete="postal-code" value="560001" /></label>
          </div>
        </section>
        <section class="co-card"><h2><i>2</i> Delivery speed</h2>
          <div class="opts" data-testid="delivery-opts">${Object.entries(DELIVERY).map(([k, d], i) => `<label class="opt"><input type="radio" name="speed" value="${k}" ${i ? '' : 'checked'} /><span><b>${d.label}</b><small>Arrives ${dayName(d.days)}</small></span><em>${d.sub(sub)}</em></label>`).join('')}</div>
        </section>
        <section class="co-card"><h2><i>3</i> Payment method</h2>
          <div class="opts" data-testid="pay-opts">
            <label class="opt"><input type="radio" name="pay" value="upi" checked /><span><b>UPI</b><small>Pay with any UPI app</small></span></label>
            <div class="opt-body" data-for="upi"><label>UPI ID<input type="text" name="upi" placeholder="name@bank" value="asha@okbank" autocomplete="off" /></label></div>
            <label class="opt"><input type="radio" name="pay" value="card" /><span><b>Debit or credit card</b><small>Visa, Mastercard, RuPay</small></span></label>
            <div class="opt-body" data-for="card" hidden><div class="fields">
              <label class="wide">Card number<input type="text" name="cardno" inputmode="numeric" placeholder="1234 5678 9012 3456" autocomplete="off" /></label>
              <label>Expiry<input type="text" name="exp" placeholder="MM/YY" autocomplete="off" /></label>
              <label>CVV<input type="password" name="cvv" inputmode="numeric" maxlength="4" autocomplete="off" /></label></div></div>
            <label class="opt"><input type="radio" name="pay" value="cod" /><span><b>Cash on delivery</b><small>Pay when your order arrives</small></span></label>
            <div class="opt-body" data-for="cod" hidden><p class="muted">Keep the exact amount ready if you can. No extra charge.</p></div>
          </div>
        </section>
      </form>
      <aside class="sumbox co-sum" data-testid="co-summary">
        <h2>Order summary</h2>
        <ul class="co-items">${entries.map(([id, q]) => { const p = byId.get(id); return `<li><span class="thumb">${art(p)}</span><span class="t">${esc(p.title)}<small>Qty ${q}</small></span><b>${inr(p.price * q)}</b></li>`; }).join('')}</ul>
        <dl class="sum-rows">
          <div><dt>Items</dt><dd>${inr(sub)}</dd></div>
          <div><dt>Delivery</dt><dd id="co-fee"></dd></div>
          <div class="sum-total"><dt>Total payable</dt><dd id="co-total" data-testid="co-total"></dd></div>
        </dl>
        <button class="btn block" form="coform" data-testid="place-order">Place order</button>
        <p class="sum-note">Demo store. No payment is taken.</p>
      </aside>
    </div>
  </div>`);
  const form = el.querySelector('form');
  const val = (n) => form.querySelector(`[name=${n}]:checked`)?.value;
  const totals = () => { const fee = DELIVERY[val('speed')].fee(sub); el.querySelector('#co-fee').textContent = fee ? inr(fee) : 'Free'; el.querySelector('#co-total').textContent = inr(sub + fee); return sub + fee; };
  const panels = () => {
    const pay = val('pay');
    el.querySelectorAll('.opt-body').forEach((b) => { b.hidden = b.dataset.for !== pay; });
    el.querySelectorAll('[name=pay]').forEach((r) => r.closest('.opt').classList.toggle('sel', r.checked));
    el.querySelectorAll('[name=speed]').forEach((r) => r.closest('.opt').classList.toggle('sel', r.checked));
  };
  form.addEventListener('change', () => { totals(); panels(); });
  totals(); panels();
  form.addEventListener('submit', (e) => {
    e.preventDefault();
    const f = new FormData(form);
    const total = totals();
    const speed = f.get('speed'), pay = f.get('pay');
    const o = orders.place({ name: f.get('name'), phone: f.get('phone'), addr: `${f.get('addr')}, ${f.get('city')} ${f.get('pin')}`, speed, payment: pay, total });
    prbTrack.purchase(o); // the confirmed order: real order id, total in rupees
    const lines = Object.entries(o.items).map(([id, q]) => { const p = byId.get(id); return `<li><span class="thumb">${art(p)}</span><span class="t">${esc(p.title)}<small>Qty ${q}</small></span><b>${inr(p.price * q)}</b></li>`; }).join('');
    el.replaceChildren(h(`<div class="confirm" data-testid="order-done">
      <div class="confirm-head"><svg viewBox="0 0 48 48" aria-hidden="true"><circle cx="24" cy="24" r="22" fill="#1f8a4c"/><path d="M14 25l7 7 13-15" fill="none" stroke="#fff" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"/></svg>
        <div><h1>Thank you, your order is placed</h1><p>Arriving ${dayName(DELIVERY[speed].days)}. A confirmation would be sent to ${esc(f.get('phone'))}.</p></div></div>
      <div class="confirm-id"><span>Order id</span><code data-testid="order-id">${esc(o.id)}</code></div>
      <dl class="confirm-meta"><div><dt>Ship to</dt><dd>${esc(o.name)}<br>${esc(o.addr)}</dd></div><div><dt>Payment</dt><dd>${PAY[pay]}</dd></div><div><dt>Total</dt><dd><b>${inr(o.total)}</b></dd></div></dl>
      <ul class="co-items">${lines}</ul>
      <div class="state-cta"><a class="btn" href="#/">Continue shopping</a><a class="btn alt" href="#/orders">Your orders</a></div></div>`));
    window.scrollTo(0, 0);
  });
  return { el };
}
