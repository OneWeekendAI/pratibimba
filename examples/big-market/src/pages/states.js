import { h, CATEGORIES, cap } from '../lib.js';

const BASKET = `<svg viewBox="0 0 160 120" class="state-art" aria-hidden="true"><ellipse cx="80" cy="108" rx="52" ry="6" fill="#dfe5e2"/><path d="M30 52h100l-9 46a8 8 0 0 1-8 6H47a8 8 0 0 1-8-6z" fill="#0a6a61"/><path d="M30 52h100l-2.6 13H32.6z" fill="#064a44"/><path d="M52 52 66 22M108 52 94 22" stroke="#0a6a61" stroke-width="5" stroke-linecap="round" fill="none"/><path d="M62 72v22M80 72v22M98 72v22" stroke="#e1f1ee" stroke-width="4" stroke-linecap="round"/><path d="M20 20 40 14 34 32z" fill="#d02a68"/><path d="M128 12l18 8-12 14z" fill="#e59a0c"/></svg>`;
const LOST = `<svg viewBox="0 0 160 120" class="state-art" aria-hidden="true"><ellipse cx="80" cy="108" rx="52" ry="6" fill="#dfe5e2"/><path d="M14 30 Q80 54 146 30" fill="none" stroke="#064a44" stroke-width="2.4" stroke-linecap="round"/><path d="M22 33l22 7-10 30z" fill="#0a6a61"/><path d="M52 40l28 2-14 36z" fill="#d02a68"/><path d="M92 42l26-4-6 34z" fill="#e59a0c"/><text x="80" y="102" text-anchor="middle" font-family="Bricolage Grotesque Variable, sans-serif" font-weight="800" font-size="30" fill="#0a6a61">404</text></svg>`;

const chips = () => `<div class="state-chips">${CATEGORIES.map((c) => `<a href="#/c/${c}">${cap(c)}</a>`).join('')}</div>`;

export const emptyCart = (testid = 'cart-empty', text = 'Add a few things you like and they will wait for you here.') => h(`<div class="state" data-testid="${testid}">${BASKET}
  <h2>Your cart is empty</h2><p>${text}</p>
  <div class="state-cta"><a class="btn" href="#/">Keep browsing</a><a class="btn alt" href="#/?deals">See today's deals</a></div>${chips()}</div>`);

export const notFound = () => h(`<div class="state" data-testid="not-found">${LOST}
  <h2>We could not find that page</h2><p>The link may be old, or the page may have moved. Try a search or head to a category.</p>
  <form class="state-search" action="#/search"><input type="search" name="q" placeholder="Search Big Market" aria-label="Search" /><button class="btn">Search</button></form>
  <div class="state-cta"><a class="btn" href="#/">Back to home</a></div>${chips()}</div>`);
