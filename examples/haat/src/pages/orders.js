import { h, byId, inr, orders as store, esc } from '../lib.js';

export function orders() {
  const all = [...store.all()].reverse();
  const body = all.length
    ? all.map((o) => `<article class="order"><div><b>${esc(o.id)}</b><span class="muted">${new Date(o.at).toLocaleDateString('en-IN', { day: 'numeric', month: 'short', year: 'numeric' })}</span></div>
        <ul>${Object.entries(o.items).map(([id, q]) => `<li>${esc(byId.get(id)?.title || id)} x ${q}</li>`).join('')}</ul><div class="big">${inr(o.total)}</div></article>`).join('')
    : '<p class="empty">No orders yet. <a href="#/">Start shopping</a></p>';
  return { el: h(`<div data-testid="orders"><h1>Your orders</h1><div class="stack">${body}</div></div>`) };
}
