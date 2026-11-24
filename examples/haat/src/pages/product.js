import { h, byId, art, inr, off, esc, cap, strip, products, stars, cart, prbTrack } from '../lib.js';
import { brandOf } from './listing.js';

const rng = (seed) => { let a = seed >>> 0; return () => { a = (a + 0x6D2B79F5) >>> 0; let t = a; t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; }; };
const pick = (r, a) => a[Math.floor(r() * a.length)];

function badgeText(p) {
  if (p.stock <= 4) return `Only ${p.stock} left`;
  if (p.tags.includes('gift')) return 'Great for gifting';
  return p.rating >= 4.4 ? 'Bestseller' : 'Popular choice';
}
const PIN_DEFAULT = '560001';
// The delivery line is a template: {fee} {day} {pin}. Pratibimba's copy variants carry the same tokens,
// and the page fills them (see paintDelivery), so the day and PIN stay the page's own facts.
const DELIVERY_TPL = '{fee} by {day} to {pin}';
function deliveryText(p, pin, tpl = DELIVERY_TPL) {
  const days = 2 + (+pin[0] % 4) + (p.stock <= 4 ? 1 : 0);
  const d = new Date(Date.now() + days * 864e5).toLocaleDateString('en-IN', { weekday: 'long', day: 'numeric', month: 'short' });
  const fee = p.price >= 499 ? 'Free delivery' : 'Standard delivery for ₹40';
  return tpl.replace(/\{(fee|day|pin)\}/g, (_, k) => ({ fee, day: d, pin })[k]);
}
const HIGH = {
  electronics: ['Designed for everyday use with a one-year brand warranty', 'Fast, reliable performance you can feel from the first day', 'Tested for Indian power conditions and humid weather', 'Comes with the cable or accessories you need in the box', 'Easy setup, no app or account needed'],
  kitchen: ['Food-safe material that is easy to clean and dry', 'Even heat and a sturdy build for daily cooking', 'Sized for a family of four', 'Works on gas and induction where applicable', 'Backed by a 7-day return window'],
  books: ['Clear print on good paper that is easy on the eyes', 'A comfortable read for a weekend or a commute', 'Written for a general reader, no background needed', 'Makes a thoughtful gift', 'Packed carefully so corners arrive intact'],
  fashion: ['Soft, breathable fabric for all-day comfort', 'True-to-size fit with room to move', 'Colour holds up through repeated washes', 'Easy returns and size exchange within 7 days', 'Neatly stitched seams and finished edges'],
  home: ['Adds warmth to a living room, bedroom or entrance', 'Sturdy build that keeps its shape over time', 'Simple to set up, no tools needed', 'Colours chosen to sit well with most walls', 'Ships in protective packaging'],
  toys: ['Child-safe materials with rounded edges', 'Encourages play, imagination and learning', 'Suitable for ages 3 and above', 'Easy to wipe clean', 'A cheerful gift for birthdays and festivals'],
  grocery: ['Sourced from trusted growers and packed fresh', 'Sealed packing to keep it dry and aromatic', 'No added colours or preservatives', 'Check the pack for best-before date', 'Good value in a family-size pack'],
  beauty: ['Gentle on skin and suitable for daily use', 'Dermatologically tested, patch test advised', 'Light fragrance that does not overpower', 'Made with ingredients familiar from home remedies', 'Cruelty-free and sealed for freshness'],
};
const SPECS = {
  electronics: [['Warranty', '1 year from the brand'], ['In the box', 'Product, cable, quick guide'], ['Power', 'Rechargeable, USB-C'], ['Connectivity', 'Bluetooth 5.3 or wired'], ['Colour', null]],
  kitchen: [['Material', 'Food-grade steel and alloy'], ['Capacity', '1.5 litre'], ['Care', 'Hand wash recommended'], ['Suitable for', 'Gas and induction'], ['Colour', null]],
  books: [['Language', 'English'], ['Binding', 'Paperback'], ['Pages', null], ['Publisher', 'Big Market Press'], ['Reading level', 'General']],
  fashion: [['Fabric', '100% cotton blend'], ['Fit', 'Regular'], ['Care', 'Machine wash cold'], ['Occasion', 'Everyday and festive'], ['Colour', null]],
  home: [['Material', 'Wood, metal and fabric'], ['Dimensions', '30 x 30 x 12 cm'], ['Assembly', 'Not required'], ['Care', 'Wipe with a dry cloth'], ['Colour', null]],
  toys: [['Age group', '3 years and above'], ['Material', 'Child-safe plastic and fabric'], ['Batteries', 'Not required'], ['Care', 'Wipe clean'], ['Colour', null]],
  grocery: [['Pack size', null], ['Shelf life', '9 months from packing'], ['Storage', 'Cool, dry place'], ['Diet', 'Vegetarian'], ['FSSAI licensed', 'Yes']],
  beauty: [['Skin type', 'All skin types'], ['Pack size', null], ['Shelf life', '24 months from manufacture'], ['Free from', 'Parabens and sulphates'], ['Usage', 'Daily']],
};
const COLOURS = ['Peacock teal', 'Saffron', 'Ivory', 'Indigo', 'Rose', 'Charcoal', 'Forest green', 'Terracotta'];
const NAMES = ['Anjali R.', 'Rohan S.', 'Meera K.', 'Imran Q.', 'Divya P.', 'Karthik N.', 'Sneha M.', 'Arjun V.', 'Fatima Z.', 'Gurpreet S.', 'Lakshmi B.', 'Vikram T.'];
const PLACES = ['Pune', 'Chennai', 'Jaipur', 'Kolkata', 'Hyderabad', 'Lucknow', 'Kochi', 'Indore'];
const REV = {
  5: [['Worth every rupee', 'Exactly as described and arrived a day early. We have been using it daily and it still feels new.'], ['Very happy with this', 'Good quality for the price. The packing was neat and the finish is better than I expected.'], ['Bought a second one', 'Gave the first to my sister and she loved it, so I ordered another. Would buy again.']],
  4: [['Good, with small nitpicks', 'Does the job well and looks good. Delivery was quick. Took a star off for the slightly loose packaging.'], ['Solid choice', 'Matches the photos and works well. A bit more detail in the instructions would help.'], ['Nice for the price', 'Happy overall. Quality is fair for this price and it is easy to use.']],
  3: [['Average, does the basics', 'It is okay for the price. Nothing special, but nothing wrong either.'], ['Mixed feelings', 'Looks fine but the finish could be better. Fine for occasional use.']],
};

