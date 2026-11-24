import { h, byId, art, inr, off, esc, cap, strip, products } from '../lib.js';

function badgeText(p) {
  if (p.stock <= 4) return `Only ${p.stock} left`;
  if (p.tags.includes('gift')) return 'Great for gifting';
  return p.rating >= 4.4 ? 'Bestseller' : 'Popular choice';
}
function deliveryText() {
  const d = new Date(Date.now() + 3 * 864e5);
  return `Free delivery by ${d.toLocaleDateString('en-IN', { weekday: 'long', day: 'numeric', month: 'short' })}`;
}

export function product(id) {
  const p = byId.get(id);
  if (!p) return { el: h('<p class="empty" data-testid="not-found">Product not found.</p>') };
  const similar = products.filter((x) => x.category === p.category && x.id !== p.id).slice(0, 6);
  const el = h(`<div>
    <div class="crumbs"><a href="#/">Home</a> / <a href="#/c/${p.category}">${cap(p.category)}</a> / ${esc(p.title)}</div>
    <div class="pdp">
      <div>${art(p)}</div>
      <div>
        <span class="badge" data-slot="product-badge" data-testid="badge">${badgeText(p)}</span>
        <h1 data-testid="title">${esc(p.title)}</h1>
        <div class="rating">${p.rating.toFixed(1)} &#9733; <span>(${p.reviews.toLocaleString('en-IN')} reviews)</span></div>
        <p><span class="big" data-testid="price">${inr(p.price)}</span> <s>${inr(p.mrp)}</s> <em>${off(p)}% off</em></p>
        <div class="delivery" data-slot="delivery-message" data-testid="delivery">${deliveryText()}</div>
        <button class="btn" data-add="${p.id}" data-slot="buy-button" data-testid="buy">Add to cart</button>
        <p style="color:var(--mute)">${p.stock} in stock &middot; ${p.tags.map(cap).join(', ') || 'Everyday'}</p>
      </div>
    </div>
    ${strip('More in ' + cap(p.category), similar)}
  </div>`);
  return { el };
}
