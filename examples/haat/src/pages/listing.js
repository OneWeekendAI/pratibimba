import { h, cap, card, esc } from '../lib.js';

export const brandOf = (p) => p.title.split(' ')[0];

const SORTS = { popular: 'Popular', 'price-low': 'Price: low to high', 'price-high': 'Price: high to low', rating: 'Top rated', newest: 'Newest' };
const sorters = {
  popular: (a, b) => b.reviews - a.reviews,
  'price-low': (a, b) => a.price - b.price,
  'price-high': (a, b) => b.price - a.price,
  rating: (a, b) => b.rating - a.rating || b.reviews - a.reviews,
  newest: (a, b) => (b.tags.includes('new') - a.tags.includes('new')) || b.id.localeCompare(a.id),
};
const PRICES = [
  ['any', 'Any price', 0, Infinity],
  ['u500', 'Under ₹500', 0, 499],
  ['500', '₹500 to ₹999', 500, 999],
  ['1000', '₹1,000 to ₹2,499', 1000, 2499],
  ['2500', '₹2,500 to ₹4,999', 2500, 4999],
  ['5000', '₹5,000 and above', 5000, Infinity],
];
const RATINGS = [['0', 'Any rating', 0], ['4', '4 stars and up', 4], ['3', '3 stars and up', 3]];
const TAGS = ['budget', 'premium', 'gift', 'new'];
const TAG_LABEL = { budget: 'Budget pick', premium: 'Premium', gift: 'Great for gifting', new: 'New' };

/**
 * Shared listing (category + search). Only #grid, count and chips repaint on a filter change;
 * the sidebar and the bar stay put.
 * opts: { crumbs (html), heading (html), headingTestid, countTestid, all, sort, emptyHtml }
 */
