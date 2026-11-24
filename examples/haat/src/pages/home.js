import { h, products, CATEGORIES, cap, card, off, inr, stars, art, esc } from '../lib.js';

const score = (p) => p.rating * Math.log(p.reviews + 2);
const topOf = (c) => products.filter((p) => p.category === c).sort((a, b) => score(b) - score(a));
const minPrice = (c) => Math.min(...products.filter((p) => p.category === c).map((p) => p.price));

const ICONS = {
  truck: '<svg viewBox="0 0 32 32"><path d="M3 8h16v14H3zM19 12h6l4 5v5H19z"/><circle cx="9" cy="23" r="2.4"/><circle cx="24" cy="23" r="2.4"/></svg>',
  back: '<svg viewBox="0 0 32 32"><path d="M8 12h12a6 6 0 0 1 0 12H10"/><path d="M12 7l-5 5 5 5"/></svg>',
  cash: '<svg viewBox="0 0 32 32"><rect x="3" y="8" width="26" height="16" rx="3"/><circle cx="16" cy="16" r="3.6"/><path d="M7 12v0M25 20v0"/></svg>',
  upi: '<svg viewBox="0 0 32 32"><rect x="9" y="3" width="14" height="26" rx="3"/><path d="M13 8h6M14 24h4"/><path d="M13 16l3 3 4-6"/></svg>',
};

function sec(title, link, body, attrs = '') {
  return `<section class="sec" ${attrs}><div class="sec-head"><h2>${title}</h2>${link || ''}</div>${body}</section>`;
}
const row = (list, cardFn = card) => `<div class="row">${list.map(cardFn).join('')}</div>`;
const dealCard = (p) => card(p).replace(/(<a class="card-img"[^>]*>)/, `$1<span class="pill-off">${off(p)}% off</span>`);

function msToClock(ms) {
  const s = Math.max(0, Math.floor(ms / 1000));
  const f = (n) => String(n).padStart(2, '0');
  return `${f(Math.floor(s / 3600))}:${f(Math.floor((s % 3600) / 60))}:${f(s % 60)}`;
}
function endOfDay() { const d = new Date(); d.setHours(24, 0, 0, 0); return d.getTime(); }

// Each region is rendered once with a stable data-slot container. Only the countdown text ticks.
export function home(qs) {
  const picks = [...products].sort((a, b) => score(b) - score(a)).slice(0, 12);
  const deals = [...products].sort((a, b) => off(b) - off(a) || b.reviews - a.reviews).slice(0, 12);
  const best = [...products].filter((p) => p.reviews > 2500 && p.rating >= 4.2).sort((a, b) => b.reviews - a.reviews).slice(0, 10);
  const collage = ['electronics', 'kitchen', 'fashion'].map((c) => topOf(c)[0]);
  const side = ['books', 'home', 'toys'].map((c) => [c, topOf(c)[1]]);

  const rows = CATEGORIES.map((c) => sec(cap(c), `<a href="#/c/${c}">See all</a>`, row(products.filter((p) => p.category === c).slice(0, 10)), `data-slot="category-row" data-category="${c}"`)).join('');
  const tiles = CATEGORIES.map((c) => `<a class="cat-tile" href="#/c/${c}"><span class="tile-art">${art(topOf(c)[2])}</span><div><h3>${cap(c)}</h3><span>${products.filter((p) => p.category === c).length} items</span></div></a>`).join('');
  const bestList = best.map((p, i) => `<a class="best" href="#/product/${p.id}"><span class="rank">${i + 1}</span>${art(p)}<div><div class="t">${esc(p.title)}</div><div class="m">${stars(p.rating)}<span>(${p.reviews.toLocaleString('en-IN')})</span></div><div class="m"><b>${inr(p.price)}</b><s>${inr(p.mrp)}</s></div></div></a>`).join('');

  const el = h(`<div>
    <div class="hero" data-slot="hero" data-testid="hero">
      <div class="hero-main">
        <div class="hero-copy">
          <span class="kicker">Festive season picks</span>
          <h1>Everything for your home, at market prices</h1>
          <p>Kitchen, books, fashion and more from sellers across India. Free delivery over ₹499, pay on delivery if you like.</p>
          <a class="btn" href="#/?deals">See today's deals</a><a class="btn ghost" href="#/c/kitchen">Shop kitchen</a>
        </div>
        <div class="collage" aria-hidden="true">${collage.map((p) => `<a href="#/product/${p.id}" tabindex="-1">${art(p)}</a>`).join('')}</div>
      </div>
      <div class="hero-side">${side.map(([c, p]) => `<a class="side-tile" href="#/c/${c}"><div><h3>${cap(c)}</h3><span>From <b>${inr(minPrice(c))}</b></span></div>${art(p)}</a>`).join('')}</div>
    </div>
    <div class="trust" data-slot="trust-strip">
      <div>${ICONS.truck}<p style="margin:0"><b>Free delivery</b><span>On orders over ₹499</span></p></div>
      <div>${ICONS.back}<p style="margin:0"><b>7-day returns</b><span>No questions asked</span></p></div>
      <div>${ICONS.cash}<p style="margin:0"><b>Cash on delivery</b><span>Pay when it arrives</span></p></div>
      <div>${ICONS.upi}<p style="margin:0"><b>UPI and cards</b><span>Safe, instant payment</span></p></div>
    </div>
    <div data-slot="deals-strip" data-testid="deals" id="deals">${sec("Today's deals", `<span class="countdown">Ends in <b data-testid="countdown">--:--:--</b></span>`, row(deals, dealCard)).replace('class="sec"', 'class="sec deals-wrap"')}</div>
    ${sec('Shop by category', '', `<div class="cat-tiles">${tiles}</div>`)}
    <div data-slot="picks-strip" data-testid="picks">${sec('Picked for you', '', row(picks)).replace('class="sec"', 'class="sec sec-panel"')}</div>
    <div data-slot="category-rows" data-testid="category-rows">${rows}</div>
    ${sec('Bestsellers', '', `<div class="best-list">${bestList}</div>`, 'data-slot="bestsellers"').replace('class="sec"', 'class="sec sec-panel"')}
  </div>`);

  const clock = el.querySelector('[data-testid=countdown]');
  const end = endOfDay();
  const tick = () => { clock.textContent = msToClock(end - Date.now()); };
  tick();
  const timer = setInterval(tick, 1000);
  if (qs && qs.has('deals')) setTimeout(() => el.querySelector('#deals')?.scrollIntoView({ block: 'start' }), 0);
  return { el, cleanup: () => clearInterval(timer) };
}
