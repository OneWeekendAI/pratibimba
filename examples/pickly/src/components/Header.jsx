import { useEffect, useState } from 'react';
import { SHOP } from '../data/shop';
import { MiniShelf } from './icons';
import CartJar from './CartJar';

/* The brand mark: a jar lid seen from above. Pleated kraft paper, a twine
   ring and a knot. No letter in a circle. */
export function BrandMark({ size = 40 }) {
  const pleats = [];
  const n = 22;
  for (let i = 0; i < n; i++) {
    const a = (i / n) * Math.PI * 2;
    const r = i % 2 ? 18.2 : 19.6;
    pleats.push(`${(20 + Math.cos(a) * r).toFixed(2)} ${(20 + Math.sin(a) * r).toFixed(2)}`);
  }
  return (
    <svg viewBox="0 0 40 40" width={size} height={size} className="brand-mark" aria-hidden="true" focusable="false">
      <polygon points={pleats.join(' ')} fill="var(--kraft)" stroke="#7a5a33" strokeWidth=".8" strokeLinejoin="round" />
      <circle cx="20" cy="20" r="13.2" fill="var(--kraft-light)" />
      <path d="M11.4 15.2c2.2-3.6 6.4-5.6 10.6-5" stroke="#fff6e4" strokeWidth="1.2" strokeLinecap="round" fill="none" opacity=".7" />
      <path d="M20.1 6.9c7.2.1 13 5.9 12.9 13.2-.1 7.2-6 12.9-13.2 12.8-7.1-.1-12.8-6-12.7-13.1.1-7.2 5.9-13 13-12.9z" fill="none" stroke="var(--twine-dark)" strokeWidth="1.6" />
      <path d="M27.6 29.6c-1.8-3.4-1-6 1.2-5.2 1.9.7.6 3.6-1.2 5.2zm0 0c3.2 1 5.7.3 5.2-1.6-.5-1.9-3.4-.9-5.2 1.6z" fill="none" stroke="var(--twine-dark)" strokeWidth="1.2" strokeLinejoin="round" />
      <path d="M27.6 29.6c-.4 1.9-1.4 3.6-2.8 5M27.6 29.6c1.6 1.1 2.6 2.7 3 4.6" stroke="var(--twine-dark)" strokeWidth="1.1" strokeLinecap="round" fill="none" />
      <circle cx="20" cy="20" r="3.2" fill="var(--wax-red)" />
      <circle cx="19.2" cy="19.2" r="1" fill="var(--wax-red-light)" />
    </svg>
  );
}

export function Brand() {
  return (
    <a href="#/" className="brand" aria-label={`${SHOP.name}, home`}>
      <BrandMark />
      <span className="brand-name">
        {SHOP.name}
        <small>{SHOP.tagline}</small>
      </span>
    </a>
  );
}

function useHash() {
  const [hash, setHash] = useState(() => window.location.hash || '#/');
  useEffect(() => {
    const on = () => setHash(window.location.hash || '#/');
    window.addEventListener('hashchange', on);
    return () => window.removeEventListener('hashchange', on);
  }, []);
  return hash;
}

export default function Header() {
  const hash = useHash();
  const onShop = hash.startsWith('#/shop') || hash.startsWith('#/product');
  const onAbout = hash.startsWith('#/about');

  return (
    <header className="header" id="top">
      <div className="wrap header-row">
        <Brand />
        <nav className="nav" aria-label="Main">
          <a href="#/shop" className="nav-link nav-shop" aria-current={onShop ? 'page' : undefined}>
            <MiniShelf size={22} />
            <span>Shop</span>
          </a>
          <a href="#/about" className="nav-link" aria-current={onAbout ? 'page' : undefined}>
            <span>About</span>
          </a>
        </nav>
        <CartJar />
      </div>
      <div className="header-shelf" aria-hidden="true" />
    </header>
  );
}
