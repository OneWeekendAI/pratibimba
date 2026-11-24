import './styles.css';
import { h, CATEGORIES, cap, cart, byId } from './lib.js';
import { home } from './pages/home.js';
import { category } from './pages/category.js';
import { product } from './pages/product.js';
import { search } from './pages/search.js';
import { cartPage } from './pages/cart.js';
import { checkout } from './pages/checkout.js';
import { artSheet } from './pages/art-sheet.js';

// Shell regions (promo bar, header, nav) render ONCE. Only the cart badge text is patched.
const app = document.getElementById('app');
app.innerHTML = `
  <div class="promo" data-slot="promo-bar">Free delivery on orders above ₹499. Cash on delivery and UPI accepted.</div>
  <header class="top">
    <a class="logo" href="#/"><svg class="mark" viewBox="0 0 28 24" aria-hidden="true"><path d="M2 2h24l-2 7H4z" fill="#fff"/><path d="M4 9a5 5 0 0 0 10 0zM14 9a5 5 0 0 0 10 0z" fill="#d02a68"/><path d="M6 14h16v8H6z" fill="#e59a0c" opacity=".0"/><path d="M4 9v0" /></svg>Big Market</a>
    <form class="search" role="search"><input type="search" name="q" placeholder="Search Big Market" aria-label="Search" /><button>Search</button></form>
    <a class="cartlink" href="#/cart" data-testid="cart-link">Cart <b data-testid="cart-count">0</b></a>
  </header>
  <nav class="cats">${CATEGORIES.map((c) => `<a href="#/c/${c}">${cap(c)}</a>`).join('')}</nav>
  <main id="view"></main>
  <footer>Big Market is a demo store. No real orders are placed.</footer>
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
  const q = new FormData(e.target).get('q').toString().trim();
  location.hash = '#/search?q=' + encodeURIComponent(q);
});

// Add-to-cart anywhere: update cart storage + badge + toast. No region re-renders.
document.addEventListener('click', (e) => {
  const b = e.target.closest('[data-add]');
  if (!b) return;
  cart.add(b.dataset.add);
  say('Added: ' + byId.get(b.dataset.add).title);
});

const routes = [
  [/^\/?$/, () => home()],
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
