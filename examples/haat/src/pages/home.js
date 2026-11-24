import { h, products, CATEGORIES, cap, strip, off } from '../lib.js';

// Each region is rendered once with a stable data-slot container.
export function home() {
  const picks = [...products].sort((a, b) => b.rating * Math.log(b.reviews + 2) - a.rating * Math.log(a.reviews + 2)).slice(0, 10);
  const deals = [...products].sort((a, b) => off(b) - off(a)).slice(0, 10);
  const rows = CATEGORIES.map((c) => {
    const list = products.filter((p) => p.category === c).slice(0, 8);
    return strip(`${cap(c)} <a href="#/c/${c}">See all</a>`, list, `data-slot="category-row" data-category="${c}"`);
  }).join('');
  const el = h(`<div>
    <section class="hero" data-slot="hero" data-testid="hero">
      <h1>Deals of the day</h1>
      <p>Everyday things from kitchen to bookshelf, priced fairly and delivered to your door.</p>
      <a class="btn" href="#/c/kitchen">Shop kitchen</a>
    </section>
    <div data-slot="picks-strip" data-testid="picks">${strip('Picked for you', picks)}</div>
    <div data-slot="deals-strip" data-testid="deals">${strip('Deals', deals)}</div>
    <div data-slot="category-rows" data-testid="category-rows">${rows}</div>
  </div>`);
  return { el };
}
