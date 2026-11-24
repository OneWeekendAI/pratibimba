import { h, products, CATEGORIES, cap, card } from '../lib.js';

const SORTS = { popular: 'Popular', 'price-low': 'Price: low to high', rating: 'Top rated' };
const sorters = {
  popular: (a, b) => b.reviews - a.reviews,
  'price-low': (a, b) => a.price - b.price,
  rating: (a, b) => b.rating - a.rating,
};

export function category(cat, qs) {
  if (!CATEGORIES.includes(cat)) return { el: h('<p class="empty" data-testid="not-found">Unknown category.</p>') };
  const all = products.filter((p) => p.category === cat);
  const max = Math.max(...all.map((p) => p.price));
  const el = h(`<div>
    <div class="crumbs"><a href="#/">Home</a> / ${cap(cat)}</div>
    <h1 data-testid="cat-title">${cap(cat)}</h1>
    <div class="listing">
      <aside class="filters">
        <h3>Max price</h3><input type="range" id="f-price" min="0" max="${max}" value="${max}" step="10" /><div id="f-price-v"></div>
        <h3>Rating</h3><label><input type="checkbox" id="f-rating" /> 4 stars and up</label>
        <h3>Tags</h3>
        ${['budget', 'premium', 'gift', 'new'].map((t) => `<label><input type="checkbox" data-tag="${t}" /> ${cap(t)}</label>`).join('')}
      </aside>
      <div>
        <div class="bar"><span id="count" data-testid="count"></span>
          <label>Sort <select id="sort" data-slot="listing-sort" data-testid="sort">${Object.entries(SORTS).map(([k, v]) => `<option value="${k}">${v}</option>`).join('')}</select></label></div>
        <div class="grid" id="grid" data-testid="grid"></div>
      </div>
    </div>
  </div>`);
  const $ = (s) => el.querySelector(s);
  const sort = $('#sort');
  sort.value = SORTS[qs.get('sort')] ? qs.get('sort') : 'popular'; // default sort: a slot later
  // Only the grid re-renders on filter/sort changes; the controls stay put.
  function paint() {
    const maxP = +$('#f-price').value;
    const tags = [...el.querySelectorAll('[data-tag]:checked')].map((i) => i.dataset.tag);
    const list = all
      .filter((p) => p.price <= maxP && (!$('#f-rating').checked || p.rating >= 4) && tags.every((t) => p.tags.includes(t)))
      .sort(sorters[sort.value]);
    $('#f-price-v').textContent = 'Up to ₹' + maxP.toLocaleString('en-IN');
    $('#count').textContent = `${list.length} products`;
    $('#grid').innerHTML = list.length ? list.map(card).join('') : '<p class="empty">Nothing matches these filters.</p>';
  }
  el.addEventListener('input', paint);
  el.addEventListener('change', paint);
  paint();
  return { el };
}