export function product(id) {
  const p = byId.get(id);
  if (!p) return { el: h('<p class="empty" data-testid="not-found">Product not found.</p>') };
  const r = rng(p.seed);
  const brand = brandOf(p);
  const sameCat = products.filter((x) => x.category === p.category && x.id !== p.id);
  const fbt = sameCat.filter((x) => x.price < p.price * 1.6 + 400).sort((a, b) => b.rating - a.rating || b.reviews - a.reviews).slice(0, 2);
  const fbtSet = [p, ...fbt];
  const viewed = sameCat.filter((x) => !fbt.includes(x)).sort((a, b) => b.reviews - a.reviews).slice(0, 8);
  const highlights = [...HIGH[p.category]].sort(() => r() - 0.5).slice(0, 4);
  const colour = pick(r, COLOURS);
  const specs = [['Brand', brand], ...SPECS[p.category].map(([k, v]) => [k, v ?? (k === 'Pages' ? 180 + Math.floor(r() * 25) * 8 + '' : k === 'Colour' ? colour : pick(r, ['250 g', '500 g', '1 kg', '100 ml', '200 ml']))]), ['Country of origin', 'India'], ['Item code', p.id.toUpperCase()]];

  // Histogram: weights follow the average rating, scaled to the review count.
  const mean = p.rating;
  const w = [5, 4, 3, 2, 1].map((s) => Math.exp(-Math.pow(s - (mean + 0.35), 2) / 1.1));
  const wsum = w.reduce((a, b) => a + b, 0);
  const pct = w.map((x) => Math.round((x / wsum) * 100));
  const reviewsFor = (() => {
    const tiers = mean >= 4.2 ? [5, 5, 4] : mean >= 3.7 ? [5, 4, 4] : [4, 3, 5];
    const used = {};
    return tiers.map((s, i) => {
      const [title, body] = REV[s][(used[s] = (used[s] ?? -1) + 1) % REV[s].length];
      return { s, title, body, who: NAMES[(Math.floor(r() * 4) + i * 4) % NAMES.length], where: pick(r, PLACES), days: 3 + Math.floor(r() * 80), helpful: Math.floor(r() * 120) + 4 - i };
    });
  })();

  const total = (ids) => ids.reduce((a, x) => a + byId.get(x).price, 0);
  const el = h(`<div class="pd">
    <div class="crumbs"><a href="#/">Home</a> / <a href="#/c/${p.category}">${cap(p.category)}</a> / ${esc(p.title)}</div>
    <div class="pdp">
      <div class="gallery" data-testid="gallery">
        <div class="thumbs" role="tablist" aria-label="Product images">${[0, 1, 2, 3].map((v) => `<button type="button" class="thumb${v ? '' : ' on'}" role="tab" aria-selected="${!v}" data-v="${v}" aria-label="Image ${v + 1}" data-testid="thumb">${art(p, v)}</button>`).join('')}</div>
        <div class="main-img" data-testid="main-image" data-v="0">${art(p, 0)}</div>
      </div>
      <div class="pinfo">
        <span class="badge" data-slot="product-badge" data-prb-slot="product-badge" data-testid="badge"><span data-prb-field="text">${badgeText(p)}</span></span>
        <h1 data-testid="title">${esc(p.title)}</h1>
        <div class="p-meta"><a class="brandlink" href="#/search?q=${encodeURIComponent(brand)}">Visit the ${esc(brand)} store</a>
          <button type="button" class="rate-link" data-goto="reviews">${stars(p.rating)} <b>${p.rating.toFixed(1)}</b> <span>${p.reviews.toLocaleString('en-IN')} ratings</span></button></div>
        <div class="p-price">
          <div><em class="pill-off">${off(p)}% off</em> <span class="big" data-testid="price">${inr(p.price)}</span></div>
          <div class="muted">M.R.P. <s>${inr(p.mrp)}</s> <span>Inclusive of all taxes</span></div>
          <div class="emi">EMI from ${inr(Math.max(99, Math.ceil(p.price / 6 / 10) * 10))} a month for 6 months. Cash on delivery available.</div>
        </div>
        <h2 class="h-sm">About this item</h2>
        <ul class="highlights" data-testid="highlights">${highlights.map((x) => `<li>${x}</li>`).join('')}</ul>
      </div>
      <aside class="buybox" data-testid="buybox" data-qtyscope>
        <div class="bb-price">${inr(p.price)}</div>
        <div class="delivery" data-slot="delivery-message" data-prb-slot="delivery-message" data-testid="delivery"><span data-prb-field="line">${deliveryText(p, PIN_DEFAULT)}</span></div>
        <form class="pin" novalidate><label>Delivery PIN code <input type="text" inputmode="numeric" maxlength="6" value="${PIN_DEFAULT}" data-testid="pin" aria-label="PIN code" /></label><button class="btn ghost small" data-testid="pin-check">Check</button></form>
        <div class="pin-err" data-testid="pin-err" role="status"></div>
        <div class="stock ${p.stock <= 4 ? 'low' : ''}" data-testid="stock">${p.stock <= 4 ? `Only ${p.stock} left in stock, order soon` : 'In stock'}</div>
        <label class="qtyrow">Quantity <select data-qty data-testid="qty">${Array.from({ length: Math.min(10, p.stock) }, (_, i) => `<option>${i + 1}</option>`).join('')}</select></label>
        <button class="btn block" data-add="${p.id}" data-slot="buy-button" data-prb-slot="buy-button" data-testid="buy"><span data-prb-field="label">Add to cart</span></button>
        <button class="btn deal block" data-buy-now="${p.id}" data-testid="buy-now">Buy now</button>
        <ul class="assure"><li>7-day easy returns</li><li>Pay on delivery or by UPI</li><li>Sold and shipped by Big Market</li></ul>
      </aside>
    </div>

    <section class="fbt" data-testid="fbt" aria-labelledby="fbt-h">
      <h2 id="fbt-h" class="sec-title">Frequently bought together</h2>
      <div class="fbt-body">
        <div class="fbt-items">${fbtSet.map((x, i) => `${i ? '<span class="plus" aria-hidden="true">+</span>' : ''}<label class="fbt-item"><a href="#/product/${x.id}">${art(x)}</a><span class="fbt-t"><input type="checkbox" data-fbt="${x.id}" checked ${i ? '' : 'disabled'} /> ${i ? '' : '<b>This item: </b>'}${esc(x.title)}</span><b>${inr(x.price)}</b></label>`).join('')}</div>
        <div class="fbt-sum"><div class="muted"><span id="fbt-n">${fbtSet.length}</span> items</div><div class="big" id="fbt-total" data-testid="fbt-total">${inr(total(fbtSet.map((x) => x.id)))}</div><button class="btn block" data-testid="fbt-add" id="fbt-add">Add all ${fbtSet.length} to cart</button></div>
      </div>
    </section>

    <section class="specs" data-testid="specs"><h2 class="sec-title">Product details</h2>
      <table><tbody>${specs.map(([k, v]) => `<tr><th scope="row">${k}</th><td>${esc(v)}</td></tr>`).join('')}</tbody></table></section>

    <section class="reviews" id="reviews" data-testid="reviews"><h2 class="sec-title">Customer reviews</h2>
      <div class="rv-grid">
        <div class="rv-sum"><div class="rv-avg"><b>${p.rating.toFixed(1)}</b><span>out of 5</span></div>${stars(p.rating)}<div class="muted">${p.reviews.toLocaleString('en-IN')} global ratings</div>
          <div class="hist" data-testid="histogram">${[5, 4, 3, 2, 1].map((s, i) => `<div class="hrow"><span>${s} star</span><i><u style="width:${pct[i]}%"></u></i><span>${pct[i]}%</span></div>`).join('')}</div></div>
        <div class="rv-list">${reviewsFor.map((v) => `<article class="rv" data-testid="review"><div class="rv-who"><span class="av" aria-hidden="true">${v.who[0]}</span>${esc(v.who)}</div>
          <div class="rv-h">${stars(v.s)} <b>${esc(v.title)}</b></div><div class="muted t-sm">Reviewed in ${v.where} ${v.days} days ago. <span class="vp">Verified purchase</span></div>
          <p>${esc(v.body)}</p><div class="muted t-sm">${v.helpful} people found this helpful</div></article>`).join('')}</div>
      </div></section>
    <div data-testid="also-viewed">${strip('Customers also viewed', viewed)}</div>
  </div>`);

  const $ = (s) => el.querySelector(s);
  // Gallery: thumbs swap the main image.
  const main = $('.main-img');
  el.addEventListener('click', (e) => {
    const t = e.target.closest('.thumb');
    if (t) {
      el.querySelectorAll('.thumb').forEach((b) => { const on = b === t; b.classList.toggle('on', on); b.setAttribute('aria-selected', on); });
      main.innerHTML = art(p, +t.dataset.v); main.dataset.v = t.dataset.v;
      return;
    }
    if (e.target.closest('[data-goto=reviews]')) { $('#reviews').scrollIntoView({ behavior: 'smooth' }); return; }
    const bn = e.target.closest('[data-buy-now]');
    if (bn) { const q = +$('[data-qty]').value; cart.set(p.id, (cart.items()[p.id] || 0) + q); prbTrack.add(p.id, q); location.hash = '#/checkout'; }
  });
  // Delivery by PIN: only the delivery line and the error line change.
  let curPin = PIN_DEFAULT;
  const dline = $('[data-prb-field=line]');
  dline.dataset.tpl = DELIVERY_TPL;
  const paintDelivery = () => { dline.textContent = deliveryText(p, curPin, dline.dataset.tpl); };
  // When the SDK writes a decided variant, its text still holds the tokens: fill them before the next paint.
  new MutationObserver(() => {
    const t = dline.textContent;
    if (/\{(fee|day|pin)\}/.test(t)) { dline.dataset.tpl = t; paintDelivery(); }
  }).observe(dline, { childList: true, characterData: true, subtree: true });
  // Truth guard. Core's copy variants carry claims ("Only {n} left", "Bestseller", "ships {ship}") that are only
  // true for some products, and a copy slot has no `requires`. After the SDK writes a variant, the page fills its
  // tokens from this product's real data, or falls back to its own truthful text when the claim would be false.
  const guard = (field, fallback, fix) => {
    if (!field) return;
    const run = () => {
      const t = field.textContent, out = fix(t);
      if (out !== null && out !== t) field.textContent = out;
    };
    new MutationObserver(run).observe(field, { childList: true, characterData: true, subtree: true });
  };
  guard($('[data-slot=product-badge] [data-prb-field=text]'), null, (t) => {
    if (/^Only \{n\} left$/.test(t)) return p.stock <= 4 ? `Only ${p.stock} left` : badgeText(p);
    if (t === 'Bestseller' && !(p.rating >= 4.4)) return badgeText(p);
    if (t === 'Great for gifting' && !p.tags.includes('gift')) return badgeText(p);
    return null;
  });
  guard($('[data-slot=buy-button] [data-prb-field=label]'), null, (t) => (/\{ship\}/.test(t) ? t.replace('{ship}', p.stock > 4 ? 'today' : 'tomorrow') : null));
  const pinForm = $('form.pin');
  pinForm.addEventListener('submit', (e) => {
    e.preventDefault();
    const pin = $('[data-testid=pin]').value.trim();
    if (!/^[1-9]\d{5}$/.test(pin)) { $('[data-testid=pin-err]').textContent = 'Enter a valid 6-digit PIN code.'; return; }
    $('[data-testid=pin-err]').textContent = '';
    curPin = pin; paintDelivery();
  });
  // Frequently bought together: checkboxes drive total + button label; add-all is handled in main.js via data-add-many.
  const fbtAdd = $('#fbt-add');
  function fbtSync() {
    const ids = [...el.querySelectorAll('[data-fbt]:checked')].map((i) => i.dataset.fbt);
    $('#fbt-total').textContent = inr(total(ids));
    $('#fbt-n').textContent = ids.length;
    fbtAdd.dataset.addMany = ids.join(',');
    fbtAdd.textContent = `Add all ${ids.length} to cart`;
  }
  el.querySelectorAll('[data-fbt]').forEach((i) => i.addEventListener('change', fbtSync));
  fbtSync();
  return { el };
}
