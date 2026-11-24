import { h, products, card, esc } from '../lib.js';

export function search(q) {
  const terms = q.toLowerCase().split(/\s+/).filter(Boolean);
  const list = terms.length ? products.filter((p) => terms.every((t) => (p.title + ' ' + p.category + ' ' + p.tags.join(' ')).toLowerCase().includes(t))) : [];
  const el = h(`<div>
    <h1 data-testid="search-title">Results for &ldquo;${esc(q)}&rdquo;</h1>
    <p data-testid="search-count">${list.length} products</p>
    <div class="grid" data-testid="grid">${list.length ? list.map(card).join('') : '<p class="empty">No matches. Try &ldquo;speaker&rdquo; or &ldquo;tea&rdquo;.</p>'}</div>
  </div>`);
  return { el };
}
