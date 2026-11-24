import './styles.css';
import './shell.css';
import './home.css';
import { h, CATEGORIES, cap, cart, byId } from './lib.js';
import { home } from './pages/home.js';
import { category } from './pages/category.js';
import { product } from './pages/product.js';
import { search } from './pages/search.js';
import { cartPage } from './pages/cart.js';
import { checkout } from './pages/checkout.js';
import { orders as ordersPage } from './pages/orders.js';
import { artSheet } from './pages/art-sheet.js';

// Shell regions (promo bar, header, nav) render ONCE. Only the cart badge text is patched.
const app = document.getElementById('app');
export const MARK = `<svg class="mark" viewBox="0 0 36 30" aria-hidden="true"><path d="M2 5 Q18 13 34 5" fill="none" stroke="#fff" stroke-width="1.6" stroke-linecap="round"/><path d="M3.6 5.6 L12.4 8.6 L7.4 20.5z" fill="#fff"/><path d="M13.6 8.6 L22.4 8.6 L18 24z" fill="#ff5c93"/><path d="M23.6 8.6 L32.4 5.6 L28.6 20.5z" fill="#f4b02a"/></svg>`;
const WORDMARK = `${MARK}<span>Big<i> Market</i></span>`;
const CART_ICON = `<svg viewBox="0 0 26 26" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M2 3h3.5l2.6 12.2h11.6L22 7H6.4"/><circle cx="9.5" cy="20.5" r="1.6"/><circle cx="18.5" cy="20.5" r="1.6"/></svg>`;
app.innerHTML = `
  <div class="promo" data-slot="promo-bar">Free delivery on orders above ₹499. Cash on delivery and UPI accepted.</div>
  <header class="top">
    <a class="logo" href="#/" aria-label="Big Market home">${WORDMARK}</a>
    <div class="loc"><span>Deliver to</span><b>Bengaluru 560001</b></div>
    <form class="search" role="search">
      <select name="cat" aria-label="Search in category"><option value="">All</option>${CATEGORIES.map((c) => `<option value="${c}">${cap(c)}</option>`).join('')}</select>
      <input type="search" name="q" placeholder="Search Big Market" aria-label="Search" autocomplete="off" />
      <button>Search</button>
    </form>
    <a class="toplink" href="#/orders"><small>Your account</small>Orders</a>
    <a class="cartlink" href="#/cart" data-testid="cart-link" aria-label="Cart">${CART_ICON}<b data-testid="cart-count">0</b><span>Cart</span></a>
  </header>
  <div class="bunting bunting-bar" aria-hidden="true"></div>
  <nav class="cats" aria-label="Categories"><a class="deals" href="#/?deals">Today's deals</a>${CATEGORIES.map((c) => `<a href="#/c/${c}">${cap(c)}</a>`).join('')}</nav>
  <main id="view"></main>
  <footer class="site">
    <a class="f-top" href="#/" onclick="window.scrollTo({top:0});">Back to top</a>
    <div class="f-cols">
      <div class="f-brand">
        <a class="logo" href="#/">${WORDMARK}</a>
        <p>Everyday things from kitchen to bookshelf, priced fairly and delivered to your door.</p>
        <div class="apps"><span><small>Get the app on</small>Android</span><span><small>Get the app on</small>iPhone</span></div>
      </div>
      <div><h4>Shop</h4><ul>${CATEGORIES.map((c) => `<li><a href="#/c/${c}">${cap(c)}</a></li>`).join('')}</ul></div>
      <div><h4>Your account</h4><ul><li><a href="#/orders">Your orders</a></li><li><a href="#/cart">Cart</a></li><li><a href="#/checkout">Checkout</a></li><li><a href="#/?deals">Today's deals</a></li></ul></div>
      <div><h4>How it works</h4><ul><li>Free delivery over ₹499</li><li>7-day easy returns</li><li>Cash on delivery</li><li>UPI, cards and net banking</li></ul></div>
    </div>
    <div class="f-base"><div><span>Pay with UPI, debit and credit cards, net banking or cash on delivery.</span><span>Big Market is a demo store. No real orders are placed.</span></div></div>
  </footer>
  <div class="toast" id="toast" role="status"></div>`;
const view = document.getElementById('view');
const badge = app.querySelector('[data-testid=cart-count]');
const toast = document.getElementById('toast');
let toastT;
function say(msg) { toast.textContent = msg; toast.classList.add('on'); clearTimeout(toastT); toastT = setTimeout(() => toast.classList.remove('on'), 1400); }
const syncBadge = () => { badge.textContent = cart.count(); };
syncBadge();
window.addEventListener('haat:cart', syncBadge);

app.querySelector('form.search').addEventListener('submit', (e) => {
  e.preventDefault();
  const fd = new FormData(e.target);
  const q = fd.get('q').toString().trim(), cat = fd.get('cat').toString();
  location.hash = !q && cat ? '#/c/' + cat : '#/search?q=' + encodeURIComponent(q);
});

// Add-to-cart anywhere: update cart storage + badge + toast. No region re-renders.
document.addEventListener('click', (e) => {
  const b = e.target.closest('[data-add]');
  if (!b) return;
  cart.add(b.dataset.add);
  say('Added: ' + byId.get(b.dataset.add).title);
});

const routes = [
  [/^\/?$/, (m, qs) => home(qs)],
  [/^\/orders$/, () => ordersPage()],
  [/^\/c\/(\w+)$/, (m, qs) => category(m[1], qs)],
  [/^\/product\/(\w+)$/, (m) => product(m[1])],
  [/^\/search$/, (m, qs) => search(qs.get('q') || '')],
  [/^\/cart$/, () => cartPage()],
  [/^\/checkout$/, () => checkout()],
  [/^\/art-sheet$/, (m, qs) => artSheet(qs)], // dev route: every product's art in one grid
];
let cleanup = null;
function render() {
  const [path, query = ''] = (location.hash.slice(1) || '/').split('?');
  const qs = new URLSearchParams(query);
  if (cleanup) { cleanup(); cleanup = null; }
  for (const [re, fn] of routes) {
    const m = path.match(re);
    if (m) {
      const out = fn(m, qs);
      view.replaceChildren(out.el);
      cleanup = out.cleanup || null;
      window.scrollTo(0, 0);
      return;
    }
  }
  view.replaceChildren(h('<p class="empty" data-testid="not-found">Page not found. <a href="#/">Back home</a></p>'));
}
window.addEventListener('hashchange', render);
render();