export function listing(opts) {
  const { all } = opts;
  const brands = [...all.reduce((m, p) => m.set(brandOf(p), (m.get(brandOf(p)) || 0) + 1), new Map())].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  const tagCount = (t) => all.filter((p) => p.tags.includes(t)).length;
  const el = h(`<div class="lst">
    <div class="crumbs">${opts.crumbs}</div>
    <div class="lst-head">
      <h1 data-testid="${opts.headingTestid}">${opts.heading}</h1>
      <span class="lst-count" id="count" data-testid="${opts.countTestid}"></span>
    </div>
    <div class="listing">
      <div class="scrim" data-close-filters></div>
      <aside class="filters" id="filters" aria-label="Filters" data-testid="filters">
        <div class="f-head"><h2>Filters</h2><button type="button" class="link" data-clear>Clear all</button><button type="button" class="x" data-close-filters aria-label="Close filters">&times;</button></div>
        <fieldset><legend>Price</legend>${PRICES.map(([k, l], i) => `<label><input type="radio" name="price" value="${k}" ${i ? '' : 'checked'} /> ${l}</label>`).join('')}</fieldset>
        <fieldset><legend>Customer rating</legend>${RATINGS.map(([k, l], i) => `<label><input type="radio" name="rating" value="${k}" ${i ? '' : 'checked'} /> ${l}</label>`).join('')}</fieldset>
        <fieldset><legend>Brand</legend>${brands.map(([b, n]) => `<label><input type="checkbox" data-brand="${esc(b)}" /> ${esc(b)} <span>${n}</span></label>`).join('')}</fieldset>
        <fieldset><legend>Good to know</legend>${TAGS.filter(tagCount).map((t) => `<label><input type="checkbox" data-tag="${t}" /> ${TAG_LABEL[t]} <span>${tagCount(t)}</span></label>`).join('')}</fieldset>
        <fieldset><legend>Availability</legend><label><input type="checkbox" id="f-stock" data-testid="f-stock" /> In stock only</label><label><input type="checkbox" id="f-low" /> Almost gone (4 or fewer)</label></fieldset>
        <div class="f-foot"><button type="button" class="btn block" data-close-filters id="f-show"></button></div>
      </aside>
      <div class="lst-main">
        <div class="bar">
          <button type="button" class="btn ghost small filter-toggle" data-testid="filter-toggle" aria-controls="filters" aria-expanded="false">Filters<b id="f-n"></b></button>
          <span class="sortpref" data-prb-slot="listing-sort" aria-hidden="true"><span data-prb-field="sort" hidden>popular</span></span>
          <label class="sortbox">Sort by <select id="sort" data-slot="listing-sort" data-testid="sort">${Object.entries(SORTS).map(([k, v]) => `<option value="${k}">${v}</option>`).join('')}</select></label>
        </div>
        <div class="chips" id="chips"></div>
        <div class="grid" id="grid" data-testid="grid"></div>
      </div>
    </div>
  </div>`);
  const $ = (s) => el.querySelector(s);
  const sort = $('#sort');
  sort.value = SORTS[opts.sort] ? opts.sort : 'popular';
  // Pratibimba slot "listing-sort": a decided default sort. It only ever sets the control; a sort the
  // visitor picked, or one in the URL, always wins. The field is static markup, never re-rendered.
  let userSorted = !!SORTS[opts.sort];
  sort.addEventListener('change', () => { userSorted = true; });
  const pref = el.querySelector('.sortpref');
  new MutationObserver(() => {
    const v = pref.querySelector('[data-prb-field=sort]')?.textContent.trim();
    if (!userSorted && SORTS[v] && sort.value !== v) { sort.value = v; paint(); }
  }).observe(pref, { childList: true, subtree: true });

  const state = () => {
    const pr = PRICES.find((x) => x[0] === el.querySelector('[name=price]:checked').value);
    return {
      pr, rating: +el.querySelector('[name=rating]:checked').value,
      brands: [...el.querySelectorAll('[data-brand]:checked')].map((i) => i.dataset.brand),
      tags: [...el.querySelectorAll('[data-tag]:checked')].map((i) => i.dataset.tag),
      stock: $('#f-stock').checked, low: $('#f-low').checked,
    };
  };
  function paint() {
    const s = state();
    const list = all.filter((p) => p.price >= s.pr[2] && p.price <= s.pr[3] && p.rating >= s.rating
      && (!s.brands.length || s.brands.includes(brandOf(p))) && s.tags.every((t) => p.tags.includes(t))
      && (!s.stock || p.stock > 0) && (!s.low || p.stock <= 4)).sort(sorters[sort.value]);
    const chips = [];
    if (s.pr[0] !== 'any') chips.push(['price', s.pr[1]]);
    if (s.rating) chips.push(['rating', `${s.rating} stars and up`]);
    s.brands.forEach((b) => chips.push(['brand:' + b, b]));
    s.tags.forEach((t) => chips.push(['tag:' + t, TAG_LABEL[t]]));
    if (s.stock) chips.push(['stock', 'In stock']);
    if (s.low) chips.push(['low', 'Almost gone']);
    $('#count').textContent = `${list.length} ${list.length === 1 ? 'product' : 'products'}`;
    $('#f-show').textContent = `Show ${list.length} ${list.length === 1 ? 'result' : 'results'}`;
    $('#f-n').textContent = chips.length ? ` (${chips.length})` : '';
    $('#chips').innerHTML = chips.map(([k, l]) => `<button type="button" class="chip" data-chip="${esc(k)}">${esc(l)} <i aria-hidden="true">&times;</i></button>`).join('');
    $('#grid').innerHTML = list.length ? list.map(card).join('') : (opts.emptyHtml || '<p class="empty">Nothing matches these filters.</p>');
  }
  const setOpen = (on) => { el.classList.toggle('f-open', on); $('.filter-toggle').setAttribute('aria-expanded', on); document.body.classList.toggle('no-scroll', on); };
  el.addEventListener('input', paint);
  el.addEventListener('change', paint);
  el.addEventListener('click', (e) => {
    const t = e.target;
    if (t.closest('.filter-toggle')) return setOpen(true);
    if (t.closest('[data-close-filters]')) return setOpen(false);
    if (t.closest('[data-clear]')) {
      el.querySelectorAll('[data-brand],[data-tag],#f-stock,#f-low').forEach((i) => { i.checked = false; });
      el.querySelector('[name=price][value=any]').checked = true;
      el.querySelector('[name=rating][value="0"]').checked = true;
      return paint();
    }
    const c = t.closest('[data-chip]');
    if (c) {
      const [k, v] = c.dataset.chip.split(':');
      if (k === 'price') el.querySelector('[name=price][value=any]').checked = true;
      else if (k === 'rating') el.querySelector('[name=rating][value="0"]').checked = true;
      else if (k === 'brand') el.querySelector(`[data-brand="${CSS.escape(v)}"]`).checked = false;
      else if (k === 'tag') el.querySelector(`[data-tag="${v}"]`).checked = false;
      else if (k === 'stock') $('#f-stock').checked = false;
      else if (k === 'low') $('#f-low').checked = false;
      paint();
    }
  });
  paint();
  return { el, cleanup: () => document.body.classList.remove('no-scroll') };
}
