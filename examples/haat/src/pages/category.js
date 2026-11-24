import { h, products, CATEGORIES, cap } from '../lib.js';
import { listing } from './listing.js';

export function category(cat, qs) {
  if (!CATEGORIES.includes(cat)) return { el: h('<p class="empty" data-testid="not-found">Unknown category.</p>') };
  return listing({
    crumbs: `<a href="#/">Home</a> / ${cap(cat)}`,
    heading: cap(cat), headingTestid: 'cat-title', countTestid: 'count',
    all: products.filter((p) => p.category === cat), sort: qs.get('sort'),
  });
}
