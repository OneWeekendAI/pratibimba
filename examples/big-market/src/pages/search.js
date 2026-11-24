import { products, esc } from '../lib.js';
import { listing } from './listing.js';

export function search(q) {
  const terms = q.toLowerCase().split(/\s+/).filter(Boolean);
  const list = terms.length ? products.filter((p) => terms.every((t) => (p.title + ' ' + p.category + ' ' + p.tags.join(' ')).toLowerCase().includes(t))) : [];
  return listing({
    crumbs: `<a href="#/">Home</a> / Search`,
    heading: `Results for &ldquo;${esc(q)}&rdquo;`, headingTestid: 'search-title', countTestid: 'search-count',
    all: list,
    emptyHtml: '<p class="empty">No matches. Try &ldquo;speaker&rdquo; or &ldquo;tea&rdquo;, or loosen the filters.</p>',
  });
}
