import { h, byId, inr, orders as store, esc, art } from '../lib.js';

export function orders() {
  const all = [...store.all()].reverse();
  const body = all.length
    ? all.map((o) => `<article class="order"><div class="order-top"><b>${esc(o.id)}</b><span class="muted">${new Date(o.at).toLocaleDateString('en-IN', { day: 'numeric', month: 'short', year: 'numeric' })}</span></div>
        <ul>${Object.entries(o.items).map(([id, q]) => `<li><span class="thumb">${byId.get(id) ? art(byId.get(id)) : ''}</span><span>${esc(byId.get(id)?.title || id)} <span class="muted">x ${q}</span></span></li>`).join('')}</ul><div><div class="big">${inr(o.total)}</div></div></article>`).join('')
    : '<div class="state"><h2>No orders yet</h2><p>When you place an order it will show up here.</p><div class="state-cta"><a class="btn" href="#/">Start shopping</a></div></div>';
  return { el: h(`<div data-testid="orders"><h1>Your orders</h1><div class="stack">${body}</div></div>`) };
}
